//! The status snapshot: docker + `~/.aka/state.json` → [`StatusReport`].

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, SecondsFormat, Utc};

use admin_kernel::BoxedError;
use admin_kernel::entities::status::{
    DockerStatus, ProxydState, ProxydStatus, RouteCounts, RouteView, RoutesStatus, ServiceStatus,
    StatusReport, TerminatedView,
};
use aka_docker::AkaDocker;
use aka_kernel::config::AkaConfig;
use aka_kernel::route::{Route, RouteTable, TcpRoute, TerminatedRoute};
use aka_kernel::state::DaemonState;
use aka_services::lifecycle::{self, ServiceStatus as AkaServiceStatus};
use aka_services::paths::AkaPaths;
use aka_services::statefile;

/// proxyd rewrites state.json on every docker event and on a 20s resync, so a
/// snapshot younger than this window can only have been written by a live
/// daemon.
pub const PROXYD_FRESH_SECS: i64 = 60;

pub type GetStatusService = Arc<dyn GetStatusTrait>;

#[async_trait]
pub trait GetStatusTrait: Send + Sync + 'static {
    async fn get_status(&self) -> Result<StatusReport, BoxedError>;
}

pub struct GetStatusUseCase {
    config: AkaConfig,
    paths: AkaPaths,
    docker: AkaDocker,
}

impl GetStatusUseCase {
    pub fn new(config: AkaConfig, paths: AkaPaths, docker: AkaDocker) -> Self {
        Self {
            config,
            paths,
            docker,
        }
    }
}

#[async_trait]
impl GetStatusTrait for GetStatusUseCase {
    #[tracing::instrument(name = "GetStatusUseCase::get_status", skip(self))]
    async fn get_status(&self) -> Result<StatusReport, BoxedError> {
        let now = Utc::now();

        // `aka status` prints the daemon version and the container table; both
        // come from the engine and both fail together when docker is down.
        let (docker, services) = match self.docker.daemon_version().await {
            Ok(version) => match lifecycle::status(&self.docker, &self.config).await {
                Ok(statuses) => (
                    DockerStatus {
                        reachable: true,
                        version: Some(version),
                        error: None,
                    },
                    statuses.into_iter().map(service_view).collect(),
                ),
                Err(err) => {
                    tracing::warn!(message = "service listing failed", error = %err);

                    (
                        DockerStatus {
                            reachable: true,
                            version: Some(version),
                            error: Some(err.to_string()),
                        },
                        unknown_services(&self.config),
                    )
                }
            },
            Err(err) => (
                DockerStatus {
                    reachable: false,
                    version: None,
                    error: Some(err.to_string()),
                },
                unknown_services(&self.config),
            ),
        };

        let (state, state_error) = match statefile::read(&self.paths.state_file()) {
            Ok(state) => (state, None),
            Err(err) => (None, Some(err.to_string())),
        };

        let routes = match state.as_ref() {
            Some(state) => routes_view(&state.table),
            None => RoutesStatus::empty(),
        };

        let proxyd = proxyd_view(
            state,
            state_error,
            pid_file_pid(&self.paths.pid_file()),
            now,
        );

        Ok(StatusReport {
            generated_at: now.to_rfc3339_opts(SecondsFormat::Secs, true),
            docker,
            services,
            proxyd,
            routes,
        })
    }
}

