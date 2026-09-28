use leptos::prelude::*;

#[component]
pub fn DashboardIcon(class: &'static str) -> impl IntoView {
    view! {
        <svg class=class viewBox="0 0 24 24">
            <rect x="3" y="3" width="7" height="7" />
            <rect x="14" y="3" width="7" height="7" />
            <rect x="14" y="14" width="7" height="7" />
            <rect x="3" y="14" width="7" height="7" />
        </svg>
    }
}

#[component]
pub fn RoutesIcon(class: &'static str) -> impl IntoView {
    view! {
        <svg class=class viewBox="0 0 24 24">
            <circle cx="6" cy="6" r="3" />
            <circle cx="18" cy="18" r="3" />
            <path d="M6 9v3a3 3 0 0 0 3 3h6" />
        </svg>
    }
}
