//! The route table reshaped the way the dashboard reads it: one row per host,
//! hosts that hang off one base collected together, and everything narrowed by
//! what was typed into the filter box.
//!
//! Front-end only — the snapshot from `admin-api` is fetched whole and never
//! narrowed server side, so a filter can never hide a route from `aka status`.

use std::collections::BTreeMap;

use admin_kernel::entities::{RouteView, RoutesStatus};

/// Which listener a route came from, in the words the route tables use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RouteClass {
    /// http/https vhost.
    Vhost,
    /// SNI passthrough stream on :443.
    Sni,
    /// https host whose TLS angie terminates because a cert pair exists.
    Terminated,
    /// Raw tcp stream published on a host port.
    Tcp,
}

impl RouteClass {
    pub fn label(self) -> &'static str {
        match self {
            Self::Vhost => "http",
            Self::Sni => "sni",
            Self::Terminated => "tls terminated",
            Self::Tcp => "tcp",
        }
    }
}

/// One host and the route that serves it. A route group carrying several hosts
/// (raw tcp shares one stream listener per port) contributes one row per host,
/// so each base's card shows the routes its own hosts reach.
pub struct RouteRow {
    pub host: String,
    pub class: RouteClass,
    /// `http`, `https`, `tls` or `tcp`, straight from the snapshot.
    pub proto: String,
    /// Backend port, also the published port for tcp. Terminated hosts carry
    /// no port in the snapshot.
    pub port: Option<u16>,
    pub servers: Vec<String>,
    pub containers: Vec<String>,
    pub rejected: Option<String>,
}

impl RouteRow {
    pub fn base(&self) -> String {
        base_of(&self.host).to_string()
    }

    /// Everything the filter box may match, in one lowercased string.
    fn haystack(&self) -> String {
        let mut text = format!(
            "{} {} {} {}",
            self.host,
            self.base(),
            self.class.label(),
            self.proto
        );

        if let Some(port) = self.port {
            text.push_str(&format!(" {port}"));
        }

        for value in self.servers.iter().chain(self.containers.iter()) {
            text.push(' ');
            text.push_str(value);
        }

        text.to_lowercase()
    }

    fn matches(&self, terms: &[String]) -> bool {
        terms.is_empty() || text_matches(&self.haystack(), terms)
    }
}

/// Every route in the snapshot, one row per host, tables in snapshot order.
pub fn rows(routes: &RoutesStatus) -> Vec<RouteRow> {
    let mut rows = Vec::new();

    for (class, group) in [
        (RouteClass::Vhost, &routes.http),
        (RouteClass::Sni, &routes.tls),
        (RouteClass::Tcp, &routes.tcp),
    ] {
        rows.extend(group_rows(class, group));
    }

    rows.extend(routes.terminated.iter().map(|route| RouteRow {
        host: route.host.clone(),
        class: RouteClass::Terminated,
        proto: route.proto.clone(),
        port: None,
        servers: route.servers.clone(),
        containers: vec![route.container.clone()],
        rejected: None,
    }));

    rows
}

fn group_rows(class: RouteClass, routes: &[RouteView]) -> Vec<RouteRow> {
    routes
        .iter()
        .flat_map(|route| {
            route.hosts.iter().map(move |host| RouteRow {
                host: host.clone(),
                class,
                proto: route.proto.clone(),
                port: Some(route.port),
                servers: route.servers.clone(),
                containers: route.containers.clone(),
                rejected: route.rejected.clone(),
            })
        })
        .collect()
}

/// How many hosts the snapshot serves at all — the denominator of a filter.
/// Raw tcp counts each of its hosts, exactly like the cards do.
pub fn host_count(routes: &RoutesStatus) -> usize {
    routes
        .http
        .iter()
        .chain(routes.tls.iter())
        .chain(routes.tcp.iter())
        .map(|route| route.hosts.len())
        .sum::<usize>()
        + routes.terminated.len()
}

