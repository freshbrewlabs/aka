use leptos::prelude::*;
use leptos_router::components::A;

use admin_kernel::entities::{
    ProxydState, ProxydStatus, RouteView, RoutesStatus, ServiceStatus, StatusReport, TerminatedView,
};

use super::signals::{LoadingState, StatusState};

const DASH: &str = "—";

fn proxyd_label(state: ProxydState) -> &'static str {
    match state {
        ProxydState::Running => "running",
        ProxydState::Stale => "stale",
        ProxydState::Stopped => "not running",
        ProxydState::NeverRan => "never ran",
    }
}

fn proxyd_class(state: ProxydState) -> &'static str {
    match state {
        ProxydState::Running => "chip-ok",
        ProxydState::Stale => "chip-warning",
        ProxydState::Stopped | ProxydState::NeverRan => "chip-error",
    }
}

fn state_class(state: &str) -> &'static str {
    match state {
        "running" => "chip-ok",
        "unknown" => "chip-warning",
        _ => "chip-error",
    }
}

fn age_label(age: Option<i64>) -> String {
    match age {
        None => DASH.to_string(),
        Some(age) if age <= 5 => "just now".to_string(),
        Some(age) if age < 60 => format!("{age}s ago"),
        Some(age) if age < 3600 => format!("{}m ago", age / 60),
        Some(age) => format!("{}h ago", age / 3600),
    }
}

fn or_dash(value: Option<String>) -> String {
    value.unwrap_or_else(|| DASH.to_string())
}

fn or_dash_pid(value: Option<u32>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| DASH.to_string())
}

/// One label/value row — the shape `aka status` prints as `key: value`.
fn detail(label: &'static str, value: String) -> AnyView {
    view! {
        <div class="detail-row">
            <span class="detail-label">{label}</span>
            <span class="detail-value">{value}</span>
        </div>
    }
    .into_any()
}

fn chip(class: &'static str, text: String) -> AnyView {
    view! { <span class=format!("chip {}", class)>{text}</span> }.into_any()
}

fn warning(text: String) -> AnyView {
    view! {
        <div class="form-errors">
            <p class="form-error">{text}</p>
        </div>
    }
    .into_any()
}

fn metric(value: String, label: &'static str) -> AnyView {
    view! {
        <div class="metric-card">
            <span class="metric-value metric-text">{value}</span>
            <span class="metric-label">{label}</span>
        </div>
    }
    .into_any()
}

// ------------------------------------------------------------------- status

#[component]
pub fn StatusPage() -> impl IntoView {
    let status = expect_context::<StatusState>();
    let refresh = status.refresh;

    let snapshot = move || match status.status.get() {
        LoadingState::Loaded(report) => format!("snapshot {}", report.generated_at),
        _ => String::new(),
    };

    view! {
        <div class="page-header">
            <h1 class="page-title">"aka status"</h1>

            <div class="row-actions">
                <span class="muted">{snapshot}</span>

                <button class="btn" on:click=move |_| refresh.run(())>"Refresh"</button>
            </div>
        </div>

        {move || match status.status.get() {
            LoadingState::Pending | LoadingState::Loading => {
                view! { <p class="muted">"Loading..."</p> }.into_any()
            }
            LoadingState::Error(err) => warning(format!("admin-api unreachable: {err}")),
            LoadingState::Loaded(report) => status_body(report),
        }}
    }
}

fn status_body(report: StatusReport) -> AnyView {
    let StatusReport {
        docker,
        services,
        proxyd,
        routes,
        ..
    } = report;

    let counts = routes.counts.clone();
    let total = counts.http + counts.tls + counts.terminated + counts.tcp;
    let summary = format!(
        "{} http, {} tls, {} terminated, {} tcp, {} conflicts",
        counts.http, counts.tls, counts.terminated, counts.tcp, counts.conflicts,
    );

    view! {
        {docker.error.map(|err| warning(format!("docker engine: {err}")))}

        <div class="metric-grid">
            {metric(or_dash(docker.version), "Docker daemon")}
            {metric(proxyd_label(proxyd.state).to_string(), "proxyd")}
            {metric(total.to_string(), "Routes")}
            {metric(counts.conflicts.to_string(), "Conflicts")}
        </div>

        {services_card(services)}
        {proxyd_card(proxyd)}
        {conflicts_card(routes.conflicts, counts.rejected)}

        <div class="card">
            <h2 class="card-title">"Route summary"</h2>

            <p class="muted">{summary}</p>

            <A href="/routes">"Full route table"</A>
        </div>
    }
    .into_any()
}

