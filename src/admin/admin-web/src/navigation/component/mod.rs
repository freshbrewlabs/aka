use leptos::prelude::*;
use leptos_router::{components::*, path};

use crate::features::status::components::{RoutesPage, StatusPage};

#[component]
pub fn Navigation() -> impl IntoView {
    view! {
        <Routes fallback=|| "Not Found">
            <Route path=path!("/") view=|| view! { <StatusPage /> } />
            <Route path=path!("/dashboard") view=|| view! { <StatusPage /> } />
            <Route path=path!("/routes") view=|| view! { <RoutesPage /> } />
        </Routes>
    }
}