/// Pid recorded in aka's pid file.
///
/// `aka status` also asks the OS whether that pid is alive (`kill -0`). That
/// probe is meaningless when admin-api runs in a container — its own pid
/// namespace does not contain the daemon — so liveness here comes from
/// state.json freshness instead.
fn pid_file_pid(path: &Path) -> Option<u32> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn proxyd_view(
    state: Option<DaemonState>,
    read_error: Option<String>,
    pid_file: Option<u32>,
    now: DateTime<Utc>,
) -> ProxydStatus {
    let Some(state) = state else {
        return ProxydStatus {
            // A file we could not read exists, so the daemon ran at some
            // point; no file at all means `aka up` never ran.
            state: if read_error.is_some() {
                ProxydState::Stopped
            } else {
                ProxydState::NeverRan
            },
            pid: None,
            pid_file,
            updated_at: None,
            age_seconds: None,
            last_reload_ok: None,
            config_path: None,
            proxy_name: None,
            proxy_ip: None,
            error: read_error,
        };
    };

    let age_seconds = DateTime::parse_from_rfc3339(&state.updated_at)
        .map(|updated| {
            now.signed_duration_since(updated.with_timezone(&Utc))
                .num_seconds()
        })
        .ok();

    // A timestamp in the future still counts as fresh: the daemon is writing.
    let fresh = age_seconds.is_some_and(|age| age <= PROXYD_FRESH_SECS);

    let state_kind = if fresh {
        ProxydState::Running
    } else if pid_file == Some(state.daemon_pid) {
        ProxydState::Stale
    } else {
        ProxydState::Stopped
    };

    let error = age_seconds.is_none().then(|| {
        format!(
            "state.json has an unparseable updated_at: {:?}",
            state.updated_at
        )
    });

    ProxydStatus {
        state: state_kind,
        pid: Some(state.daemon_pid),
        pid_file,
        updated_at: Some(state.updated_at.clone()),
        age_seconds,
        last_reload_ok: Some(state.proxy.last_reload_ok),
        config_path: (!state.config_path.is_empty()).then_some(state.config_path),
        proxy_name: Some(state.proxy.name),
        proxy_ip: state.proxy.ip,
        error,
    }
}

fn routes_view(table: &RouteTable) -> RoutesStatus {
    let counts = RouteCounts {
        // The CLI's summary line counts accepted routes only.
        http: accepted(&table.http),
        tls: accepted(&table.tls),
        terminated: table.terminated.len(),
        tcp: table.tcp.iter().filter(|r| r.rejected.is_none()).count(),
        conflicts: table.conflicts.len(),
        rejected: table
            .http
            .iter()
            .chain(table.tls.iter())
            .filter(|r| r.rejected.is_some())
            .count()
            + table.tcp.iter().filter(|r| r.rejected.is_some()).count(),
    };

    RoutesStatus {
        counts,
        http: table.http.iter().map(route_view).collect(),
        tls: table.tls.iter().map(route_view).collect(),
        terminated: table.terminated.iter().map(terminated_view).collect(),
        tcp: table.tcp.iter().map(tcp_view).collect(),
        conflicts: table.conflicts.clone(),
    }
}

fn accepted(routes: &[Route]) -> usize {
    routes.iter().filter(|r| r.rejected.is_none()).count()
}

fn route_view(route: &Route) -> RouteView {
    RouteView {
        hosts: route.hosts.clone(),
        proto: route.proto.as_str().to_string(),
        port: route.port,
        containers: route.containers.clone(),
        servers: route.upstream.servers.clone(),
        rejected: route.rejected.clone(),
    }
}

fn tcp_view(route: &TcpRoute) -> RouteView {
    RouteView {
        hosts: route.hosts.clone(),
        proto: "tcp".to_string(),
        port: route.port,
        containers: route.containers.clone(),
        servers: route.upstream.servers.clone(),
        rejected: route.rejected.clone(),
    }
}

fn terminated_view(route: &TerminatedRoute) -> TerminatedView {
    TerminatedView {
        host: route.host.clone(),
        container: route.container.clone(),
        proto: route.proto.as_str().to_string(),
        servers: route.upstream.servers.clone(),
    }
}

fn service_view(status: AkaServiceStatus) -> ServiceStatus {
    ServiceStatus {
        name: status.name,
        exists: status.exists,
        running: status.running,
        state: status.state,
        image: status.image,
        ip: status.ip,
    }
}

