pub mod healthcheck;

use axum::{Router, routing::get};
use provider_rs::Provider;

pub fn router() -> Router<Provider> {
    Router::new().route("/healthcheck", get(healthcheck::handler))
}
