use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// How a route's traffic is proxied. Comes from `VIRTUAL_PROTO`/`aka.proto`:
/// - `http`: angie terminates http and proxy_passes http to the backend
/// - `https`: angie terminates http and proxy_passes https to the backend
/// - `tcp`: angie `stream` proxies raw TCP on the route's port
/// - `tls`: angie `stream` routes by SNI and passes TLS through untouched
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RouteProto {
    Http,
    Https,
    Tcp,
    Tls,
}

impl RouteProto {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "" | "http" => Some(Self::Http),
            "https" => Some(Self::Https),
            "tcp" => Some(Self::Tcp),
            "tls" => Some(Self::Tls),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Https => "https",
            Self::Tcp => "tcp",
            Self::Tls => "tls",
        }
    }
}

/// A load-balanced group of backends declared by the containers behind it.
/// Backends are `ip:port` strings on docker networks the proxy is attached to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Upstream {
    /// Sorted, deduplicated backend addresses.
    pub servers: Vec<String>,
}

/// One host group sharing a backend set. `hosts` is sorted and deduplicated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Route {
    pub hosts: Vec<String>,
    pub proto: RouteProto,
    /// Backend port.
    pub port: u16,
    /// Names of the source containers (for status/diagnostics).
    pub containers: Vec<String>,
    pub upstream: Upstream,
    /// Set when the route could not be served (conflict note).
    pub rejected: Option<String>,
}

/// An https host whose TLS is terminated by angie because `<host>.crt` and
/// `<host>.key` exist in `ssl_certs_dir`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminatedRoute {
    pub host: String,
    pub cert: String,
    pub key: String,
    pub container: String,
    pub proto: RouteProto,
    pub upstream: Upstream,
}

/// A raw tcp route: angie `stream` listens on `port` and proxies to upstream.
/// The host is informational (DNS resolves it to the proxy; the port selects
/// the stream server).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TcpRoute {
    pub hosts: Vec<String>,
    pub port: u16,
    pub containers: Vec<String>,
    pub upstream: Upstream,
    pub rejected: Option<String>,
}

/// Everything the renderers need; produced by route discovery.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteTable {
    /// http + https vhosts (one entry per host group).
    pub http: Vec<Route>,
    /// tls sni-passthrough routes.
    pub tls: Vec<Route>,
    /// https hosts terminated by angie (certs available).
    pub terminated: Vec<TerminatedRoute>,
    /// raw tcp stream routes.
    pub tcp: Vec<TcpRoute>,
    /// Problems found during discovery (port clashes, unreachable backends...).
    pub conflicts: Vec<String>,
}

impl RouteTable {
    pub fn is_empty(&self) -> bool {
        self.http.is_empty()
            && self.tls.is_empty()
            && self.terminated.is_empty()
            && self.tcp.is_empty()
    }

    pub fn all_hosts(&self) -> BTreeSet<String> {
        let mut hosts = BTreeSet::new();
        hosts.extend(self.http.iter().flat_map(|r| r.hosts.iter().cloned()));
        hosts.extend(self.tls.iter().flat_map(|r| r.hosts.iter().cloned()));
        hosts.extend(self.terminated.iter().map(|r| r.host.clone()));
        hosts.extend(self.tcp.iter().flat_map(|r| r.hosts.iter().cloned()));
        hosts
    }
}