/// The managed containers with an unknown state, for when docker itself is
/// unreachable: the dashboard still names the services it cannot see.
fn unknown_services(config: &AkaConfig) -> Vec<ServiceStatus> {
    [&config.dns.container_name, &config.proxy.container_name]
        .into_iter()
        .map(|name| ServiceStatus {
            name: name.clone(),
            exists: false,
            running: false,
            state: "unknown".to_string(),
            image: String::new(),
            ip: None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use aka_kernel::route::{RouteProto, Upstream};
    use aka_kernel::state::ProxyInfo;
    use chrono::Duration;

    fn now() -> DateTime<Utc> {
        Utc::now()
    }

    fn stamp(offset_secs: i64) -> String {
        (Utc::now() + Duration::seconds(offset_secs)).to_rfc3339_opts(SecondsFormat::Secs, true)
    }

    fn state(updated_at: &str) -> DaemonState {
        DaemonState {
            daemon_pid: 4242,
            config_path: "/Users/dev/.aka.toml".into(),
            updated_at: updated_at.into(),
            proxy: ProxyInfo {
                name: "aka_proxy".into(),
                ip: Some("172.20.0.2".into()),
                last_reload_ok: true,
            },
            table: RouteTable::default(),
        }
    }

    #[test]
    fn no_state_file_means_the_daemon_never_ran() {
        let proxyd = proxyd_view(None, None, None, now());

        assert_eq!(proxyd.state, ProxydState::NeverRan);
        assert_eq!(proxyd.error, None);
        assert_eq!(proxyd.age_seconds, None);
    }

    #[test]
    fn an_unreadable_state_file_is_stopped_with_the_reason() {
        let proxyd = proxyd_view(None, Some("state.json is not json".into()), Some(11), now());

        assert_eq!(proxyd.state, ProxydState::Stopped);
        assert_eq!(proxyd.error.as_deref(), Some("state.json is not json"));
        assert_eq!(proxyd.pid_file, Some(11));
    }

    #[test]
    fn a_fresh_snapshot_is_running_even_without_a_pid_file() {
        let proxyd = proxyd_view(Some(state(&stamp(-3))), None, None, now());

        assert_eq!(proxyd.state, ProxydState::Running);
        assert!((0..=5).contains(&proxyd.age_seconds.expect("age")));
        assert_eq!(proxyd.proxy_ip.as_deref(), Some("172.20.0.2"));
        assert_eq!(proxyd.config_path.as_deref(), Some("/Users/dev/.aka.toml"));
        assert_eq!(proxyd.last_reload_ok, Some(true));
    }

    #[test]
    fn a_future_timestamp_still_counts_as_fresh() {
        let proxyd = proxyd_view(Some(state(&stamp(20))), None, None, now());

        assert_eq!(proxyd.state, ProxydState::Running);
    }

    #[test]
    fn an_old_snapshot_with_a_matching_pid_file_is_stale() {
        let proxyd = proxyd_view(Some(state(&stamp(-300))), None, Some(4242), now());

        assert_eq!(proxyd.state, ProxydState::Stale);
        assert!(proxyd.age_seconds.expect("age") > PROXYD_FRESH_SECS);
    }

    #[test]
    fn an_old_snapshot_with_a_foreign_pid_is_stopped() {
        let proxyd = proxyd_view(Some(state(&stamp(-300))), None, Some(99), now());

        assert_eq!(proxyd.state, ProxydState::Stopped);
    }

    #[test]
    fn an_unparseable_timestamp_is_reported_not_fresh() {
        let proxyd = proxyd_view(Some(state("yesterday")), None, Some(4242), now());

        assert_eq!(proxyd.age_seconds, None);
        assert_eq!(proxyd.state, ProxydState::Stale);
        let error = proxyd.error.expect("diagnostic");
        assert!(error.contains("updated_at"), "{error}");
    }

    #[test]
    fn an_empty_config_path_reads_as_defaults() {
        let mut state = state(&stamp(0));
        state.config_path = String::new();

        let proxyd = proxyd_view(Some(state), None, None, now());

        assert_eq!(proxyd.config_path, None);
    }

    fn route(hosts: &[&str], proto: &str, rejected: Option<&str>) -> Route {
        Route {
            hosts: hosts.iter().map(|h| (*h).to_string()).collect(),
            proto: RouteProto::parse(proto).expect("proto"),
            port: 80,
            containers: vec!["demo-web".into()],
            upstream: Upstream {
                servers: vec!["172.20.0.5:80".into()],
            },
            rejected: rejected.map(str::to_string),
        }
    }

    #[test]
    fn counts_match_the_cli_summary_line() {
        let table = RouteTable {
            http: vec![
                route(&["demo.docker"], "http", None),
                route(
                    &["taken.docker"],
                    "http",
                    Some("hosts claimed by demo.docker"),
                ),
            ],
            tls: vec![route(&["secure.docker"], "tls", None)],
            terminated: vec![TerminatedRoute {
                host: "secure.docker".into(),
                cert: "/certs/secure.docker.crt".into(),
                key: "/certs/secure.docker.key".into(),
                container: "secure-app".into(),
                proto: RouteProto::Https,
                upstream: Upstream {
                    servers: vec!["172.20.0.7:8443".into()],
                },
            }],
            tcp: vec![TcpRoute {
                hosts: vec!["cache.docker".into()],
                port: 6379,
                containers: vec!["demo-cache".into()],
                upstream: Upstream {
                    servers: vec!["172.20.0.8:6379".into()],
                },
                rejected: Some("port 6379 is reserved".into()),
            }],
            conflicts: vec!["a".into(), "b".into()],
        };

        let routes = routes_view(&table);

        assert_eq!(routes.counts.http, 1);
        assert_eq!(routes.counts.tls, 1);
        assert_eq!(routes.counts.terminated, 1);
        assert_eq!(routes.counts.tcp, 0);
        assert_eq!(routes.counts.conflicts, 2);
        assert_eq!(routes.counts.rejected, 2);
    }

    /// The CLI hides rejected routes; the dashboard keeps them with their
    /// note, so a rejected host is never silently missing.
    #[test]
    fn rejected_routes_are_kept_with_their_note() {
        let table = RouteTable {
            http: vec![route(
                &["taken.docker"],
                "http",
                Some("hosts claimed by demo.docker"),
            )],
            ..Default::default()
        };

        let routes = routes_view(&table);

        assert_eq!(routes.http.len(), 1);
        assert_eq!(
            routes.http[0].rejected.as_deref(),
            Some("hosts claimed by demo.docker")
        );
        assert_eq!(routes.http[0].proto, "http");
        assert_eq!(routes.http[0].servers, ["172.20.0.5:80"]);
    }

    #[test]
    fn tcp_and_terminated_routes_carry_their_labels() {
        let table = RouteTable {
            tcp: vec![TcpRoute {
                hosts: vec!["cache.docker".into()],
                port: 6379,
                containers: vec!["demo-cache".into()],
                upstream: Upstream {
                    servers: vec!["172.20.0.8:6379".into()],
                },
                rejected: None,
            }],
            terminated: vec![TerminatedRoute {
                host: "secure.docker".into(),
                cert: String::new(),
                key: String::new(),
                container: "secure-app".into(),
                proto: RouteProto::Https,
                upstream: Upstream {
                    servers: vec!["172.20.0.7:8443".into()],
                },
            }],
            ..Default::default()
        };

        let routes = routes_view(&table);

        assert_eq!(routes.tcp[0].proto, "tcp");
        assert_eq!(routes.tcp[0].port, 6379);
        assert_eq!(routes.terminated[0].proto, "https");
        assert_eq!(routes.terminated[0].container, "secure-app");
    }

    #[test]
    fn unreachable_docker_still_names_the_managed_services() {
        let services = unknown_services(&AkaConfig::default());

        assert_eq!(services.len(), 2);
        assert_eq!(services[0].name, "aka_dns");
        assert_eq!(services[0].state, "unknown");
        assert!(!services[0].running);
    }

    #[test]
    fn an_empty_table_reads_as_no_routes() {
        let routes = routes_view(&RouteTable::default());

        assert_eq!(routes, RoutesStatus::empty());
    }
}
