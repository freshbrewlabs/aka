//! Route discovery: turn container declarations (dory-compatible env vars,
//! aka labels) into a `RouteTable`, merging load-balanced groups and
//! flagging conflicts.

use std::collections::{BTreeMap, BTreeSet};

use aka_kernel::config::{AkaConfig, ProxyConfig};
use aka_kernel::container::ContainerSummary;
use aka_kernel::route::{Route, RouteProto, RouteTable, TcpRoute, TerminatedRoute, Upstream};

use crate::certs::CertPair;

/// env var / docker label names (labels win).
pub const ENV_HOST: &str = "VIRTUAL_HOST";
pub const ENV_PORT: &str = "VIRTUAL_PORT";
pub const ENV_PROTO: &str = "VIRTUAL_PROTO";
pub const LABEL_HOST: &str = "aka.host";
pub const LABEL_PORT: &str = "aka.port";
pub const LABEL_PROTO: &str = "aka.proto";

pub const DEFAULT_HTTP_PORT: u16 = 80;

fn is_managed(name: &str, cfg: &AkaConfig) -> bool {
    name == cfg.proxy.container_name || name == cfg.dns.container_name
}

pub fn declares_routes(summary: &ContainerSummary) -> bool {
    raw_field(summary, LABEL_HOST, ENV_HOST).is_some()
}

fn raw_field<'a>(summary: &'a ContainerSummary, label: &str, env: &str) -> Option<&'a str> {
    summary
        .labels
        .get(label)
        .map(String::as_str)
        .or_else(|| summary.env.get(env).map(String::as_str))
        .map(str::trim)
        .filter(|v| !v.is_empty())
}

