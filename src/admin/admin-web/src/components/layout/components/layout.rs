use leptos::prelude::*;
use leptos_router::components::Router;

use crate::{components::layout::components::side_bar::SideBar, navigation::Navigation};

#[component]
pub fn Layout() -> impl IntoView {
    view! {
        <div id="app">
            <Router>
                <SideBar />

                <main class="main-content">
                    <div class="main-inner">
                        <Navigation />
                    </div>
                </main>
            </Router>
        </div>
    }
}
