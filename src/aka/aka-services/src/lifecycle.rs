//! Managed-service lifecycle: specs for the dns + proxy containers,
//! drift-aware `up`, `down`, status, ip and pull.

use aka_docker::AkaDocker;
use aka_docker::bollard::models::{ContainerCreateBody, HostConfig};
use aka_kernel::BoxedError;
use aka_kernel::config::AkaConfig;
use futures::StreamExt;
use sha2::{Digest, Sha256};
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

/// Create/start a service; recreate it when its spec label drifted.
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

    match docker.summary_opt(name, "").await? {
        Some(existing) => {
            let current = existing.labels.get(LABEL_SPEC).cloned().unwrap_or_default();

            if current != wanted {
                info!("recreating {name} (config changed)");
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
        None => {
            docker.create_start(name, body.clone()).await?;
            Ok(Ensured::Created)
        }
    }
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

    Ok(outcomes)
}

/// Stop (but keep) the managed containers.
pub async fn down(docker: &AkaDocker, cfg: &AkaConfig) -> Result<(), BoxedError> {
    for name in [&cfg.dns.container_name, &cfg.proxy.container_name] {
        docker.stop(name).await?;
    }
    Ok(())
}

pub async fn status(docker: &AkaDocker, cfg: &AkaConfig) -> Result<Vec<ServiceStatus>, BoxedError> {
    let mut statuses = Vec::new();

    for name in [&cfg.dns.container_name, &cfg.proxy.container_name] {
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

/// Pull both managed images, forwarding progress lines.
pub async fn pull(
    docker: &AkaDocker,
    cfg: &AkaConfig,
    mut on_line: impl FnMut(&str),
) -> Result<(), BoxedError> {
    for image in [&cfg.dns.image, &cfg.proxy.image] {
        if docker.image_exists(image).await {
            on_line(&format!("{image} already present"));
            continue;
        }
        let mut stream = docker.pull(image);
        while let Some(line) = stream.next().await {
            on_line(&line?);
        }
    }
    Ok(())
}
