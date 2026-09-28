//! The proxy daemon (`aka _proxyd`, hidden): watches docker, renders angie
//! configs, keeps the proxy's published tcp port set current (recreating it
//! on drift), reloads it on config changes and publishes
//! `~/.aka/state.json` for `aka status` / `aka routes`.

use std::path::Path;

use aka_docker::AkaDocker;
use aka_kernel::BoxedError;
use aka_kernel::config::AkaConfig;
use aka_kernel::container::ContainerSummary;
use aka_kernel::route::RouteTable;
use aka_kernel::state::{DaemonState, ProxyInfo};
use futures::{Stream, StreamExt};
use tokio::time::{Duration, interval, timeout};
use tracing::{error, info, warn};

use crate::angie::{HTTP_FILENAME, STREAM_FILENAME, render_http, render_stream};
use crate::certs::find_cert_pairs;
use crate::discover::{declares_routes, discover};
use crate::lifecycle;
use crate::paths::AkaPaths;
use crate::statefile;

pub struct Proxyd {
    docker: AkaDocker,
    cfg: AkaConfig,
    paths: AkaPaths,
    config_path: String,
    last_render: String,
    last_reload_ok: bool,
}

impl Proxyd {
    pub fn new(docker: AkaDocker, cfg: AkaConfig, paths: AkaPaths, config_path: String) -> Self {
        Self {
            docker,
            cfg,
            paths,
            config_path,
            last_render: String::new(),
            last_reload_ok: false,
        }
    }