/// Hosts that share a base: `www.parking.localhost` and `db.parking.localhost`
/// both belong to `parking.localhost`, the `*.{base}.localhost` aka resolves.
pub struct RouteGroup {
    pub base: String,
    pub rows: Vec<RouteRow>,
}

/// The snapshot grouped by base and narrowed by `query`, bases in alphabetical
/// order and their hosts alphabetically within.
///
/// A row survives when every term appears somewhere in it — its host, base,
/// class, proto, port, upstream or containers. Because the base is part of the
/// match, naming a base keeps that base whole.
pub fn groups(routes: &RoutesStatus, query: &str) -> Vec<RouteGroup> {
    let terms = terms(query);

    let mut by_base: BTreeMap<String, Vec<RouteRow>> = BTreeMap::new();

    for row in rows(routes).into_iter().filter(|row| row.matches(&terms)) {
        by_base.entry(row.base()).or_default().push(row);
    }

    by_base
        .into_iter()
        .map(|(base, mut rows)| {
            rows.sort_by(|a, b| a.host.cmp(&b.host).then(a.class.cmp(&b.class)));

            RouteGroup { base, rows }
        })
        .collect()
}

/// The base a host hangs off: its last two labels, so anything below
/// `parking.localhost` belongs to `parking.localhost`. A host that is already
/// two labels or fewer (`aka.localhost`) is its own base.
pub fn base_of(host: &str) -> &str {
    match host.rfind('.') {
        // No dot at all, or a trailing dot: nothing to take off the front.
        None => host,
        Some(last) => match host[..last].rfind('.') {
            Some(cut) => &host[cut + 1..],
            None => host,
        },
    }
}