fn services_card(services: Vec<ServiceStatus>) -> AnyView {
    view! {
        <div class="card">
            <h2 class="card-title">"Managed services"</h2>

            <table class="data-table">
                <thead>
                    <tr>
                        <th>"Service"</th>
                        <th>"State"</th>
                        <th>"Image"</th>
                        <th>"IP"</th>
                    </tr>
                </thead>
                <tbody>
                    {services
                        .into_iter()
                        .map(|service| {
                            let state = service.state.clone();
                            let image = or_dash((!service.image.is_empty()).then_some(service.image));

                            view! {
                                <tr>
                                    <td>{service.name}</td>
                                    <td>{chip(state_class(&state), state)}</td>
                                    <td class="muted">{image}</td>
                                    <td class="muted">{or_dash(service.ip)}</td>
                                </tr>
                            }
                        })
                        .collect_view()}
                </tbody>
            </table>
        </div>
    }
    .into_any()
}

fn proxyd_card(proxyd: ProxydStatus) -> AnyView {
    let ProxydStatus {
        state,
        pid,
        pid_file,
        updated_at,
        age_seconds,
        last_reload_ok,
        config_path,
        proxy_name,
        proxy_ip,
        error,
    } = proxyd;

    let refreshed = match updated_at {
        Some(updated_at) => format!("{} ({})", updated_at, age_label(age_seconds)),
        None => DASH.to_string(),
    };

    let reload = match last_reload_ok {
        Some(true) => "ok",
        Some(false) => "pending",
        None => DASH,
    };

    let proxy = match (proxy_name, proxy_ip) {
        (Some(name), Some(ip)) => format!("{} @ {}", name, ip),
        (Some(name), None) => name,
        _ => DASH.to_string(),
    };

    let config = config_path.unwrap_or_else(|| "defaults (no config file)".to_string());

    let hint = match state {
        ProxydState::NeverRan => "No state.json yet — run `aka up`.",
        ProxydState::Stale => "proxyd stopped refreshing state.json; `aka restart` brings it back.",
        _ => "",
    };

    view! {
        <div class="card">
            <div class="card-header">
                <h2 class="card-title">"Proxy daemon"</h2>

                {chip(proxyd_class(state), proxyd_label(state).to_string())}
            </div>

            {error.map(warning)}

            {(!hint.is_empty()).then(|| view! { <p class="muted">{hint}</p> })}

            <div class="detail-list">
                {detail("pid", or_dash_pid(pid))}
                {detail("pid file", or_dash_pid(pid_file))}
                {detail("refreshed", refreshed)}
                {detail("angie reload", reload.to_string())}
                {detail("config", config)}
                {detail("proxy", proxy)}
            </div>
        </div>
    }
    .into_any()
}

// ------------------------------------------------------------------- routes

#[component]
pub fn RoutesPage() -> impl IntoView {
    let status = expect_context::<StatusState>();
    let refresh = status.refresh;

    view! {
        <div class="page-header">
            <h1 class="page-title">"Routes"</h1>

            <div class="row-actions">
                <button class="btn" on:click=move |_| refresh.run(())>"Refresh"</button>
            </div>
        </div>

        {move || match status.status.get() {
            LoadingState::Pending | LoadingState::Loading => {
                view! { <p class="muted">"Loading..."</p> }.into_any()
            }
            LoadingState::Error(err) => warning(format!("admin-api unreachable: {err}")),
            LoadingState::Loaded(report) => routes_body(report.routes),
        }}
    }
}

