use leptos::prelude::*;

/// The Status page reports whether the engine and every service are live, so
/// the glyph is a monitor trace, not a generic dashboard grid.
#[component]
pub fn StatusIcon(class: &'static str) -> impl IntoView {
    view! {
        <svg class=class viewBox="0 0 24 24" stroke-linecap="round" stroke-linejoin="round">
            <path d="M2 12h4l3 8 4-16 3 8h6" />
        </svg>
    }
}

/// Routes are mappings from a host to a backend: an endpoint at each end with
/// a path between them.
#[component]
pub fn RoutesIcon(class: &'static str) -> impl IntoView {
    view! {
        <svg class=class viewBox="0 0 24 24" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="6" cy="19" r="3" />
            <circle cx="18" cy="5" r="3" />
            <path d="M9 19h8a3.5 3.5 0 0 0 0-7H7a3.5 3.5 0 0 1 0-7h8" />
        </svg>
    }
}
