use leptos::{logging::log, prelude::*};

use crate::state::AppState;
pub use http::data::LoadingState;

pub struct HandleHealthcheckResponse {
    pub version: ReadSignal<LoadingState<String>>,
    pub healthy: ReadSignal<LoadingState<String>>,
    pub errors: ReadSignal<Option<Vec<String>>>,
}

pub fn handle_healthcheck_signals(state: AppState) -> HandleHealthcheckResponse {
    let app_client = state.app_client;

    let (version, set_version) = signal(LoadingState::Pending);
    let (healthy, set_healthy) = signal(LoadingState::Pending);
    let (errors, set_errors) = signal(None);

    let fetch_healthcheck_action = Action::new_unsync(move |_: &bool| {
        set_version.set(LoadingState::Loading);
        set_healthy.set(LoadingState::Loading);
        set_errors.set(None);

        async move {
            match app_client.get().healthcheck().await {
                Ok(payload) => {
                    set_version.set(LoadingState::Loaded(payload.version.clone()));
                    set_healthy.set(LoadingState::Loaded(payload.healthy.to_string()));
                    set_errors.set(None);
                }
                Err(err) => {
                    log!("error fetching: {:?}", err);

                    set_errors.set(Some(vec![err.to_string()]));
                }
            };
        }
    });

    // Poll on the same cadence as the status snapshot: a one-shot fetch leaves
    // the chip stale (and stuck red) once the API comes back.
    wasm_bindgen_futures::spawn_local(async move {
        loop {
            fetch_healthcheck_action.dispatch(true);

            gloo_timers::future::TimeoutFuture::new(
                crate::features::status::signals::REFRESH_SECS * 1000,
            )
            .await;
        }
    });

    HandleHealthcheckResponse {
        version,
        healthy,
        errors,
    }
}
