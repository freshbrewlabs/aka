use leptos::prelude::*;

use crate::components::layout::components::layout::Layout;
use crate::features::status::signals::spawn_status_state;
use crate::state::{AppState, api_url};

#[component]
pub fn App() -> impl IntoView {
    let state = AppState::new(&api_url()).expect("failed to create app state");

    provide_context(state);

    // The status snapshot lives here, at the root, so the polling loop and the
    // reactive owner it writes into live exactly as long as the app.
    provide_context(spawn_status_state(state));

    view! { <Layout /> }
}
