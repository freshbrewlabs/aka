//! dnsmasq argument construction for the aka-dns image. The image's
//! entrypoint executes `dnsmasq --no-daemon $DNSMASQ_ARGS`, so the CLI owns
//! the config (dory passed domain/addr pairs as container args; an env var
//! keeps quoting simple).

use aka_kernel::config::DnsConfig;

/// Full dnsmasq argument string for `DNSMASQ_ARGS`.
pub fn dns_args(dns: &DnsConfig) -> String {
    let mut args = vec![format!("--port={}", dns.port)];

    for domain in &dns.domains {
        // dnsmasq `address=/domain/IP` answers for the domain and every
        // subdomain; `#` is the catch-all wildcard dory supports.
        args.push(format!(
            "--address=/{}{}",
            domain.domain,
            address_suffix(&domain.address)
        ));
    }

    args.push("--no-resolv".into());
    args.push("--no-poll".into());
    args.push("--bogus-priv".into());

    args.join(" ")
}

fn address_suffix(address: &str) -> String {
    if address.is_empty() {
        String::new()
    } else {
        format!("/{address}")
    }
}

/// The `port` line written to macOS resolver files (matches the dns port).
pub fn resolver_port_line(dns: &DnsConfig) -> String {
    format!("port {}", dns.port)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aka_kernel::config::DomainAddress;

    #[test]
    fn default_args() {
        let args = dns_args(&DnsConfig::default());
        assert!(args.contains("--port=53"));
        assert!(args.contains("--address=/docker/127.0.0.1"));
        assert!(args.contains("--no-resolv"));
    }

    #[test]
    fn multiple_domains_and_wildcard() {
        let dns = DnsConfig {
            domains: vec![
                DomainAddress {
                    domain: "test".into(),
                    address: "127.0.0.1".into(),
                },
                DomainAddress {
                    domain: "#".into(),
                    address: "192.168.66.110".into(),
                },
            ],
            ..Default::default()
        };

        let args = dns_args(&dns);
        assert!(args.contains("--address=/test/127.0.0.1"));
        assert!(args.contains("--address=/#/192.168.66.110"));
    }
}
