//! Managed-service lifecycle: specs for the dns, proxy and admin (status
//! dashboard) containers, drift-aware `up`, `down`, status, ip and pull.

use aka_docker::AkaDocker;
use aka_docker::bollard::models::{ContainerCreateBody, HostConfig};
use aka_kernel::BoxedError;
use aka_kernel::config::AkaConfig;
use futures::StreamExt;
use sha2::{Digest, Sha256};
use std::path::Path;
use tracing::{debug, info};

use crate::dnsmasq::dns_args;
use crate::paths::AkaPaths;

pub const LABEL_MANAGED: &str = "aka.managed";
pub const LABEL_SERVICE: &str = "aka.service";
pub const LABEL_SPEC: &str = "aka.spec";

/// What `up` did for one service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ensured {
    AlreadyRunning,
    Started,
    Recreated,
    Created,
}

#[derive(Debug, Clone)]
pub struct ServiceStatus {
    pub name: String,
    pub exists: bool,
    pub running: bool,
    pub state: String,
    pub image: String,
    pub ip: Option<String>,
}

pub fn dns_spec(cfg: &AkaConfig) -> ContainerCreateBody {
    let spec = spec_hash(&format!(
        "dns|{}|{}|{}:{}|{}|{:?}",
        cfg.dns.image,
        cfg.dns.bind_ip,
        cfg.dns.port,
        cfg.dns.port,
        cfg.proxy.restart,
        cfg.dns.domains
    ));

    ContainerCreateBody {
        image: Some(cfg.dns.image.clone()),
        labels: Some(labels("dns", &spec)),
        env: Some(vec![format!("DNSMASQ_ARGS={}", dns_args(&cfg.dns))]),
        exposed_ports: Some(vec![
            format!("{}/tcp", cfg.dns.port),
            format!("{}/udp", cfg.dns.port),
        ]),
        host_config: Some(HostConfig {
            port_bindings: Some(bindings(&[
                (cfg.dns.port, cfg.dns.port, "tcp", &cfg.dns.bind_ip),
                (cfg.dns.port, cfg.dns.port, "udp", &cfg.dns.bind_ip),
            ])),
            cap_add: Some(vec!["NET_ADMIN".into()]),
            restart_policy: Some(restart_policy(&cfg.proxy.restart)),
            ..Default::default()
        }),
        ..Default::default()
    }
}