/// The filter box as terms: whitespace-separated, lowercased, every one of
/// them required.
pub fn terms(query: &str) -> Vec<String> {
    query
        .to_lowercase()
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

/// Every term is a substring of `text`, which gets lowercased here so callers
/// can pass it raw. An empty term list matches everything.
pub fn text_matches(text: &str, terms: &[String]) -> bool {
    let text = text.to_lowercase();

    terms.iter().all(|term| text.contains(term))
}

#[cfg(test)]
mod tests {
    use admin_kernel::entities::{RouteCounts, TerminatedView};

    use super::*;

    fn route(
        hosts: &[&str],
        proto: &str,
        port: u16,
        servers: &[&str],
        containers: &[&str],
    ) -> RouteView {
        RouteView {
            hosts: hosts.iter().map(|host| host.to_string()).collect(),
            proto: proto.to_string(),
            port,
            containers: containers
                .iter()
                .map(|container| container.to_string())
                .collect(),
            servers: servers.iter().map(|server| server.to_string()).collect(),
            rejected: None,
        }
    }

    fn snapshot() -> RoutesStatus {
        RoutesStatus {
            counts: RouteCounts {
                http: 1,
                tls: 1,
                terminated: 1,
                tcp: 1,
                conflicts: 0,
                rejected: 0,
            },
            http: vec![route(
                &["www.parking.localhost"],
                "http",
                8080,
                &["172.20.0.5:8080"],
                &["web"],
            )],
            tls: vec![route(
                &["db.parking.localhost"],
                "tls",
                5432,
                &["172.20.0.6:5432"],
                &["db"],
            )],
            terminated: vec![TerminatedView {
                host: "parking.localhost".to_string(),
                container: "web".to_string(),
                proto: "https".to_string(),
                servers: vec!["172.20.0.5:8080".to_string()],
            }],
            tcp: vec![route(
                &["redis.parking.localhost", "cache.docker"],
                "tcp",
                6379,
                &["172.20.0.7:6379"],
                &["redis"],
            )],
            conflicts: vec!["host x.docker already served by web".to_string()],
        }
    }

    #[test]
    fn a_host_belongs_to_its_last_two_labels() {
        assert_eq!(base_of("www.parking.localhost"), "parking.localhost");
        assert_eq!(base_of("a.b.c.x.test"), "x.test");
        assert_eq!(base_of("aka.localhost"), "aka.localhost");
        assert_eq!(base_of("localhost"), "localhost");
    }

    #[test]
    fn hosts_share_a_base_card_across_route_classes() {
        let groups = groups(&snapshot(), "");

        let bases: Vec<&str> = groups.iter().map(|group| group.base.as_str()).collect();
        assert_eq!(bases, vec!["cache.docker", "parking.localhost"]);

        let parking = &groups[1];
        assert_eq!(parking.rows.len(), 4);
        assert_eq!(
            parking
                .rows
                .iter()
                .map(|row| row.host.as_str())
                .collect::<Vec<_>>(),
            vec![
                "db.parking.localhost",
                "parking.localhost",
                "redis.parking.localhost",
                "www.parking.localhost"
            ],
            "hosts sort within their base, whatever class they came from"
        );
    }

    #[test]
    fn a_tcp_route_with_several_hosts_shows_up_under_each_base() {
        let groups = groups(&snapshot(), "");

        assert!(
            groups[0]
                .rows
                .iter()
                .any(|row| row.host == "cache.docker" && row.port == Some(6379))
        );
    }

    #[test]
    fn naming_a_base_keeps_that_base_whole() {
        let groups = groups(&snapshot(), "parking");

        assert_eq!(groups.len(), 1);
        assert_eq!(
            groups[0].rows.len(),
            4,
            "every host of the base, not just one"
        );
    }

    #[test]
    fn a_term_can_match_port_container_or_class() {
        let snapshot = snapshot();

        let by_port = groups(&snapshot, "6379");
        assert_eq!(
            by_port.iter().map(|g| g.base.as_str()).collect::<Vec<_>>(),
            vec!["cache.docker", "parking.localhost"]
        );

        let by_container = groups(&snapshot, "web");
        assert_eq!(
            by_container
                .iter()
                .map(|g| g.base.as_str())
                .collect::<Vec<_>>(),
            vec!["parking.localhost"]
        );
        assert_eq!(
            by_container[0].rows.len(),
            2,
            "web serves www and the terminated host"
        );

        let by_class = groups(&snapshot, "sni");
        assert_eq!(by_class[0].rows.len(), 1);
        assert_eq!(by_class[0].rows[0].host, "db.parking.localhost");
    }

    #[test]
    fn every_term_must_appear_in_the_same_row() {
        let matched = groups(&snapshot(), "redis 6379");

        // Both hosts of the tcp route are served by container redis on 6379,
        // so both bases survive; no other row mentions either term.
        assert_eq!(
            matched
                .iter()
                .map(|group| group.base.as_str())
                .collect::<Vec<_>>(),
            vec!["cache.docker", "parking.localhost"]
        );
        assert_eq!(matched[1].rows.len(), 1, "the base's other hosts drop out");

        // Neither term pair lives in one row together.
        assert!(groups(&snapshot(), "redis 8080").is_empty());
    }

    #[test]
    fn a_base_term_and_a_class_term_narrow_inside_the_base() {
        let matched = groups(&snapshot(), "parking sni");

        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].rows.len(), 1);
        assert_eq!(matched[0].rows[0].host, "db.parking.localhost");
    }

    #[test]
    fn matching_ignores_case() {
        assert_eq!(groups(&snapshot(), "WWW.PARKING").len(), 1);
    }

    #[test]
    fn host_count_counts_hosts_not_route_groups() {
        // The tcp group carries two hosts.
        assert_eq!(host_count(&snapshot()), 5);
    }

    #[test]
    fn conflicts_are_matched_as_plain_text() {
        let query = terms("x.docker");
        assert!(text_matches("host x.docker already served by web", &query));
        assert!(!text_matches("host y.docker already served by web", &query));
        assert!(text_matches("anything at all", &terms("")));
    }
}
