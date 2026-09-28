use axum::{
    Json,
    extract::State,
    response::{IntoResponse, Response},
};

use admin_services::GetStatusService;
use provider::Provider;

use super::map_service_error;

#[tracing::instrument(name = "GET /api/v1/status", skip(provider))]
pub async fn handler(State(provider): State<Provider>) -> Response {
    let service = provider.fetch_unchecked::<GetStatusService>();

    match service.get_status().await {
        Ok(report) => Json(report).into_response(),
        Err(err) => map_service_error(err),
    }
}
