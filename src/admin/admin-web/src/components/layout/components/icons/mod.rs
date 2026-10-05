use leptos::prelude::*;

/// Status is the live state of the engine and every service, so the glyph is
/// a monitor showing a heartbeat trace: the status board itself.
#[component]
pub fn StatusIcon(class: &'static str) -> impl IntoView {
    view! {
        <svg class=class viewBox="0 0 24 24" stroke-linecap="round" stroke-linejoin="round">
            <rect x="2" y="3" width="20" height="14" rx="2" />
            <path d="M8 21h8" />
            <path d="M12 17v4" />
            <path d="M6 10.5h2.5l1.5 3 3-6 1.5 3H18" />
        </svg>
    }
}

/// A route is a path from one place to another, so the glyph is exactly that:
/// an origin node and a destination node joined by the way between them.
#[component]
pub fn RoutesIcon(class: &'static str) -> impl IntoView {
    view! {
        <svg class=class viewBox="0 0 24 24" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="5.5" cy="19" r="2.5" />
            <path d="M8 19h7.5a3.5 3.5 0 0 0 0-7H9a3.5 3.5 0 0 1 0-7h7" />
            <circle cx="18.5" cy="5" r="2.5" />
        </svg>
    }
}
