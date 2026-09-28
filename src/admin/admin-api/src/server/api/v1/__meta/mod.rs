pub mod healthcheck;

use axum::{Router, routing::get};
use provider::Provider;

pub fn router() -> Router<Provider> {
    Router::new().route("/healthcheck", get(healthcheck::handler))
}