    /// Event-driven loop with a slow resync timer (IP drift, dockerd restarts).
    pub async fn run(&mut self) -> Result<(), BoxedError> {
        self.paths.ensure_dirs()?;
        self.docker.ensure_network(&self.cfg.proxy.network).await?;

        let mut events = Box::pin(self.docker.events());
        let mut resync = interval(Duration::from_secs(20));
        resync.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            if let Err(err) = self.refresh().await {
                error!("refresh failed: {err}");
            }

            tokio::select! {
                _ = resync.tick() => {}
                event = events.next() => {
                    match event {
                        Some(message) => {
                            let action = message.action.unwrap_or_else(|| "event".into());
                            let actor = message
                                .actor
                                .as_ref()
                                .and_then(|a| a.attributes.as_ref())
                                .and_then(|a| a.get("name"))
                                .cloned()
                                .unwrap_or_default();
                            info!("docker event: {action} {actor}");
                            drain_burst(&mut events).await;
                        }
                        None => {
                            warn!("docker event stream ended; reconnecting in 5s");
                            tokio::time::sleep(Duration::from_secs(5)).await;
                            events = Box::pin(self.docker.events());
                        }
                    }
                }
            }
        }
    }

    pub async fn refresh(&mut self) -> Result<(), BoxedError> {
        let summaries = self.docker.list_summaries(&self.cfg.proxy.network).await?;

        let certs = find_cert_pairs(&self.cfg.proxy.ssl_certs_dir);
        let table = discover(&summaries, &self.cfg, &certs);

        for conflict in &table.conflicts {
            warn!("route conflict: {conflict}");
        }

        let render_changed = self.write_configs(&table)?;

        // Publish the current tcp route set; a change recreates the proxy,
        // which then boots with the freshly rendered configs.
        let mut tcp_ports: Vec<u16> = table
            .tcp
            .iter()
            .filter(|r| r.rejected.is_none())
            .map(|r| r.port)
            .collect();
        tcp_ports.sort_unstable();
        tcp_ports.dedup();
        let ensured =
            lifecycle::ensure_proxy(&self.docker, &self.cfg, &self.paths, &tcp_ports).await?;

        let proxy_running = self.docker.running(&self.cfg.proxy.container_name).await?;
        match ensured {
            lifecycle::Ensured::Created | lifecycle::Ensured::Recreated => {
                self.last_reload_ok = proxy_running;
            }
            _ if render_changed && proxy_running => {
                self.reload_proxy(&table).await;
            }
            _ if render_changed => {
                info!("config updated (proxy not running; applies on start)");
                self.last_reload_ok = false;
            }
            _ => {}
        }

        // Attach after the (possibly new) proxy container exists.
        self.attach_proxy_to_route_networks(&summaries).await?;

        let proxy_ip = self
            .docker
            .ip_of(&self.cfg.proxy.container_name, &self.cfg.proxy.network)
            .await?;

        let state = DaemonState {
            daemon_pid: std::process::id(),
            config_path: self.config_path.clone(),
            updated_at: statefile::now_label(),
            proxy: ProxyInfo {
                name: self.cfg.proxy.container_name.clone(),
                ip: proxy_ip,
                last_reload_ok: self.last_reload_ok,
            },
            table,
        };
        statefile::write(&self.paths.state_file(), &state)?;

        Ok(())
    }

    /// Make the proxy container reachable to every declared route container;
    /// returns true when at least one attachment was made.
    async fn attach_proxy_to_route_networks(
        &self,
        summaries: &[ContainerSummary],
    ) -> Result<bool, BoxedError> {
        if !self.docker.running(&self.cfg.proxy.container_name).await? {
            return Ok(false);
        }

        let mut attached_any = false;
        for summary in summaries {
            if summary.state != "running"
                || summary.name == self.cfg.proxy.container_name
                || !declares_routes(summary)
            {
                continue;
            }

            for network in summary.all_networks() {
                if network == self.cfg.proxy.network {
                    continue;
                }
                if self
                    .docker
                    .connect_container(network, &self.cfg.proxy.container_name)
                    .await
                    .is_ok()
                {
                    attached_any = true;
                }
            }
        }

        Ok(attached_any)
    }

    fn write_configs(&mut self, table: &RouteTable) -> Result<bool, BoxedError> {
        let http = render_http(table, &self.cfg.proxy);
        let stream = render_stream(table, &self.cfg.proxy);
        let fingerprint = format!("{http}\u{1}{stream}");

        if fingerprint == self.last_render {
            return Ok(false);
        }

        write_in_place(&self.paths.http_dir().join(HTTP_FILENAME), &http)?;
        write_in_place(&self.paths.stream_dir().join(STREAM_FILENAME), &stream)?;
        self.last_render = fingerprint;
        info!(
            "rendered {} http, {} tls, {} tcp routes",
            table.http.len(),
            table.tls.len(),
            table.tcp.len()
        );
        Ok(true)
    }

    async fn reload_proxy(&mut self, table: &RouteTable) {
        let container = self.cfg.proxy.container_name.clone();

        match self.docker.exec(&container, &["angie", "-t"]).await {
            Ok((true, _)) => {}
            Ok((false, output)) => {
                error!("angie -t failed, keeping previous config:\n{output}");
                self.last_reload_ok = false;
                return;
            }
            Err(err) => {
                error!("angie -t could not run: {err}");
                self.last_reload_ok = false;
                return;
            }
        }

        match self
            .docker
            .exec(&container, &["angie", "-s", "reload"])
            .await
        {
            Ok((true, _)) => {
                self.last_reload_ok = true;
                info!("reloaded {container} ({} hosts)", table.all_hosts().len());
            }
            Ok((false, output)) => error!("angie reload failed: {output}"),
            Err(err) => error!("angie reload could not run: {err}"),
        }
    }
}

/// Collapse event bursts: consume everything pending, stop after 100ms idle.
async fn drain_burst<S: Stream<Item = aka_docker::bollard::models::EventMessage> + Unpin>(
    events: &mut S,
) {
    while let Ok(Some(_)) = timeout(Duration::from_millis(100), events.next()).await {}
}

/// Overwrite the file contents without replacing the inode: the proxy
/// container bind-mounts these paths, and mounts follow inodes.
fn write_in_place(path: &Path, contents: &str) -> Result<(), BoxedError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    if std::fs::read_to_string(path).unwrap_or_default() == contents {
        return Ok(());
    }

    std::fs::write(path, contents)?;
    Ok(())
}
