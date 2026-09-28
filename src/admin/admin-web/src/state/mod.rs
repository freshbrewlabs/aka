use leptos::prelude::RwSignal;
use web_sys::window;

use admin_rs::Client;

/// Compile-time override for an API that does **not** share the page's origin:
/// the `dev` profile containers bake in `http://localhost:3000`, and a native
/// loop is `ADMIN_API_URL=http://localhost:3000 trunk serve`.
pub const API_URL_OVERRIDE: Option<&str> = option_env!("ADMIN_API_URL");

/// The page's own origin: the `aka-admin` container serves the dashboard and
/// `/api/v1/*` from one port, so at `http://aka.docker/` the API is
/// `http://aka.docker`, at `http://localhost:3001/` it is
/// `http://localhost:3001`, and no origin ever needs to be guessed.
pub fn api_url() -> String {
    API_URL_OVERRIDE
        .map(str::to_string)
        .or_else(|| {
            window()
                .and_then(|window| window.location().origin().ok())
                .filter(|origin| !origin.is_empty() && origin != "null")
        })
        .unwrap_or_else(|| "http://localhost:3000".to_string())
}

#[derive(Clone, Copy)]
pub struct AppState {
    pub app_client: RwSignal<Client>,
}

impl AppState {
    pub fn new(api_url: &str) -> Result<Self, String> {
        let app_client =
            Client::new(api_url).map_err(|err| format!("error making app client: {:?}", err))?;

        let app_client = RwSignal::new(app_client);

        Ok(Self { app_client })
    }
}