/// Hosts in `VIRTUAL_HOST`/`aka.host` may be separated by commas and/or
/// whitespace (compose `VIRTUAL_HOST: a.docker b.docker` is one plain string).
fn split_hosts(raw: &str) -> Vec<String> {
    raw.split(|c: char| c == ',' || c.is_whitespace())
        .map(str::to_ascii_lowercase)
        .filter(|h| !h.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

struct Declaration {
    container: String,
    hosts: Vec<String>,
    proto: RouteProto,
    port: u16,
    server: String,
}

pub fn discover(
    summaries: &[ContainerSummary],
    cfg: &AkaConfig,
    certs: &BTreeMap<String, CertPair>,
) -> RouteTable {
    let mut table = RouteTable::default();
    let mut declarations = Vec::new();

    for summary in summaries {
        if summary.state != "running" || is_managed(&summary.name, cfg) || !declares_routes(summary)
        {
            continue;
        }

        if let Some(decl) = declare(summary, &mut table.conflicts) {
            declarations.push(decl);
        }
    }

    declarations.sort_by(|a, b| a.container.cmp(&b.container));

    let mut http = group(&declarations, |p| {
        matches!(p, RouteProto::Http | RouteProto::Https)
    });
    let mut tls = group(&declarations, |p| p == RouteProto::Tls);
    resolve_host_clashes(&mut http, &mut table.conflicts);
    resolve_host_clashes(&mut tls, &mut table.conflicts);
    table.http = http.into_values().collect();
    table.tls = tls.into_values().collect();

    if cfg.proxy.tls_enabled && !cfg.proxy.ssl_certs_dir.is_empty() {
        table.terminated = terminated_routes(&table.http, certs);
    }

    table.tcp = group_tcp(&declarations, &cfg.proxy, &mut table.conflicts);
    table
}

/// Parse one container's declaration; push problem notes into `conflicts`.
fn declare(summary: &ContainerSummary, conflicts: &mut Vec<String>) -> Option<Declaration> {
    let hosts = split_hosts(raw_field(summary, LABEL_HOST, ENV_HOST)?);

    if hosts.is_empty() {
        conflicts.push(format!(
            "container {}: no usable hosts in {LABEL_HOST}/{ENV_HOST}",
            summary.name
        ));
        return None;
    }

    let proto = match raw_field(summary, LABEL_PROTO, ENV_PROTO) {
        Some(raw) => match RouteProto::parse(raw) {
            Some(proto) => proto,
            None => {
                conflicts.push(format!(
                    "container {}: unknown proto {raw:?}, using http",
                    summary.name
                ));
                RouteProto::Http
            }
        },
        None => RouteProto::Http,
    };

    let declared_port = match raw_field(summary, LABEL_PORT, ENV_PORT) {
        Some(raw) if raw != "auto" => match raw.parse::<u16>() {
            Ok(port) => Some(port),
            Err(_) => {
                conflicts.push(format!(
                    "container {}: invalid port {raw:?}, falling back",
                    summary.name
                ));
                None
            }
        },
        _ => None,
    };

    let port = declared_port.or_else(|| match proto {
        RouteProto::Http | RouteProto::Https => Some(
            summary
                .ports
                .iter()
                .next()
                .copied()
                .unwrap_or(DEFAULT_HTTP_PORT),
        ),
        // raw routes must be explicit; 80/443 exposed by the app itself are
        // never the intended tcp target
        RouteProto::Tcp | RouteProto::Tls => summary
            .ports
            .iter()
            .find(|p| !matches!(**p, 80 | 443))
            .copied(),
    });

    let Some(port) = port else {
        conflicts.push(format!(
            "container {}: {} route needs a port ({ENV_PORT} or an exposed port)",
            summary.name,
            proto.as_str()
        ));
        return None;
    };

    let Some(ip) = summary.backend_ip() else {
        conflicts.push(format!(
            "container {}: no network address to proxy to",
            summary.name
        ));
        return None;
    };

    Some(Declaration {
        container: summary.name.clone(),
        hosts,
        proto,
        port,
        server: format!("{ip}:{port}"),
    })
}

/// Merge declarations sharing (proto, port) into load-balanced groups;
/// backends, hosts and containers union.
fn group(
    declarations: &[Declaration],
    take: impl Fn(RouteProto) -> bool,
) -> BTreeMap<(String, RouteProto, u16), Route> {
    // One vhost per host, like nginx-proxy: every host gets its own route and
    // only containers declaring *that host* share its upstream. Replicas of
    // the same host load-balance together; unrelated hosts never mix backends.
    let mut groups: BTreeMap<(String, RouteProto, u16), Route> = BTreeMap::new();

    for decl in declarations.iter().filter(|d| take(d.proto)) {
        for host in &decl.hosts {
            let route = groups
                .entry((host.clone(), decl.proto, decl.port))
                .or_insert_with(|| Route {
                    hosts: vec![host.clone()],
                    proto: decl.proto,
                    port: decl.port,
                    containers: Vec::new(),
                    upstream: Upstream {
                        servers: Vec::new(),
                    },
                    rejected: None,
                });

            if !route.upstream.servers.contains(&decl.server) {
                route.upstream.servers.push(decl.server.clone());
                route.upstream.servers.sort();
            }
            if !route.containers.contains(&decl.container) {
                route.containers.push(decl.container.clone());
            }
        }
    }

    groups
}

fn resolve_host_clashes(
    routes: &mut BTreeMap<(String, RouteProto, u16), Route>,
    conflicts: &mut Vec<String>,
) {
    // The same host declared under different proto/port groups would create
    // ambiguous server blocks; the first group (by sorted key) wins.
    let mut owners: BTreeMap<&str, String> = BTreeMap::new();

    for ((host, _proto, _port), route) in routes.iter_mut() {
        match owners.get(host.as_str()) {
            Some(owner) => {
                route.rejected = Some(format!("hosts claimed by {owner}"));
                conflicts.push(format!(
                    "host {host} already served by {owner}; {} ignored",
                    route.containers.join(",")
                ));
            }
            None => {
                owners.insert(host.as_str(), route.containers.join(","));
            }
        }
    }
}

/// Raw tcp routes share a stream listener per port; a second, different
/// backend set on the same port is a conflict.
fn group_tcp(
    declarations: &[Declaration],
    proxy: &ProxyConfig,
    conflicts: &mut Vec<String>,
) -> Vec<TcpRoute> {
    let reserved = proxy.reserved_stream_ports();
    let mut groups: BTreeMap<u16, TcpRoute> = BTreeMap::new();

    for decl in declarations.iter().filter(|d| d.proto == RouteProto::Tcp) {
        match groups.get_mut(&decl.port) {
            None => {
                let rejected = reserved
                    .contains(&decl.port)
                    .then(|| "port reserved by the http/tls listeners".to_owned());
                if rejected.is_some() {
                    conflicts.push(format!(
                        "container {}: tcp route on port {} rejected (reserved)",
                        decl.container, decl.port
                    ));
                }
                groups.insert(
                    decl.port,
                    TcpRoute {
                        hosts: decl.hosts.clone(),
                        port: decl.port,
                        containers: vec![decl.container.clone()],
                        upstream: Upstream {
                            servers: vec![decl.server.clone()],
                        },
                        rejected,
                    },
                );
            }
            Some(existing) => {
                if !existing.upstream.servers.contains(&decl.server) && existing.rejected.is_none()
                {
                    conflicts.push(format!(
                        "container {}: tcp port {} already routed to {}; ignored",
                        decl.container,
                        decl.port,
                        existing.containers.join(",")
                    ));
                    continue;
                }

                existing.hosts.extend(decl.hosts.iter().cloned());
                existing.hosts.sort();
                existing.hosts.dedup();
                if !existing.containers.contains(&decl.container) {
                    existing.containers.push(decl.container.clone());
                }
                if !existing.upstream.servers.contains(&decl.server) {
                    existing.upstream.servers.push(decl.server.clone());
                    existing.upstream.servers.sort();
                }
            }
        }
    }

    groups.into_values().collect()
}

/// http/https routes whose hosts own a cert pair get TLS termination.
fn terminated_routes(http: &[Route], certs: &BTreeMap<String, CertPair>) -> Vec<TerminatedRoute> {
    let mut terminated = Vec::new();

    for route in http.iter().filter(|r| r.rejected.is_none()) {
        for host in &route.hosts {
            if let Some(pair) = certs.get(host) {
                let name = |p: &std::path::Path| {
                    p.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default()
                };
                terminated.push(TerminatedRoute {
                    host: host.clone(),
                    cert: name(&pair.cert),
                    key: name(&pair.key),
                    container: route.containers.join(","),
                    proto: route.proto,
                    upstream: route.upstream.clone(),
                });
            }
        }
    }

    terminated
}

/// Hosts declared by running containers (for status output).
pub fn declared_hosts(summaries: &[ContainerSummary], cfg: &AkaConfig) -> Vec<String> {
    summaries
        .iter()
        .filter(|s| s.state == "running" && !is_managed(&s.name, cfg) && declares_routes(s))
        .flat_map(|s| split_hosts(raw_field(s, LABEL_HOST, ENV_HOST).unwrap_or_default()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use aka_kernel::container::NetworkEndpoint;

    fn container(
        name: &str,
        ip: &str,
        state: &str,
        env: &[(&str, &str)],
        ports: &[u16],
    ) -> ContainerSummary {
        ContainerSummary {
            id: name.into(),
            name: name.into(),
            state: state.into(),
            image: "img".into(),
            env: env
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            labels: Default::default(),
            ports: ports.iter().copied().collect(),
            networks: vec![NetworkEndpoint {
                network: "aka".into(),
                ip: ip.into(),
            }],
        }
    }

    fn run(containers: Vec<ContainerSummary>) -> RouteTable {
        discover(&containers, &AkaConfig::default(), &Default::default())
    }

    #[test]
    fn http_route_from_env() {
        let table = run(vec![container(
            "web",
            "172.20.0.5",
            "running",
            &[("VIRTUAL_HOST", "myapp.docker"), ("VIRTUAL_PORT", "3000")],
            &[],
        )]);

        assert_eq!(table.http.len(), 1);
        assert_eq!(table.http[0].hosts, vec!["myapp.docker"]);
        assert_eq!(table.http[0].proto, RouteProto::Http);
        assert_eq!(table.http[0].port, 3000);
        assert_eq!(table.http[0].upstream.servers, vec!["172.20.0.5:3000"]);
        assert!(table.tcp.is_empty());
        assert!(table.conflicts.is_empty());
    }

    #[test]
    fn default_port_is_first_exposed_then_80() {
        let table = run(vec![
            container(
                "web",
                "172.20.0.5",
                "running",
                &[("VIRTUAL_HOST", "a.docker")],
                &[3001, 8080],
            ),
            container(
                "api",
                "172.20.0.6",
                "running",
                &[("VIRTUAL_HOST", "b.docker")],
                &[],
            ),
        ]);

        let a = table
            .http
            .iter()
            .find(|r| r.hosts[0] == "a.docker")
            .unwrap();
        let b = table
            .http
            .iter()
            .find(|r| r.hosts[0] == "b.docker")
            .unwrap();
        assert_eq!(a.port, 3001);
        assert_eq!(b.port, 80);
    }

    #[test]
    fn labels_win_over_env_hosts_split_lowercase() {
        let mut c = container(
            "web",
            "172.20.0.5",
            "running",
            &[
                ("VIRTUAL_HOST", "ignored.docker"),
                ("VIRTUAL_PROTO", "https"),
            ],
            &[80],
        );
        c.labels
            .insert(LABEL_HOST.into(), "Label.Docker, second.docker".into());
        let table = run(vec![c]);

        assert_eq!(table.http.len(), 2);
        for route in &table.http {
            assert_eq!(route.hosts.len(), 1, "one vhost per route");
            assert_eq!(route.proto, RouteProto::Https);
            assert_eq!(route.upstream.servers, vec!["172.20.0.5:80"]);
        }
        let hosts: Vec<&str> = table.http.iter().map(|r| r.hosts[0].as_str()).collect();
        assert_eq!(hosts, vec!["label.docker", "second.docker"]);
        assert!(table.conflicts.is_empty());
    }

    #[test]
    fn comma_and_space_hosts_both_split() {
        let containers = vec![container(
            "web",
            "172.20.0.5",
            "running",
            &[
                ("VIRTUAL_HOST", "a.docker  b.docker,\tc.docker"),
                ("VIRTUAL_PORT", "3000"),
            ],
            &[],
        )];
        let table = discover(&containers, &AkaConfig::default(), &Default::default());

        let hosts: Vec<&str> = table.http.iter().map(|r| r.hosts[0].as_str()).collect();
        assert_eq!(hosts, vec!["a.docker", "b.docker", "c.docker"]);
        assert!(table.conflicts.is_empty());
        assert_eq!(
            declared_hosts(&containers, &AkaConfig::default()),
            vec!["a.docker", "b.docker", "c.docker"]
        );
    }

    #[test]
    fn unrelated_hosts_never_share_upstreams() {
        // Live bug: all http:80 routes once merged into one upstream, so a
        // request for one host could round-robin onto another's backend.
        let table = run(vec![
            container(
                "demo",
                "172.21.0.4",
                "running",
                &[("VIRTUAL_HOST", "demo.docker"), ("VIRTUAL_PORT", "80")],
                &[],
            ),
            container(
                "app",
                "172.20.0.6",
                "running",
                &[("VIRTUAL_HOST", "app.docker"), ("VIRTUAL_PORT", "80")],
                &[],
            ),
        ]);

        assert_eq!(table.http.len(), 2);
        let demo = table
            .http
            .iter()
            .find(|r| r.hosts[0] == "demo.docker")
            .unwrap();
        let app = table
            .http
            .iter()
            .find(|r| r.hosts[0] == "app.docker")
            .unwrap();
        assert_eq!(demo.upstream.servers, vec!["172.21.0.4:80"]);
        assert_eq!(app.upstream.servers, vec!["172.20.0.6:80"]);
        assert!(table.conflicts.is_empty());
    }

    #[test]
    fn same_host_same_port_load_balances() {
        let table = run(vec![
            container(
                "one",
                "172.20.0.2",
                "running",
                &[("VIRTUAL_HOST", "x.docker"), ("VIRTUAL_PORT", "3000")],
                &[],
            ),
            container(
                "two",
                "172.20.0.3",
                "running",
                &[("VIRTUAL_HOST", "x.docker"), ("VIRTUAL_PORT", "3000")],
                &[],
            ),
        ]);

        assert_eq!(table.http.len(), 1);
        assert_eq!(
            table.http[0].upstream.servers,
            vec!["172.20.0.2:3000", "172.20.0.3:3000"]
        );
        assert_eq!(table.http[0].containers, vec!["one", "two"]);
        assert!(table.conflicts.is_empty());
    }

    #[test]
    fn same_host_different_port_clashes() {
        let table = run(vec![
            container(
                "one",
                "172.20.0.2",
                "running",
                &[("VIRTUAL_HOST", "x.docker"), ("VIRTUAL_PORT", "3000")],
                &[],
            ),
            container(
                "two",
                "172.20.0.3",
                "running",
                &[("VIRTUAL_HOST", "x.docker"), ("VIRTUAL_PORT", "4000")],
                &[],
            ),
        ]);

        assert_eq!(table.http.len(), 2);
        let rejected = table.http.iter().find(|r| r.rejected.is_some()).unwrap();
        assert_eq!(rejected.port, 4000);
        assert!(table.conflicts.iter().any(|c| c.contains("already served")));
    }

    #[test]
    fn tcp_reserved_port_rejected() {
        let table = run(vec![
            container(
                "redis",
                "172.20.0.7",
                "running",
                &[
                    ("VIRTUAL_HOST", "redis.docker"),
                    ("VIRTUAL_PORT", "6379"),
                    ("VIRTUAL_PROTO", "tcp"),
                ],
                &[],
            ),
            container(
                "bad",
                "172.20.0.8",
                "running",
                &[
                    ("VIRTUAL_HOST", "bad.docker"),
                    ("VIRTUAL_PORT", "80"),
                    ("VIRTUAL_PROTO", "tcp"),
                ],
                &[],
            ),
        ]);

        let good = table.tcp.iter().find(|r| r.port == 6379).unwrap();
        assert_eq!(good.upstream.servers, vec!["172.20.0.7:6379"]);
        assert!(good.rejected.is_none());

        let bad = table.tcp.iter().find(|r| r.port == 80).unwrap();
        assert!(bad.rejected.is_some());
        assert!(table.conflicts.iter().any(|c| c.contains("reserved")));
    }

    #[test]
    fn tcp_port_conflict_between_backends() {
        let table = run(vec![
            container(
                "a",
                "172.20.0.2",
                "running",
                &[
                    ("VIRTUAL_HOST", "a.docker"),
                    ("VIRTUAL_PORT", "6379"),
                    ("VIRTUAL_PROTO", "tcp"),
                ],
                &[],
            ),
            container(
                "b",
                "172.20.0.3",
                "running",
                &[
                    ("VIRTUAL_HOST", "b.docker"),
                    ("VIRTUAL_PORT", "6379"),
                    ("VIRTUAL_PROTO", "tcp"),
                ],
                &[],
            ),
        ]);

        assert_eq!(table.tcp.len(), 1);
        assert_eq!(table.tcp[0].containers, vec!["a"]);
        assert!(table.conflicts.iter().any(|c| c.contains("already routed")));
    }

    #[test]
    fn tcp_same_backend_two_hosts_merges() {
        let table = run(vec![container(
            "redis",
            "172.20.0.2",
            "running",
            &[
                ("VIRTUAL_HOST", "redis.docker,cache.docker"),
                ("VIRTUAL_PORT", "6379"),
                ("VIRTUAL_PROTO", "tcp"),
            ],
            &[],
        )]);

        assert_eq!(table.tcp.len(), 1);
        assert_eq!(table.tcp[0].hosts, vec!["cache.docker", "redis.docker"]);
    }

    #[test]
    fn stopped_and_managed_containers_ignored() {
        let table = run(vec![
            container(
                "old",
                "172.20.0.2",
                "exited",
                &[("VIRTUAL_HOST", "old.docker")],
                &[80],
            ),
            container(
                "aka_proxy",
                "172.20.0.9",
                "running",
                &[("VIRTUAL_HOST", "self.docker")],
                &[80],
            ),
        ]);

        assert!(table.is_empty());
    }

    #[test]
    fn tls_proto_produces_sni_route() {
        let table = run(vec![container(
            "db",
            "172.20.0.4",
            "running",
            &[
                ("VIRTUAL_HOST", "db.docker"),
                ("VIRTUAL_PORT", "5432"),
                ("VIRTUAL_PROTO", "tls"),
            ],
            &[],
        )]);

        assert_eq!(table.tls.len(), 1);
        assert_eq!(table.tls[0].port, 5432);
        assert!(table.http.is_empty());
    }

    #[test]
    fn missing_backend_ip_is_a_conflict() {
        let mut c = container("web", "", "running", &[("VIRTUAL_HOST", "x.docker")], &[80]);
        c.networks.clear();
        let table = run(vec![c]);

        assert!(table.http.is_empty());
        assert!(
            table
                .conflicts
                .iter()
                .any(|c| c.contains("no network address"))
        );
    }
}
