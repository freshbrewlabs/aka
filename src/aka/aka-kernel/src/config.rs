use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[derive(Default)]
pub struct AkaConfig {
    pub dns: DnsConfig,
    pub proxy: ProxyConfig,
    pub resolv: ResolvConfig,
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