pub fn proxy_spec(cfg: &AkaConfig, paths: &AkaPaths, tcp_ports: &[u16]) -> ContainerCreateBody {
    let mut binds = vec![
        format!("{}:/etc/angie/http.d", paths.http_dir().display()),
        format!("{}:/etc/angie/stream.d", paths.stream_dir().display()),
    ];
    if !cfg.proxy.ssl_certs_dir.is_empty() {
        binds.push(format!("{}:/etc/angie/certs:ro", cfg.proxy.ssl_certs_dir));
    }

    let mut exposed = vec![format!("{}/tcp", cfg.proxy.http_port)];
    let mut ports = vec![(
        cfg.proxy.http_port,
        cfg.proxy.http_port,
        "tcp",
        cfg.proxy.bind_ip.as_str(),
    )];
    if cfg.proxy.tls_enabled {
        exposed.push(format!("{}/tcp", cfg.proxy.tls_port));
        ports.push((
            cfg.proxy.tls_port,
            cfg.proxy.tls_port,
            "tcp",
            cfg.proxy.bind_ip.as_str(),
        ));
    }
    if !cfg.proxy.ssl_certs_dir.is_empty() {
        // TLS-terminated vhosts live on the fixed termination port.
        exposed.push(format!("{}/tcp", crate::angie::TERMINATION_PORT));
        ports.push((
            crate::angie::TERMINATION_PORT,
            crate::angie::TERMINATION_PORT,
            "tcp",
            cfg.proxy.bind_ip.as_str(),
        ));
    }
    // Raw tcp routes are served by angie's stream listeners; docker can only
    // publish ports fixed at create time, so the published set is part of the
    // spec and proxyd recreates the proxy when it changes.
    let tcp_ports = {
        let mut v = tcp_ports.to_vec();
        v.sort_unstable();
        v.dedup();
        v
    };
    for port in &tcp_ports {
        if *port == cfg.proxy.http_port || (cfg.proxy.tls_enabled && *port == cfg.proxy.tls_port) {
            continue;
        }
        exposed.push(format!("{port}/tcp"));
        ports.push((*port, *port, "tcp", cfg.proxy.bind_ip.as_str()));
    }

    let spec = spec_hash(&format!(
        "proxy|{}|{}|{:?}|{:?}|{:?}",
        cfg.proxy.image, cfg.proxy.network, ports, binds, tcp_ports
    ));

    ContainerCreateBody {
        image: Some(cfg.proxy.image.clone()),
        labels: Some(labels("proxy", &spec)),
        exposed_ports: Some(exposed),
        host_config: Some(HostConfig {
            port_bindings: Some(bindings(&ports)),
            binds: Some(binds),
            network_mode: Some(cfg.proxy.network.clone()),
            restart_policy: Some(restart_policy(&cfg.proxy.restart)),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// The dashboard listens on 80 (the image's `PORT` default); aka publishes
/// it on `admin.host_port` for the direct (`aka down`-proof) view.
pub const ADMIN_CONTAINER_PORT: u16 = 80;
/// Where the host's aka home, resolved config and docker socket appear
/// inside the admin container (paths the image already expects).
const ADMIN_CONTAINER_HOME: &str = "/aka/home";
const ADMIN_CONTAINER_CONFIG: &str = "/aka/config.toml";
const ADMIN_CONTAINER_SOCKET: &str = "/var/run/docker.sock";

/// The `aka status` dashboard, run with the same drift-aware lifecycle as
/// dns + proxy. It declares its own proxy routes through `VIRTUAL_HOST`
/// (`aka.<domain>` for every aka domain), so the proxy picks the status page
/// up exactly like any other route container — no special-casing in the
/// renderer or discovery.
///
/// `config_file` is the config the CLI resolved: inside the container aka's
/// own discovery finds nothing, so mounting it (read-only, `AKA_CONFIG`)
/// keeps the dashboard reading the same names/tags/state the host uses.
pub fn admin_spec(
    cfg: &AkaConfig,
    paths: &AkaPaths,
    config_file: Option<&Path>,
) -> ContainerCreateBody {
    let hosts = cfg.admin_hosts();

    let mut env = vec![
        format!("PORT={ADMIN_CONTAINER_PORT}"),
        format!("AKA_HOME={ADMIN_CONTAINER_HOME}"),
        "RUST_LOG=info".into(),
        "ENVIRONMENT=local".into(),
    ];
    let mut binds = vec![
        format!("{}:{ADMIN_CONTAINER_HOME}:ro", paths.home.display()),
        format!("{}:{ADMIN_CONTAINER_SOCKET}", cfg.admin.docker_socket),
    ];

    if !hosts.is_empty() {
        env.push(format!("VIRTUAL_HOST={}", hosts.join(",")));
        env.push(format!("VIRTUAL_PORT={ADMIN_CONTAINER_PORT}"));
    }
    if let Some(file) = config_file.filter(|p| p.is_file()) {
        env.push(format!("AKA_CONFIG={ADMIN_CONTAINER_CONFIG}"));
        binds.push(format!("{}:{ADMIN_CONTAINER_CONFIG}:ro", file.display()));
    }

    let spec = spec_hash(&format!(
        "admin|{}|{:?}|{}:{}:{}|{}|{}|{:?}",
        cfg.admin.image,
        hosts,
        cfg.admin.bind_ip,
        cfg.admin.host_port,
        ADMIN_CONTAINER_PORT,
        cfg.proxy.network,
        cfg.proxy.restart,
        binds
    ));

    ContainerCreateBody {
        image: Some(cfg.admin.image.clone()),
        labels: Some(labels("admin", &spec)),
        env: Some(env),
        exposed_ports: Some(vec![format!("{ADMIN_CONTAINER_PORT}/tcp")]),
        host_config: Some(HostConfig {
            port_bindings: Some(bindings(&[(
                cfg.admin.host_port,
                ADMIN_CONTAINER_PORT,
                "tcp",
                &cfg.admin.bind_ip,
            )])),
            binds: Some(binds),
            // standalone on the proxy's network: reachable by IP for the
            // rendered vhost, published on `host_port` for direct access
            network_mode: Some(cfg.proxy.network.clone()),
            restart_policy: Some(restart_policy(&cfg.proxy.restart)),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn labels(service: &str, spec: &str) -> std::collections::HashMap<String, String> {
    [
        (LABEL_MANAGED.into(), "true".into()),
        (LABEL_SERVICE.into(), service.into()),
        (LABEL_SPEC.into(), spec.into()),
    ]
    .into_iter()
    .collect()
}

fn spec_hash(input: &str) -> String {
    Sha256::digest(input.as_bytes())
        .iter()
        .take(8)
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn restart_policy(kind: &str) -> aka_docker::bollard::models::RestartPolicy {
    let name = match kind {
        "always" => aka_docker::bollard::models::RestartPolicyNameEnum::ALWAYS,
        "on-failure" => aka_docker::bollard::models::RestartPolicyNameEnum::ON_FAILURE,
        "no" => aka_docker::bollard::models::RestartPolicyNameEnum::NO,
        _ => aka_docker::bollard::models::RestartPolicyNameEnum::UNLESS_STOPPED,
    };
    aka_docker::bollard::models::RestartPolicy {
        name: Some(name),
        maximum_retry_count: None,
    }
}

fn bindings(ports: &[(u16, u16, &str, &str)]) -> aka_docker::bollard::models::PortMap {
    ports
        .iter()
        .map(|(host, container, proto, ip)| {
            (
                format!("{container}/{proto}"),
                Some(vec![aka_docker::bollard::models::PortBinding {
                    host_ip: Some(ip.to_string()),
                    host_port: Some(host.to_string()),
                }]),
            )
        })
        .collect()
}

/// Why a container running the wanted spec is still stale: `aka pull` moves a
/// tag (`name:latest`, or the retag that pins it to the CLI's version) onto a
/// new build, and no part of the spec hash can see that — the spec records the
/// image *reference*. Comparing image ids is what makes `aka up` apply a pull.
///
/// An unknown side is never drift: the wanted reference may not be pulled yet,
/// or the engine may no longer have the image record the container runs from.
/// Both cases are `aka pull`'s job, not a reason to recreate.
fn image_drift(running: Option<&str>, wanted: Option<&str>) -> Option<&'static str> {
    match (running, wanted) {
        (Some(running), Some(wanted)) if running != wanted => Some("image updated, pulling it in"),
        _ => None,
    }
}

/// Create/start a service; recreate it when its spec label drifted, or when
/// the image its spec names now points at different content than the running
/// container was created from (`aka pull` then `aka up`).
pub async fn ensure_service(
    docker: &AkaDocker,
    name: &str,
    body: &ContainerCreateBody,
) -> Result<Ensured, BoxedError> {
    let wanted = body
        .labels
        .as_ref()
        .and_then(|l| l.get(LABEL_SPEC))
        .cloned()
        .unwrap_or_default();

    let Some(existing) = docker.summary_opt(name, "").await? else {
        docker.create_start(name, body.clone()).await?;
        return Ok(Ensured::Created);
    };

    // The wanted image id costs one more inspect, so it is only resolved once
    // the spec itself agrees; spec drift already explains the recreate.
    let drift = if existing
        .labels
        .get(LABEL_SPEC)
        .map(String::as_str)
        .unwrap_or_default()
        != wanted
    {
        Some("config changed")
    } else {
        image_drift(
            existing.image_id.as_deref(),
            match body.image.as_deref() {
                Some(image) => docker.image_id(image).await,
                None => None,
            }
            .as_deref(),
        )
    };

    if let Some(drift) = drift {
        info!("recreating {name} ({drift})");
        docker.stop_remove(name).await?;
        docker.create_start(name, body.clone()).await?;
        return Ok(Ensured::Recreated);
    }

    if existing.state == "running" {
        return Ok(Ensured::AlreadyRunning);
    }

    docker.start(name).await?;
    Ok(Ensured::Started)
}

/// Bring the enabled services up. Returns (service, outcome) per enabled
/// service.
/// TCP route ports (non-rejected) the proxy must publish for the current
/// container set.
pub async fn desired_tcp_ports(
    docker: &AkaDocker,
    cfg: &AkaConfig,
) -> Result<Vec<u16>, BoxedError> {
    let summaries = docker.list_summaries(&cfg.proxy.network).await?;
    let certs = crate::certs::find_cert_pairs(&cfg.proxy.ssl_certs_dir);
    let table = crate::discover::discover(&summaries, cfg, &certs);
    let mut ports: Vec<u16> = table
        .tcp
        .iter()
        .filter(|r| r.rejected.is_none())
        .map(|r| r.port)
        .collect();
    ports.sort_unstable();
    ports.dedup();
    Ok(ports)
}

/// Ensure the proxy publishes exactly `tcp_ports`; recreates on drift.
pub async fn ensure_proxy(
    docker: &AkaDocker,
    cfg: &AkaConfig,
    paths: &AkaPaths,
    tcp_ports: &[u16],
) -> Result<Ensured, BoxedError> {
    ensure_service(
        docker,
        &cfg.proxy.container_name,
        &proxy_spec(cfg, paths, tcp_ports),
    )
    .await
}

pub async fn up(
    docker: &AkaDocker,
    cfg: &AkaConfig,
    paths: &AkaPaths,
    config_file: Option<&Path>,
) -> Result<Vec<(String, Ensured)>, BoxedError> {
    paths.ensure_dirs()?;
    docker.ensure_network(&cfg.proxy.network).await?;

    let mut outcomes = Vec::new();

    if cfg.dns.enabled {
        let outcome = ensure_service(docker, &cfg.dns.container_name, &dns_spec(cfg)).await?;
        debug!("dns: {outcome:?}");
        outcomes.push((cfg.dns.container_name.clone(), outcome));
    }

    if cfg.proxy.enabled {
        let tcp_ports = desired_tcp_ports(docker, cfg).await?;
        let outcome = ensure_proxy(docker, cfg, paths, &tcp_ports).await?;
        debug!("proxy: {outcome:?}");
        outcomes.push((cfg.proxy.container_name.clone(), outcome));
    }

    if cfg.admin.enabled {
        let outcome = ensure_service(
            docker,
            &cfg.admin.container_name,
            &admin_spec(cfg, paths, config_file),
        )
        .await?;
        debug!("admin: {outcome:?}");
        outcomes.push((cfg.admin.container_name.clone(), outcome));
    }

    Ok(outcomes)
}

/// Stop and remove the managed containers, docker-compose `down` style: what
/// survives is everything mounted in from the host (rendered configs, state)
/// plus the `aka` network, which route containers from other projects are
/// attached to. The next `aka up` creates fresh containers, so it always runs
/// the build the configured image tags name right then.
pub async fn down(docker: &AkaDocker, cfg: &AkaConfig) -> Result<(), BoxedError> {
    for name in [
        &cfg.dns.container_name,
        &cfg.proxy.container_name,
        &cfg.admin.container_name,
    ] {
        docker.stop_remove(name).await?;
    }
    Ok(())
}

pub async fn status(docker: &AkaDocker, cfg: &AkaConfig) -> Result<Vec<ServiceStatus>, BoxedError> {
    let mut statuses = Vec::new();

    for name in [
        &cfg.dns.container_name,
        &cfg.proxy.container_name,
        &cfg.admin.container_name,
    ] {
        match docker.summary_opt(name, &cfg.proxy.network).await? {
            Some(s) => {
                let ip = s.backend_ip().map(str::to_owned);
                statuses.push(ServiceStatus {
                    name: name.clone(),
                    exists: true,
                    running: s.state == "running",
                    state: s.state,
                    image: s.image,
                    ip,
                });
            }
            None => statuses.push(ServiceStatus {
                name: name.clone(),
                exists: false,
                running: false,
                state: "absent".into(),
                image: String::new(),
                ip: None,
            }),
        }
    }

    Ok(statuses)
}

/// Summary of a named container (dory's `dory ip` target).
pub async fn ip_of(
    docker: &AkaDocker,
    cfg: &AkaConfig,
    name: &str,
) -> Result<Option<String>, BoxedError> {
    docker.ip_of(name, &cfg.proxy.network).await
}

/// Pull every managed image, forwarding progress lines.
///
/// Each image is first tried at the tag matching the running CLI's version
/// (every published build carries `:latest` plus its crate version), and the
/// configured reference is then retagged onto that build, so `aka up` runs
/// the images that match the CLI. When the hub has no exact version match
/// (unreleased ref, custom repo), the configured tag is pulled as before.
///
/// Pulling only changes what the tags point at; the next `aka up` is what
/// applies it, because `ensure_service` recreates a container created from
/// different content than its tag now names.
pub async fn pull(
    docker: &AkaDocker,
    cfg: &AkaConfig,
    version: &str,
    mut on_line: impl FnMut(&str),
) -> Result<(), BoxedError> {
    let mut moved = false;

    for image in [&cfg.dns.image, &cfg.proxy.image, &cfg.admin.image] {
        let (name, tag) = aka_docker::split_image(image);
        let pinned = format!("{name}:{version}");

        if tag != version {
            // A missing version build is not fatal: the configured tag is the fallback.
            let mut stream = docker.pull(&pinned);
            while let Some(line) = stream.next().await {
                on_line(&line.unwrap_or_else(|err| format!("error: {err}")));
            }
            if let Some(pinned_id) = docker.image_id(&pinned).await {
                // Retag only when the reference does not already name that
                // build, so re-pulling an unchanged image says so instead of
                // sending the user off to `aka up` for nothing.
                if docker.image_id(image).await.as_ref() != Some(&pinned_id) {
                    docker.tag(&pinned, image).await?;
                    moved = true;
                    on_line(&format!("{image} pinned to {version}"));
                } else {
                    on_line(&format!("{image} already at {version}"));
                }
                continue;
            }
            on_line(&format!("{pinned} unavailable; falling back to {image}"));
        }

        // Nothing to pin and nothing to fetch: the local build stays as it is
        // (offline-friendly), so the line says the hub was not consulted.
        if docker.image_exists(image).await {
            on_line(&format!("{image} already present (left as is)"));
            continue;
        }
        let mut stream = docker.pull(image);
        while let Some(line) = stream.next().await {
            on_line(&line?);
        }
        moved = true;
    }

    if moved {
        on_line("aka up to run them: a service on an older build is recreated");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_moved_tag_is_drift_and_an_unknown_is_not() {
        // The spec (and so the container's `image` reference) is identical in
        // every case here: only the content the tag points at moved.
        assert_eq!(
            image_drift(Some("sha256:old"), Some("sha256:new")),
            Some("image updated, pulling it in")
        );
        assert_eq!(image_drift(Some("sha256:same"), Some("sha256:same")), None);

        // Nothing to compare against, either side: leave the container alone.
        assert_eq!(image_drift(None, Some("sha256:new")), None);
        assert_eq!(image_drift(Some("sha256:old"), None), None);
        assert_eq!(image_drift(None, None), None);
    }

    fn paths() -> AkaPaths {
        AkaPaths {
            home: std::path::PathBuf::from("/tmp/aka-test-home"),
        }
    }

    #[test]
    fn admin_spec_declares_its_own_proxy_routes() {
        let cfg = AkaConfig::default();
        let body = admin_spec(&cfg, &paths(), None);
        let env = body.env.unwrap();

        assert!(
            env.contains(&"VIRTUAL_HOST=aka.docker".to_string()),
            "the status page routes through normal discovery: {env:?}"
        );
        assert!(env.contains(&"VIRTUAL_PORT=80".to_string()));
        assert!(env.contains(&"PORT=80".to_string()));
        assert!(env.contains(&"AKA_HOME=/aka/home".to_string()));
        assert!(
            !env.iter().any(|e| e.starts_with("AKA_CONFIG=")),
            "a config path that is not a file must not be mounted"
        );

        let host = body.host_config.unwrap();
        let binds = host.binds.unwrap();
        assert!(binds.contains(&"/tmp/aka-test-home:/aka/home:ro".to_string()));
        assert!(binds.contains(&"/var/run/docker.sock:/var/run/docker.sock".to_string()));

        let bindings = host.port_bindings.unwrap();
        let pb = &bindings["80/tcp"].as_ref().unwrap()[0];
        assert_eq!(pb.host_port.as_deref(), Some("3001"));
        assert_eq!(pb.host_ip.as_deref(), Some("127.0.0.1"));
        assert_eq!(host.network_mode.as_deref(), Some("aka"));
    }

    #[test]
    fn admin_spec_mounts_the_resolved_config_when_it_exists() {
        let dir = std::env::temp_dir().join(format!("aka-admin-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("aka.toml");
        std::fs::write(&file, "[proxy]\nhttp_port = 8080\n").unwrap();

        let cfg = AkaConfig::default();
        let body = admin_spec(&cfg, &paths(), Some(&file));
        assert!(
            body.env
                .unwrap()
                .contains(&"AKA_CONFIG=/aka/config.toml".to_string())
        );

        // a stale path (deleted, or defaults with no file on disk) stays unmounted
        let body = admin_spec(&cfg, &paths(), Some(&dir.join("gone.toml")));
        assert!(
            !body
                .env
                .unwrap()
                .iter()
                .any(|e| e.starts_with("AKA_CONFIG="))
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