fn routes_body(routes: RoutesStatus) -> AnyView {
    let RoutesStatus {
        counts,
        http,
        tls,
        terminated,
        tcp,
        conflicts,
    } = routes;

    view! {
        <div class="metric-grid">
            {metric(counts.http.to_string(), "http / https vhosts")}
            {metric(counts.tls.to_string(), "tls passthrough")}
            {metric(counts.terminated.to_string(), "tls terminated")}
            {metric(counts.tcp.to_string(), "raw tcp")}
            {metric(counts.rejected.to_string(), "rejected")}
        </div>

        {route_table("http / https vhosts", None, http)}
        {route_table("tls sni passthrough", Some("routed by SNI on :443"), tls)}
        {terminated_table(terminated)}
        {route_table("raw tcp", Some("published on :port"), tcp)}

        {conflicts_list(conflicts)}
    }
    .into_any()
}

fn route_table(title: &'static str, note: Option<&'static str>, routes: Vec<RouteView>) -> AnyView {
    let count = routes.len();
    let label = match note {
        Some(note) => format!("{} route(s) · {}", count, note),
        None => format!("{} route(s)", count),
    };

    view! {
        <div class="card">
            <div class="card-header">
                <h2 class="card-title">{title}</h2>

                <span class="muted">{label}</span>
            </div>

            {if count == 0 {
                view! { <p class="muted">"No routes in this class."</p> }.into_any()
            } else {
                view! {
                    <table class="data-table">
                        <thead>
                            <tr>
                                <th>"Hosts"</th>
                                <th>"Proto"</th>
                                <th>"Port"</th>
                                <th>"Upstream"</th>
                                <th>"Containers"</th>
                            </tr>
                        </thead>
                        <tbody>
                            {routes
                                .into_iter()
                                .map(|route| {
                                    let rejected = route.rejected;
                                    let row_class = if rejected.is_some() { "row-rejected" } else { "" };

                                    view! {
                                        <tr class=row_class>
                                            <td>
                                                {route.hosts.join(", ")}

                                                {rejected.map(|note| view! {
                                                    <div class="row-note">{note}</div>
                                                })}
                                            </td>
                                            <td class="muted">{route.proto}</td>
                                            <td class="muted">{route.port.to_string()}</td>
                                            <td class="muted">{route.servers.join(", ")}</td>
                                            <td class="muted">{route.containers.join(", ")}</td>
                                        </tr>
                                    }
                                })
                                .collect_view()}
                        </tbody>
                    </table>
                }
                .into_any()
            }}
        </div>
    }
    .into_any()
}

fn terminated_table(terminated: Vec<TerminatedView>) -> AnyView {
    let count = terminated.len();

    view! {
        <div class="card">
            <div class="card-header">
                <h2 class="card-title">"tls terminated"</h2>

                <span class="muted">{format!("{} host(s) · cert pair in ssl_certs_dir", count)}</span>
            </div>

            {if count == 0 {
                view! { <p class="muted">"No terminated hosts."</p> }.into_any()
            } else {
                view! {
                    <table class="data-table">
                        <thead>
                            <tr>
                                <th>"Host"</th>
                                <th>"Proto"</th>
                                <th>"Upstream"</th>
                                <th>"Container"</th>
                            </tr>
                        </thead>
                        <tbody>
                            {terminated
                                .into_iter()
                                .map(|route| view! {
                                    <tr>
                                        <td>{route.host}</td>
                                        <td class="muted">{route.proto}</td>
                                        <td class="muted">{route.servers.join(", ")}</td>
                                        <td class="muted">{route.container}</td>
                                    </tr>
                                })
                                .collect_view()}
                        </tbody>
                    </table>
                }
                .into_any()
            }}
        </div>
    }
    .into_any()
}

/// Shown on the status page only when something is actually wrong.
fn conflicts_card(conflicts: Vec<String>, rejected: usize) -> AnyView {
    if conflicts.is_empty() && rejected == 0 {
        return ().into_any();
    }

    conflicts_list(conflicts)
}

fn conflicts_list(conflicts: Vec<String>) -> AnyView {
    if conflicts.is_empty() {
        return ().into_any();
    }

    view! {
        <div class="card card-warning">
            <h2 class="card-title">"Conflicts"</h2>

            <ul class="plain-list">
                {conflicts
                    .into_iter()
                    .map(|conflict| view! { <li>{conflict}</li> })
                    .collect_view()}
            </ul>
        </div>
    }
    .into_any()
}
