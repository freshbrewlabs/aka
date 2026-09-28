use admin_http::healthcheck::HealthcheckRes;
use axum::{Json, extract::State, response::IntoResponse};
use provider::Provider;

#[tracing::instrument(name = "GET /api/v1/__meta/healthcheck", skip(_provider))]
pub async fn handler(_provider: State<Provider>) -> impl IntoResponse {
    Json(HealthcheckRes {
        healthy: true,
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}
