use leptos::prelude::*;

use super::signals::{HandleHealthcheckResponse, LoadingState, handle_healthcheck_signals};
use crate::state::AppState;

#[component]
pub fn Healthcheck() -> impl IntoView {
    let state = expect_context::<AppState>();

    let HandleHealthcheckResponse {
        version,
        healthy,
        errors,
    } = handle_healthcheck_signals(state);
    let status = move || {
        if errors.get().is_some() {
            ("healthcheck-error", "API Error")
        } else {
            match healthy.get() {
                LoadingState::Loaded(healthy) if healthy == "true" => {
                    ("healthcheck-ok", "API Connected")
                }
                LoadingState::Loaded(_) => ("healthcheck-warning", "API Unhealthy"),
                LoadingState::Loading | LoadingState::Pending => {
                    ("healthcheck-loading", "Connecting...")
                }
                LoadingState::Error(_) => ("healthcheck-error", "API Error"),
            }
        }
    };

    let version_display = move || match version.get() {
        LoadingState::Loaded(v) => Some(format!("v{v}")),
        _ => None,
    };

    view! {
        <div class="sidebar-healthcheck">
            <span class=move || format!("healthcheck-dot {}", status().0)></span>
            <span class="healthcheck-text">{move || status().1}</span>
            <span class="healthcheck-version">{version_display}</span>
        </div>
    }
}
