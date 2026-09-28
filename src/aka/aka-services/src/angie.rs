//! Render the angie (nginx-compatible) http + stream configs from a
//! `RouteTable`. Deterministic output: identical tables render byte-equal,
//! which lets the daemon skip no-op reloads.

use aka_kernel::config::ProxyConfig;
use aka_kernel::route::{Route, RouteProto, RouteTable, TcpRoute, TerminatedRoute};

pub const HTTP_FILENAME: &str = "aka.conf";
pub const STREAM_FILENAME: &str = "aka.conf";

/// Certificates are mounted at this path inside the proxy container.
pub const CONTAINER_CERTS_DIR: &str = "/etc/angie/certs";

/// The localhost port TLS-terminated vhosts live on; the stream server hops
/// into it after SNI preread (the client's TLS handshake continues end to
/// end between client and angie's http ssl server).
pub const TERMINATION_PORT: u16 = 8443;

pub fn render_http(table: &RouteTable, proxy: &ProxyConfig) -> String {
    let mut out = String::new();
    out.push_str("# managed by aka (aka _proxyd); do not edit\n\n");
    out.push_str(
        "map $http_upgrade $aka_connection {\n    default upgrade;\n    ''      close;\n}\n\n",
    );
    out.push_str(&format!(
        "server {{\n    listen {} default_server;\n    server_name _aka_default;\n    return 404 \"aka: no route for Host \\\"$host\\\"\\n\";\n}}\n\n",
        proxy.http_port
    ));

    for (index, route) in table
        .http
        .iter()
        .filter(|r| r.rejected.is_none() && !r.hosts.is_empty())
        .enumerate()
    {
        let name = upstream_name("h", index);
        out.push_str(&upstream_block(&name, &route.upstream.servers));
        out.push_str(&server_block(
            &route.hosts,
            proxy.http_port,
            route.proto,
            &name,
        ));
    }

    for (index, route) in table
        .terminated
        .iter()
        .filter(|r| !route_upstream_empty(r))
        .enumerate()
    {
        let name = upstream_name("ht", index);
        out.push_str(&upstream_block(&name, &route.upstream.servers));
        out.push_str(&terminated_server(route, &name));
    }

    out
}

fn route_upstream_empty(route: &TerminatedRoute) -> bool {
    route.upstream.servers.is_empty()
}

fn server_block(hosts: &[String], port: u16, proto: RouteProto, upstream: &str) -> String {
    let scheme = match proto {
        RouteProto::Https => "https",
        _ => "http",
    };

    let mut out = format!(
        "server {{\n    listen {port};\n    server_name {};\n\n    location / {{\n        proxy_pass {scheme}://{upstream};\n        proxy_http_version 1.1;\n        proxy_set_header Host $host;\n        proxy_set_header X-Real-IP $remote_addr;\n        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;\n        proxy_set_header X-Forwarded-Proto $scheme;\n        proxy_set_header Upgrade $http_upgrade;\n        proxy_set_header Connection $aka_connection;\n",
        hosts.join(" ")
    );

    if proto == RouteProto::Https {
        out.push_str("        proxy_ssl_server_name on;\n        proxy_ssl_verify off;\n        proxy_ssl_name $host;\n");
    }

    out.push_str("    }\n}\n\n");
    out
}

/// TLS-terminated vhost: SNI on the shared :8443 port selects the server
/// block; the cert pair comes from `ssl_certs_dir`.
fn terminated_server(route: &TerminatedRoute, upstream: &str) -> String {
    let scheme = match route.proto {
        RouteProto::Https => "https",
        _ => "http",
    };

    format!(
        "# terminated {} ({})\nserver {{\n    listen {TERMINATION_PORT} ssl;\n    http2 on;\n    server_name {};\n    ssl_certificate     {CONTAINER_CERTS_DIR}/{};\n    ssl_certificate_key {CONTAINER_CERTS_DIR}/{};\n\n    location / {{\n        proxy_pass {scheme}://{upstream};\n        proxy_http_version 1.1;\n        proxy_set_header Host $host;\n        proxy_set_header X-Real-IP $remote_addr;\n        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;\n        proxy_set_header X-Forwarded-Proto https;\n        proxy_set_header Upgrade $http_upgrade;\n        proxy_set_header Connection $aka_connection;\n    }}\n}}\n\n",
        route.host, route.container, route.host, route.cert, route.key
    )
}

