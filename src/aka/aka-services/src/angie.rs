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
/// The down-page include lives in the rendered http dir but must not match
/// angie's `include http.d/*.conf` glob, so it carries a `.conf.inc` suffix
/// and is loaded through explicit `include` lines from the server blocks.
pub const DOWN_INCLUDE_FILENAME: &str = "aka-down.conf.inc";

/// `DOWN_INCLUDE_FILENAME` inside the proxy container (the http.d mount).
pub const DOWN_INCLUDE_PATH: &str = "/etc/angie/http.d/aka-down.conf.inc";

pub fn render_http(table: &RouteTable, proxy: &ProxyConfig) -> String {
    let mut out = String::new();
    out.push_str("# managed by aka (aka _proxyd); do not edit\n\n");
    out.push_str(
        "map $http_upgrade $aka_connection {\n    default upgrade;\n    ''      close;\n}\n\n",
    );
    // Unrouted hosts get the 502 down page: a Host aka knows nothing about
    // is a service that isn't running, and the page refreshes itself until
    // one appears.
    out.push_str(&format!(
        "server {{\n    listen {} default_server;\n    server_name _aka_default;\n\n    include {DOWN_INCLUDE_PATH};\n    return 502;\n}}\n\n",
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

    out.push_str(&format!(
        "    }}\n\n    include {DOWN_INCLUDE_PATH};\n}}\n\n"
    ));
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
        "# terminated {} ({})\nserver {{\n    listen {TERMINATION_PORT} ssl;\n    http2 on;\n    server_name {};\n    ssl_certificate     {CONTAINER_CERTS_DIR}/{};\n    ssl_certificate_key {CONTAINER_CERTS_DIR}/{};\n\n    location / {{\n        proxy_pass {scheme}://{upstream};\n        proxy_http_version 1.1;\n        proxy_set_header Host $host;\n        proxy_set_header X-Real-IP $remote_addr;\n        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;\n        proxy_set_header X-Forwarded-Proto https;\n        proxy_set_header Upgrade $http_upgrade;\n        proxy_set_header Connection $aka_connection;\n    }}\n\n    include {DOWN_INCLUDE_PATH};\n}}\n\n",
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
/// The include every rendered server block pulls in: 502/503/504 — the
/// codes angie emits when no upstream answers (refused, timed out, no live
/// server), and the default server's verdict for unrouted hosts — serve the
/// self-refreshing down page. The body comes from `return` in a named
/// location, so angie workers never read a host-mounted file and
/// error_page cannot recurse into itself (verified on angie 1.12.2).
pub fn render_down_include(admin_url: Option<&str>) -> String {
    format!(
        "# managed by aka (aka _proxyd); do not edit\nerror_page 502 503 504 @aka_down;\nlocation @aka_down {{\n    default_type text/html;\n    return 502 \"{}\";\n}}\n",
        nginx_quote_body(down_page(admin_url).trim_end_matches('\n'))
    )
}

/// Escape `s` for embedding inside an angie double-quoted config string.
fn nginx_quote_body(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Body served for dead/missing services. It reloads itself every 2 seconds
/// so it clears the moment the service answers again, and links to the
/// admin dashboard in a new tab (absent when the dashboard is disabled).
///
/// The body lands inside an angie `return` string, so it must contain no
/// `$` (angie would expand it as a variable). The failing host is filled in
/// client-side via `textContent` — never a config-side variable — so a
/// hostile Host header cannot inject markup.
fn down_page(admin_url: Option<&str>) -> String {
    let dashboard = match admin_url {
        Some(url) => format!(
            "<p><a href=\"{url}\" target=\"_blank\" rel=\"noopener\">Open the aka admin dashboard in a new tab</a></p>"
        ),
        None => "<p><small>The aka admin dashboard is disabled; start services from the CLI.</small></p>".to_owned(),
    };
    DOWN_PAGE_TEMPLATE.replace("<!--aka-dashboard-->", &dashboard)
}

const DOWN_PAGE_TEMPLATE: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta http-equiv="refresh" content="2">
<title>aka 502: no running service</title>
<style>
:root { color-scheme: dark }
body { margin: 0; min-height: 100vh; display: flex; align-items: center; justify-content: center; background: #10141a; color: #dce3ec; font-family: ui-sans-serif, system-ui, -apple-system, sans-serif }
main { max-width: 38rem; padding: 2.5rem; text-align: center }
.badge { display: inline-block; padding: 0.35rem 0.9rem; border: 1px solid #3b4657; border-radius: 999px; color: #ffb454; font-weight: 600; letter-spacing: 0.08em; margin-bottom: 1.2rem }
h1 { font-size: 1.2rem; margin: 0 0 1rem }
p { color: #9aa7b8; line-height: 1.6; margin: 0.6rem 0 }
code { background: #1a2230; border: 1px solid #2a3646; border-radius: 6px; padding: 0.1rem 0.4rem }
a { color: #7cc7ff }
small { color: #66748a }
</style>
</head>
<body>
<main>
<div class="badge">502</div>
<h1>No running service for <span id="host">this host</span></h1>
<p>aka has no reachable backend container for this host. Start the container (or fix its <code>VIRTUAL_HOST</code>); this page reloads every 2 seconds and clears itself once the service answers.</p>
<!--aka-dashboard-->
<p><small>aka dev proxy: generated page, do not edit</small></p>
</main>
<script>document.getElementById("host").textContent = location.host;</script>
</body>
</html>"#;

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

    #[test]
    fn every_http_server_includes_the_down_page() {
        let table = RouteTable {
            http: vec![route(
                &["a.docker"],
                RouteProto::Http,
                3000,
                &["172.20.0.5:3000"],
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

        let out = rendered_http(&table);
        // default server + plain vhost + terminated vhost
        assert_eq!(out.matches(DOWN_INCLUDE_PATH).count(), 3, "{out}");
        // unrouted hosts answer with the down page, not plain text
        assert!(out.contains("return 502;"));
        assert!(!out.contains("aka: no route"));
    }

    #[test]
    fn down_page_refreshes_itself_and_links_the_admin_dashboard() {
        let page = render_down_include(Some("http://aka.docker/"));
        assert!(page.contains("error_page 502 503 504 @aka_down;"));
        assert!(page.contains("return 502"));
        // self-refresh, new tab, and the quotes are angie-escaped
        assert!(page.contains("http-equiv=\\\"refresh\\\" content=\\\"2\\\""));
        assert!(page.contains("target=\\\"_blank\\\""));
        assert!(page.contains("href=\\\"http://aka.docker/\\\""));
        assert!(!render_down_include(None).contains("<a href"));
        assert!(render_down_include(None).contains("disabled"));
    }
}
