pub mod components;
pub mod features;
pub mod navigation;
pub mod state;

use leptos::prelude::*;

use components::app::components::App;

fn main() {
    leptos::mount::mount_to_body(|| view! { <App /> })
}
