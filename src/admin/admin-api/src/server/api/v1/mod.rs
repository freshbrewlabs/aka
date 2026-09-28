pub mod __meta;
pub mod status;

use axum::{
    Json, Router,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use provider::Provider;
use serde::Serialize;

use crate::BoxedError;

#[derive(Serialize)]
pub struct ErrorRes {
    pub message: String,
}

/// Read-only, unauthenticated by design: this stack runs on the developer's
/// own machine and exposes nothing but aka's own status. Every route here is
/// served straight from the router — no auth extractor, no session, no token.
pub fn router() -> Router<Provider> {
    Router::new()
        .nest("/__meta", __meta::router())
        .route("/status", get(status::handler))
}

pub(crate) fn map_service_error(err: BoxedError) -> Response {
    tracing::error!(message = "service error", error = %err);

    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorRes {
            message: err.to_string(),
        }),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use admin_kernel::entities::StatusReport;
    use admin_services::{GetStatusService, GetStatusTrait};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use provider::Provider;
    use tower::ServiceExt;

    use super::*;

    struct OkStatus;

    #[async_trait::async_trait]
    impl GetStatusTrait for OkStatus {
        async fn get_status(&self) -> Result<StatusReport, BoxedError> {
            Ok(StatusReport {
                generated_at: "2026-01-01T00:00:00Z".into(),
                docker: admin_http::DockerStatus {
                    reachable: true,
                    version: Some("27.4.0".into()),
                    error: None,
                },
                services: Vec::new(),
                proxyd: admin_http::ProxydStatus {
                    state: admin_http::ProxydState::Running,
                    pid: Some(1),
                    pid_file: Some(1),
                    updated_at: Some("2026-01-01T00:00:00Z".into()),
                    age_seconds: Some(0),
                    last_reload_ok: Some(true),
                    config_path: None,
                    proxy_name: None,
                    proxy_ip: None,
                    error: None,
                },
                routes: admin_http::RoutesStatus::empty(),
            })
        }
    }

    struct FailingStatus;

    #[async_trait::async_trait]
    impl GetStatusTrait for FailingStatus {
        async fn get_status(&self) -> Result<StatusReport, BoxedError> {
            Err("state.json exploded".into())
        }
    }

    fn app_with(service: GetStatusService) -> Router {
        let mut provider = Provider::new();
        provider.store::<GetStatusService>(service);
        crate::server::app(provider)
    }

    fn ok_app() -> Router {
        app_with(Arc::new(OkStatus))
    }

    async fn ask(app: Router, req: Request<Body>) -> StatusCode {
        app.oneshot(req).await.unwrap().status()
    }

    /// The contract of this whole stack: it runs locally and asks for nothing.
    #[tokio::test]
    async fn every_route_answers_without_credentials() {
        for uri in ["/api/v1/status", "/api/v1/__meta/healthcheck"] {
            let status = ask(ok_app(), Request::get(uri).body(Body::empty()).unwrap()).await;

            assert_eq!(status, StatusCode::OK, "GET {uri}");
        }
    }

    /// An `Authorization` header must be ignored, not validated: if someone
    /// later bolts auth on, the dashboard stops rather than half-works.
    #[tokio::test]
    async fn authorization_headers_are_ignored() {
        for header in ["Bearer garbage", "Basic dXNlcjpwYXNz", "Token abc"] {
            let status = ask(
                ok_app(),
                Request::get("/api/v1/status")
                    .header(axum::http::header::AUTHORIZATION, header)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;

            assert_eq!(status, StatusCode::OK, "header: {header:?}");
        }
    }

    #[tokio::test]
    async fn a_service_failure_becomes_a_500_with_the_reason() {
        let app = app_with(Arc::new(FailingStatus));

        let res = app
            .oneshot(Request::get("/api/v1/status").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);

        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .expect("body collected");
        let json: serde_json::Value = serde_json::from_slice(&bytes).expect("json error body");

        assert_eq!(json["message"], "state.json exploded");
    }

    #[tokio::test]
    async fn unknown_route_is_not_found() {
        let status = ask(
            ok_app(),
            Request::get("/api/v1/nope").body(Body::empty()).unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::NOT_FOUND);
    }
}
