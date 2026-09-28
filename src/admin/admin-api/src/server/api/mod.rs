pub mod v1;

use axum::Router;
use provider::Provider;

pub fn router() -> Router<Provider> {
    Router::new().nest("/v1", v1::router())
}
