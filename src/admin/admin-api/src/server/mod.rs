pub mod api;

use std::path::{Path, PathBuf};

use axum::Router;
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};
use tracing::info;

use provider::Provider;

use crate::BoxedError;

/// Where the built dashboard lives. Inside the `aka-admin` image the release
/// build of `admin-web` is copied here; on a dev machine the directory usually
/// does not exist, and `trunk serve` delivers the frontend instead.
pub const DEFAULT_WEB_DIR: &str = "/usr/share/aka-admin/web";

pub fn web_dir() -> PathBuf {
    std::env::var("ADMIN_WEB_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_WEB_DIR))
}

pub async fn start(
    provider: Provider,
    addr: impl tokio::net::ToSocketAddrs,
) -> Result<(), BoxedError> {
    info!("starting http server");

    let tcp_listener = tokio::net::TcpListener::bind(addr).await?;

    let bound = tcp_listener.local_addr()?.to_string();

    info!(message = "listening on local", address = bound);

    axum::serve(tcp_listener, app(provider)).await?;

    Ok(())
}

/// One container, one origin: `/api/*` is the API and everything else is the
/// dashboard build, so the browser never crosses an origin.
///
/// Permissive CORS stays for the dev loop, where `trunk serve` runs on a
/// different port than the API.
fn app(provider: Provider) -> Router {
    app_from(provider, &web_dir())
}

/// `web_dir` is a parameter, not an environment read, so tests can point it at
/// a throwaway build.
pub fn app_from(provider: Provider, web_dir: &Path) -> Router {
    let router = Router::new().nest("/api", api::router().fallback(unknown_endpoint));

    let router = match static_files(web_dir) {
        Some(spa) => {
            info!(message = "serving dashboard", dir = %web_dir.display());

            router.fallback_service(spa)
        }
        None => {
            info!(
                message = "no dashboard build found; serving the API only",
                dir = %web_dir.display()
            );

            router
        }
    };

    router.layer(CorsLayer::permissive()).with_state(provider)
}

/// An unknown endpoint stays an API problem: `/api/*` answers JSON 404 even
/// when a dashboard build is present, so a typo is never answered with HTML.
async fn unknown_endpoint() -> (axum::http::StatusCode, axum::Json<UnknownEndpoint>) {
    (
        axum::http::StatusCode::NOT_FOUND,
        axum::Json(UnknownEndpoint {
            error: "no such endpoint",
        }),
    )
}

#[derive(serde::Serialize)]
struct UnknownEndpoint {
    error: &'static str,
}

/// Static assets with an `index.html` fallback, so client-side routes
/// (`/dashboard`, `/routes`) load the app instead of 404ing. `None` when no
/// build is present.
fn static_files(dir: &Path) -> Option<ServeDir<ServeFile>> {
    let index = dir.join("index.html");

    // Trunk names its wasm/css bundles by hash and injects them into
    // index.html, so only index.html itself needs to be resolved here.
    index
        .is_file()
        .then(|| ServeDir::new(dir).fallback(ServeFile::new(index)))
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use admin_kernel::entities::{
        DockerStatus, ProxydState, ProxydStatus, RoutesStatus, StatusReport,
    };
    use admin_services::{GetStatusService, GetStatusTrait};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use provider::Provider;
    use tower::ServiceExt;

    use super::*;
    use std::path::Path;

    struct OkStatus;

    #[async_trait::async_trait]
    impl GetStatusTrait for OkStatus {
        async fn get_status(&self) -> Result<StatusReport, BoxedError> {
            Ok(StatusReport {
                generated_at: "2026-01-01T00:00:00Z".into(),
                docker: DockerStatus {
                    reachable: true,
                    version: Some("27.4.0".into()),
                    error: None,
                },
                services: Vec::new(),
                proxyd: ProxydStatus {
                    state: ProxydState::Running,
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
                routes: RoutesStatus::empty(),
            })
        }
    }

    /// A throwaway dashboard build: index.html plus one hashed asset, the
    /// shape trunk produces.
    fn fake_web_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "aka-admin-web-{}-{:?}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("index.html"), b"<html>dashboard</html>").unwrap();
        let mut asset = std::fs::File::create(dir.join("admin-web-abc123_bg.wasm")).unwrap();
        asset.write_all(b"wasm").unwrap();
        dir
    }

    fn app_with_web(dir: PathBuf) -> Router {
        let mut provider = Provider::new();
        provider.store::<GetStatusService>(Arc::new(OkStatus));
        app_from(provider, &dir)
    }

    async fn body_of(res: axum::response::Response) -> String {
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .expect("body collected");
        String::from_utf8_lossy(&bytes).to_string()
    }

    #[tokio::test]
    async fn a_client_side_route_serves_the_app() {
        let dir = fake_web_dir();
        let res = app_with_web(dir.clone())
            .oneshot(Request::get("/routes").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(body_of(res).await, "<html>dashboard</html>");
    }

    #[tokio::test]
    async fn a_built_asset_is_served_directly() {
        let dir = fake_web_dir();
        let res = app_with_web(dir.clone())
            .oneshot(
                Request::get("/admin-web-abc123_bg.wasm")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(body_of(res).await, "wasm");
    }

    /// The API keeps winning over the SPA fallback on the same origin.
    #[tokio::test]
    async fn api_routes_are_not_shadowed_by_the_dashboard() {
        let dir = fake_web_dir();
        let res = app_with_web(dir.clone())
            .oneshot(
                Request::get("/api/v1/__meta/healthcheck")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(res.status(), StatusCode::OK);
        assert!(body_of(res).await.contains("healthy"));
    }

    /// A typo under `/api` is a JSON 404, never the dashboard build.
    #[tokio::test]
    async fn an_api_route_still_404s_instead_of_serving_the_app() {
        let dir = fake_web_dir();
        let res = app_with_web(dir.clone())
            .oneshot(Request::get("/api/v1/nope").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(res.status(), StatusCode::NOT_FOUND);
        assert!(body_of(res).await.contains("no such endpoint"));
    }

    /// Without a build (the native dev loop) the API must not answer `/` with
    /// a body it cannot produce.
    #[tokio::test]
    async fn no_dashboard_build_serves_the_api_only() {
        let mut provider = Provider::new();
        provider.store::<GetStatusService>(Arc::new(OkStatus));

        let res = app_from(provider, Path::new("/aka-admin-web-absent"))
            .oneshot(Request::get("/dashboard").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }
}
