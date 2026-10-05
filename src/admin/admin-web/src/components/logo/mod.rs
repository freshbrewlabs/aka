use leptos::prelude::*;

#[component]
pub fn Logo(#[prop(default = "")] class: &'static str) -> impl IntoView {
    view! {
        <svg class=class viewBox="86 28 61 88" fill="none" xmlns="http://www.w3.org/2000/svg">
            <path d="M100 42V102" stroke="#e07b27" stroke-width="15" stroke-linecap="round"/>
            <path d="M100 72L133 42" stroke="#e07b27" stroke-width="15" stroke-linecap="round"/>
            <path d="M100 72L133 102" stroke="#e07b27" stroke-width="15" stroke-linecap="round"/>
        </svg>
    }
}
