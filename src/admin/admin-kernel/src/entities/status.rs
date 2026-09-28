//! The `aka status` snapshot as a wire contract.
//!
//! One report is everything `aka status` prints: the docker daemon version,
//! the managed service containers, the proxy daemon's health (read from
//! `~/.aka/state.json`) and the discovered route table. Field names follow
//! aka's own types so a dashboard row can be matched against the CLI line it
//! came from.

use serde::{Deserialize, Serialize};

/// One `aka status` run, produced fresh for every request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatusReport {
    /// RFC 3339 UTC stamp of when this report was built.
    pub generated_at: String,
    pub docker: DockerStatus,
    /// The managed service containers (`aka_dns`, `aka_proxy`).
    pub services: Vec<ServiceStatus>,
    pub proxyd: ProxydStatus,
    pub routes: RoutesStatus,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DockerStatus {
    pub reachable: bool,
    /// The daemon's reported version when reachable.
    pub version: Option<String>,
    /// Why the daemon is unreachable (also explains `unknown` service states).
    pub error: Option<String>,
}

/// One managed container.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServiceStatus {
    pub name: String,
    pub exists: bool,
    pub running: bool,
    /// Docker state (`running`, `exited`, ...), `absent` when the container
    /// does not exist, and `unknown` when the daemon itself was unreachable.
    pub state: String,
    pub image: String,
    /// IP on aka's proxy network.
    pub ip: Option<String>,
}

/// Liveness of `aka _proxyd` — the daemon that owns the route table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProxydState {
    /// No state file yet: `aka up` has never run.
    NeverRan,
    /// state.json was refreshed inside the freshness window.
    Running,
    /// The pid file still names the daemon's pid, but nothing has refreshed
    /// state.json for longer than the window.
    Stale,
    /// A state file exists, but nothing is refreshing it and the pid file
    /// names a different process.
    Stopped,
}

/// The proxyd block of `aka status`, plus how old the snapshot is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProxydStatus {
    pub state: ProxydState,
    /// Pid the daemon recorded in state.json.
    pub pid: Option<u32>,
    /// Pid in `~/.aka/proxyd.pid`, when that file exists.
    pub pid_file: Option<u32>,
    /// `updated_at` from state.json.
    pub updated_at: Option<String>,
    /// Seconds between `updated_at` and `StatusReport::generated_at`.
    pub age_seconds: Option<i64>,
    /// Whether the last `angie -t` passed.
    pub last_reload_ok: Option<bool>,
    /// Config the daemon resolved; `None` means aka used its defaults.
    pub config_path: Option<String>,
    /// Proxy container name and IP as the daemon last saw them.
    pub proxy_name: Option<String>,
    pub proxy_ip: Option<String>,
    /// state.json could not be read or parsed; the route tables are empty.
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RouteCounts {
    /// Accepted routes only, exactly like the CLI's summary line.
    pub http: usize,
    pub tls: usize,
    pub terminated: usize,
    pub tcp: usize,
    pub conflicts: usize,
    /// Routes discovery rejected; each one also produced a conflict line.
    pub rejected: usize,
}

/// An http/https, tls-passthrough or raw-tcp route group.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RouteView {
    /// Hosts sharing this backend set (raw tcp: informational only).
    pub hosts: Vec<String>,
    /// `http`, `https`, `tls` or `tcp`.
    pub proto: String,
    /// Backend port (for tcp routes also the published host port).
    pub port: u16,
    /// Containers that declared the route.
    pub containers: Vec<String>,
    /// Load-balanced `ip:port` backends.
    pub servers: Vec<String>,
    /// Why discovery rejected this route, when it did.
    pub rejected: Option<String>,
}

/// An https host whose TLS angie terminates because a cert pair exists.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TerminatedView {
    pub host: String,
    pub container: String,
    pub proto: String,
    pub servers: Vec<String>,
}

/// The route table, grouped the way `aka status` prints it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoutesStatus {
    pub counts: RouteCounts,
    /// http and https-backend vhosts.
    pub http: Vec<RouteView>,
    /// SNI passthrough routes.
    pub tls: Vec<RouteView>,
    pub terminated: Vec<TerminatedView>,
    /// Raw tcp stream routes.
    pub tcp: Vec<RouteView>,
    /// Port clashes, host collisions, unreachable backends.
    pub conflicts: Vec<String>,
}

impl RoutesStatus {
    /// The empty table: no daemon state to show.
    pub fn empty() -> Self {
        Self {
            counts: RouteCounts {
                http: 0,
                tls: 0,
                terminated: 0,
                tcp: 0,
                conflicts: 0,
                rejected: 0,
            },
            http: Vec::new(),
            tls: Vec::new(),
            terminated: Vec::new(),
            tcp: Vec::new(),
            conflicts: Vec::new(),
        }
    }
}
