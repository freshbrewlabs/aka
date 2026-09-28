use leptos::{logging::log, prelude::*};

use admin_kernel::entities::StatusReport;

use crate::state::AppState;
pub use http::data::LoadingState;

/// proxyd rewrites state.json every 20s and on every docker event, and the
/// snapshot itself costs two docker calls plus one file read — a few seconds
/// keeps the dashboard honest without hammering the engine.
pub const REFRESH_SECS: u32 = 5;

/// The shared snapshot behind both pages.
#[derive(Clone, Copy)]
pub struct StatusState {
    pub status: ReadSignal<LoadingState<StatusReport>>,
    pub refresh: Callback<()>,
}

/// Fetches immediately, then every `REFRESH_SECS`. Called from the app root so
/// the loop and the reactive owner it writes into die together.
pub fn spawn_status_state(state: AppState) -> StatusState {
    let (status, set_status) = signal(LoadingState::Pending);

    let fetch_status_action = Action::new_unsync(move |_: &()| {
        let client = state.app_client.get_untracked();

        async move {
            match client.status().await {
                Ok(report) => set_status.set(LoadingState::Loaded(report)),
                Err(err) => {
                    log!("error fetching aka status: {:?}", err);

                    set_status.set(LoadingState::Error(err.to_string()));
                }
            }
        }
    });

    wasm_bindgen_futures::spawn_local(async move {
        loop {
            fetch_status_action.dispatch(());

            gloo_timers::future::TimeoutFuture::new(REFRESH_SECS * 1000).await;
        }
    });

    let refresh = Callback::new(move |_| {
        fetch_status_action.dispatch(());
    });

    StatusState { status, refresh }
}
