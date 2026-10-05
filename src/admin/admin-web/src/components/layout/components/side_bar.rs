use leptos::prelude::*;
use leptos_router::components::A;

use crate::components::layout::components::icons::{RoutesIcon, StatusIcon};
use crate::components::logo::Logo;
use crate::features::healthcheck::components::Healthcheck;

#[component]
pub fn SideBar() -> impl IntoView {
    view! {
        <aside class="sidebar" id="sidebar">
            <div class="sidebar-header">
                <A href="/dashboard" attr:class="logo">
                    <Logo class="logo-icon" />
                    <span>"aka admin"</span>
                </A>
            </div>

            <nav class="sidebar-nav">
                <div class="nav-section">
                    <ul class="nav-items">
                        <NavItem
                            text="Status"
                            href="/dashboard"
                            icon=move || view! { <StatusIcon class="nav-icon" /> }
                            badge=None
                        />
                        <NavItem
                            text="Routes"
                            href="/routes"
                            icon=move || view! { <RoutesIcon class="nav-icon" /> }
                            badge=None
                        />
                    </ul>
                </div>
            </nav>

            <div class="sidebar-footer">
                <div class="sidebar-divider"></div>
                <Healthcheck />
            </div>
        </aside>
    }
}

#[component]
pub fn NavItem<I, V>(
    text: &'static str,
    href: &'static str,
    icon: I,
    badge: Option<usize>,
) -> impl IntoView
where
    I: Fn() -> V + Send + 'static,
    V: IntoView + 'static,
{
    view! {
        <li class="nav-item">
            <A href=href attr:class="nav-link">
                {icon()}

                <span>{text}</span>

                {badge.map(|badge| view! {
                    <div class="nav-badge">{badge}</div>
                })}
            </A>
        </li>
    }
}
