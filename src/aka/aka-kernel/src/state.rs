use serde::{Deserialize, Serialize};

use crate::route::RouteTable;

/// Snapshot the proxy daemon writes to `~/.aka/state.json` on every refresh;
/// read by `aka status` and `aka routes`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonState {
    pub daemon_pid: u32,
    /// Resolved config file path (empty string: defaults).
    pub config_path: String,
    pub updated_at: String,
    pub proxy: ProxyInfo,
    pub table: RouteTable,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProxyInfo {
    pub name: String,
    /// IP the proxy container currently has on the aka network.
    pub ip: Option<String>,
    /// Whether the rendered config passed `angie -t` on the last reload.
    pub last_reload_ok: bool,
}
