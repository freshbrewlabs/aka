use leptos::prelude::*;

#[component]
pub fn Logo(#[prop(default = "")] class: &'static str) -> impl IntoView {
    view! {
        <svg class=class viewBox="0 0 512 512" fill="none" xmlns="http://www.w3.org/2000/svg">
            <rect x="10" y="10" width="492" height="492" rx="106" stroke="#d4872a" stroke-width="20"/>
            <path d="M85 256 L149 256 L192 170 L256 342 L320 213 L363 256 L427 256"
                  stroke="#d4872a" stroke-width="32" stroke-linecap="round" stroke-linejoin="round"/>
        </svg>
    }
}
