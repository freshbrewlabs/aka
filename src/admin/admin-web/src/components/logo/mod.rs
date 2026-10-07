use leptos::prelude::*;

/// The full `aka` wordmark — `a` `k` `a` from `docs/design/aka-logo*.svg`.
/// The ink letterforms inherit the surrounding text colour; the `k` carries
/// the brand amber.
#[component]
pub fn Logo(class: &'static str) -> impl IntoView {
    view! {
        <svg
            class=class
            viewBox="-12 24 252 94"
            fill="none"
            stroke-width="15"
            stroke-linecap="round"
            stroke-linejoin="round"
            xmlns="http://www.w3.org/2000/svg"
        >
            <circle cx="40" cy="70" r="30" stroke="currentColor" />
            <path d="M70 42V102" stroke="currentColor" />
            <circle cx="190" cy="70" r="30" stroke="currentColor" />
            <path d="M220 42V102" stroke="currentColor" />
            <path d="M100 42V102" stroke="#e07b27" />
            <path d="M100 72L133 42" stroke="#e07b27" />
            <path d="M100 72L133 102" stroke="#e07b27" />
        </svg>
    }
}
