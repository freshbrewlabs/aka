use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[derive(Default)]
pub struct AkaConfig {
    pub dns: DnsConfig,
    pub proxy: ProxyConfig,
    pub admin: AdminConfig,
    pub resolv: ResolvConfig,
}

impl AkaConfig {
    /// Virtual hosts the admin dashboard is routed on: the explicit
    /// `[admin] hosts` list when set, else `aka.<domain>` for every domain
    /// aka serves (dnsmasq's `#` catch-all excluded — `aka.#` is not a
    /// hostname). Lowercased and deduplicated, like discovery splits them.
    pub fn admin_hosts(&self) -> Vec<String> {
        let raw = if self.admin.hosts.is_empty() {
            self.dns
                .domains
                .iter()
                .filter(|d| !d.domain.is_empty() && d.domain != "#")
                .map(|d| format!("aka.{}", d.domain))
                .collect()
        } else {
            self.admin.hosts.clone()
        };

        raw.into_iter()
            .map(|h| h.to_ascii_lowercase())
            .filter(|h| !h.is_empty())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DnsConfig {
    pub enabled: bool,
    /// Domains resolved (wildcard subdomains included) and the address returned.
    pub domains: Vec<DomainAddress>,
    pub container_name: String,
    pub image: String,
    /// Host port dnsmasq listens on (must be 53 when the system resolver
    /// points at it without a `port` line; macOS resolver files may carry one).
    pub port: u16,
    /// Host interface/IP to publish the dns ports on.
    pub bind_ip: String,
    /// What to do when something else already holds the dns port.
    pub kill_others: KillOthers,
}

impl Default for DnsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            domains: vec![DomainAddress::default()],
            container_name: "aka_dns".into(),
            image: "dewey4iv/aka-dns:latest".into(),
            port: 53,
            bind_ip: "127.0.0.1".into(),
            kill_others: KillOthers::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DomainAddress {
    /// TLD to serve, `#` for a catch-all wildcard (dnsmasq `address=/#/ip`).
    pub domain: String,
    /// Answer returned for queries against the domain.
    pub address: String,
}

impl Default for DomainAddress {
    fn default() -> Self {
        Self {
            domain: "docker".into(),
            address: "127.0.0.1".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProxyConfig {
    pub enabled: bool,
    pub container_name: String,
    pub image: String,
    /// Docker network the proxy container lives on.
    pub network: String,
    /// Host port published for http traffic (container listens on the same).
    pub http_port: u16,
    /// Publish the tls port and enable SNI routing (passthrough + termination).
    pub tls_enabled: bool,
    pub tls_port: u16,
    /// Host directory with `<host>.crt`/`<host>.key` pairs for TLS termination.
    /// Empty string disables termination; matching hosts fall back to SNI
    /// passthrough.
    pub ssl_certs_dir: String,
    /// Host interface/IP to publish the proxy ports on.
    pub bind_ip: String,
    /// Restart policy for the managed containers.
    pub restart: String,
    /// Largest request body the proxy accepts, in angie's own size syntax: a
    /// number of bytes with an optional `k`/`m`/`g` suffix, or `0` for no
    /// limit (the default). angie's built-in 1 MiB cap answers 413 to uploads
    /// that are ordinary on a dev box, and aka's `stream` routes never had a
    /// cap to begin with. The empty string renders no directive at all, which
    /// restores angie's default — and leaves the directive to a drop-in of
    /// your own in `~/.aka/proxy.d/http`.
    pub max_body_size: String,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            container_name: "aka_proxy".into(),
            image: "dewey4iv/aka-proxy:latest".into(),
            network: "aka".into(),
            http_port: 80,
            tls_enabled: true,
            tls_port: 443,
            ssl_certs_dir: String::new(),
            bind_ip: "127.0.0.1".into(),
            restart: "unless-stopped".into(),
            max_body_size: "0".into(),
        }
    }
}

impl ProxyConfig {
    /// Host ports angie cannot also use as raw stream listeners.
    pub fn reserved_stream_ports(&self) -> Vec<u16> {
        let mut ports = vec![self.http_port];
        if self.tls_enabled {
            ports.push(self.tls_port);
        }
        ports
    }

    /// The `client_max_body_size` to render, or `None` when the value is empty
    /// and angie's own default should stand.
    pub fn body_size_setting(&self) -> Option<&str> {
        (!self.max_body_size.is_empty()).then_some(self.max_body_size.as_str())
    }

    /// angie's size syntax: `0`, a byte count, or a byte count with exactly one
    /// `k`/`m`/`g` suffix.
    pub fn is_body_size(value: &str) -> bool {
        let digits = match value.chars().last() {
            Some('k' | 'K' | 'm' | 'M' | 'g' | 'G') => &value[..value.len() - 1],
            Some(_) => value,
            None => return false,
        };
        !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
    }
}

/// The `aka status` web dashboard, run beside dns + proxy. Unauthenticated
/// by design — every published port must stay on loopback.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AdminConfig {
    pub enabled: bool,
    pub container_name: String,
    pub image: String,
    /// Host port published for direct access (the container listens on 80;
    /// proxied access goes through the proxy's http vhosts instead).
    pub host_port: u16,
    /// Host interface/IP the dashboard port is published on. Loopback only:
    /// the dashboard has no auth.
    pub bind_ip: String,
    /// Host docker socket, mounted at the standard container path so the
    /// dashboard can talk to the engine.
    pub docker_socket: String,
    /// vhosts the proxy routes to the dashboard; empty means
    /// `aka.<domain>` for every `[dns.domains]` entry.
    pub hosts: Vec<String>,
}

impl Default for AdminConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            container_name: "aka_admin".into(),
            image: "dewey4iv/aka-admin:latest".into(),
            host_port: 3001,
            bind_ip: "127.0.0.1".into(),
            docker_socket: "/var/run/docker.sock".into(),
            hosts: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ResolvConfig {
    pub enabled: bool,
    /// Nameserver written into the resolver files / resolv.conf chunk.
    pub nameserver: String,
    /// Port the nameserver listens on (defaults to the dns port at load time).
    pub port: Option<u16>,
}

impl Default for ResolvConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            nameserver: "127.0.0.1".into(),
            port: None,
        }
    }
}

/// ask | yes/true | no/false — mirrors dory's `kill_others`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum KillOthers {
    #[default]
    Ask,
    Bool(bool),
    Str(String),
}

impl KillOthers {
    /// `None` means ask interactively; otherwise a pre-decided answer.
    pub fn answer(&self) -> Option<bool> {
        match self {
            KillOthers::Ask => None,
            KillOthers::Bool(b) => Some(*b),
            KillOthers::Str(s) => match s.to_ascii_lowercase().as_str() {
                "yes" | "y" | "true" => Some(true),
                "no" | "n" | "false" => Some(false),
                _ => None,
            },
        }
    }
}