fn upstream_block(name: &str, servers: &[String]) -> String {
    let mut out = format!("upstream {name} {{\n    zone {name} 64k;\n");
    for server in servers {
        out.push_str(&format!(
            "    server {server} max_fails=2 fail_timeout=5s;\n"
        ));
    }
    out.push_str("}\n\n");
    out
}

fn upstream_name(kind: &str, index: usize) -> String {
    format!("aka_{kind}{index}")
}

pub fn render_stream(table: &RouteTable, proxy: &ProxyConfig) -> String {
    let mut out = String::new();
    out.push_str("# managed by aka (aka _proxyd); do not edit\n\n");

    let tls_routes: Vec<&Route> = table
        .tls
        .iter()
        .filter(|r| r.rejected.is_none() && !r.hosts.is_empty() && !r.upstream.servers.is_empty())
        .collect();

    let sni_enabled = proxy.tls_enabled && (!tls_routes.is_empty() || !table.terminated.is_empty());

    if sni_enabled {
        out.push_str(
            "map $ssl_preread_server_name $aka_sni {\n    default            aka_reject;\n",
        );
        for route in &table.terminated {
            out.push_str(&format!("    {:<23}aka_local8443;\n", route.host));
        }
        for (index, route) in tls_routes.iter().enumerate() {
            for host in &route.hosts {
                out.push_str(&format!("    {:<23}{};\n", host, upstream_name("t", index)));
            }
        }
        out.push_str("}\n\n");

        out.push_str(&upstream_block("aka_reject", &["127.0.0.1:1".to_owned()]));
        if !table.terminated.is_empty() {
            out.push_str(&upstream_block(
                "aka_local8443",
                &[format!("127.0.0.1:{TERMINATION_PORT}")],
            ));
        }
        for (index, route) in tls_routes.iter().enumerate() {
            out.push_str(&upstream_block(
                &upstream_name("t", index),
                &route.upstream.servers,
            ));
        }

        out.push_str(&format!(
            "server {{\n    listen {};\n    ssl_preread on;\n    proxy_pass $aka_sni;\n    proxy_connect_timeout 5s;\n    proxy_timeout 1h;\n}}\n\n",
            proxy.tls_port
        ));
    }

    let tcp_routes: Vec<&TcpRoute> = table
        .tcp
        .iter()
        .filter(|r| r.rejected.is_none() && !r.upstream.servers.is_empty())
        .collect();

    for (index, route) in tcp_routes.iter().enumerate() {
        out.push_str(&format!("# tcp: {}\n", route.hosts.join(", ")));
        out.push_str(&upstream_block(
            &upstream_name("c", index),
            &route.upstream.servers,
        ));
        out.push_str(&format!(
            "server {{\n    listen {};\n    proxy_pass {};\n    proxy_connect_timeout 5s;\n    proxy_timeout 1h;\n}}\n\n",
            route.port,
            upstream_name("c", index)
        ));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use aka_kernel::route::{Route, Upstream};

    fn route(hosts: &[&str], proto: RouteProto, port: u16, servers: &[&str]) -> Route {
        Route {
            hosts: hosts.iter().map(|h| h.to_string()).collect(),
            proto,
            port,
            containers: vec!["web".into()],
            upstream: Upstream {
                servers: servers.iter().map(|s| s.to_string()).collect(),
            },
            rejected: None,
        }
    }

    fn rendered_http(table: &RouteTable) -> String {
        render_http(table, &ProxyConfig::default())
    }

    fn rendered_stream(table: &RouteTable) -> String {
        render_stream(table, &ProxyConfig::default())
    }

    #[test]
    fn http_vhost_renders_upstream_and_headers() {
        let table = RouteTable {
            http: vec![route(
                &["myapp.docker"],
                RouteProto::Http,
                3000,
                &["172.20.0.5:3000"],
            )],
            ..Default::default()
        };

        let out = rendered_http(&table);
        assert!(out.contains("upstream aka_h0 {"));
        assert!(out.contains("server 172.20.0.5:3000 max_fails=2 fail_timeout=5s;"));
        assert!(out.contains("server_name myapp.docker;"));
        assert!(out.contains("proxy_pass http://aka_h0;"));
        assert!(out.contains("proxy_set_header Upgrade $http_upgrade;"));
    }

    #[test]
    fn https_backend_uses_https_upstream() {
        let table = RouteTable {
            http: vec![route(
                &["secure.docker"],
                RouteProto::Https,
                8443,
                &["172.20.0.9:8443"],
            )],
            ..Default::default()
        };

        let out = rendered_http(&table);
        assert!(out.contains("proxy_pass https://aka_h0;"));
        assert!(out.contains("proxy_ssl_verify off;"));
    }

    #[test]
    fn rendering_is_deterministic() {
        let table = RouteTable {
            http: vec![
                route(
                    &["a.docker"],
                    RouteProto::Http,
                    3000,
                    &["172.20.0.5:3000", "172.20.0.6:3000"],
                ),
                route(&["b.docker"], RouteProto::Http, 3001, &["172.20.0.7:3001"]),
            ],
            tcp: vec![aka_kernel::route::TcpRoute {
                hosts: vec!["redis.docker".into()],
                port: 6379,
                containers: vec!["redis".into()],
                upstream: Upstream {
                    servers: vec!["172.20.0.8:6379".into()],
                },
                rejected: None,
            }],
            ..Default::default()
        };

        assert_eq!(rendered_http(&table), rendered_http(&table));
        assert_eq!(rendered_stream(&table), rendered_stream(&table));
    }

    #[test]
    fn stream_maps_sni_to_passthrough_and_termination() {
        let table = RouteTable {
            tls: vec![route(
                &["db.docker"],
                RouteProto::Tls,
                5432,
                &["172.20.0.4:5432"],
            )],
            terminated: vec![TerminatedRoute {
                host: "site.docker".into(),
                cert: "site.docker.crt".into(),
                key: "site.docker.key".into(),
                container: "web".into(),
                proto: RouteProto::Http,
                upstream: Upstream {
                    servers: vec!["172.20.0.5:3000".into()],
                },
            }],
            ..Default::default()
        };

        let out = rendered_stream(&table);
        // map entries are `key value;` — a semicolon after the key is a
        // config error (once shipped live).
        assert!(
            out.contains(&format!("{:<23}aka_t0;", "db.docker")),
            "{out}"
        );
        assert!(
            out.contains(&format!("{:<23}aka_local8443;", "site.docker")),
            "{out}"
        );
        assert!(!out.contains("db.docker;"));
        assert!(!out.contains("site.docker;"));
        assert!(out.contains("ssl_preread on;"));
        assert!(out.contains("listen 443;"));

        let http = rendered_http(&table);
        assert!(http.contains("listen 8443 ssl;"));
        assert!(http.contains(&format!("{}/site.docker.crt", CONTAINER_CERTS_DIR)));
        assert!(http.contains("proxy_pass http://aka_ht0;"));
    }

    #[test]
    fn tcp_stream_server_listens_on_route_port() {
        let table = RouteTable {
            tcp: vec![aka_kernel::route::TcpRoute {
                hosts: vec!["redis.docker".into()],
                port: 6379,
                containers: vec!["redis".into()],
                upstream: Upstream {
                    servers: vec!["172.20.0.8:6379".into()],
                },
                rejected: None,
            }],
            ..Default::default()
        };

        let out = rendered_stream(&table);
        assert!(out.contains("listen 6379;"));
        assert!(out.contains("proxy_pass aka_c0;"));
    }

    #[test]
    fn rejected_routes_are_not_rendered() {
        let mut rejected = route(
            &["dup.docker"],
            RouteProto::Http,
            4000,
            &["172.20.0.9:4000"],
        );
        rejected.rejected = Some("hosts claimed".into());
        let table = RouteTable {
            http: vec![rejected],
            ..Default::default()
        };

        assert!(!rendered_http(&table).contains("dup.docker"));
    }

    #[test]
    fn tls_disabled_omits_sni_block() {
        let proxy = ProxyConfig {
            tls_enabled: false,
            ..Default::default()
        };
        let table = RouteTable {
            tls: vec![route(
                &["db.docker"],
                RouteProto::Tls,
                5432,
                &["172.20.0.4:5432"],
            )],
            ..Default::default()
        };

        let out = render_stream(&table, &proxy);
        assert!(!out.contains("ssl_preread"));
        assert!(!out.contains("listen 443;"));
    }
}
