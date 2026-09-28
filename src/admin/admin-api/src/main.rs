pub mod provider;
pub mod server;

use tracing::info;

use admin_kernel::BoxedError;

#[tokio::main]
async fn main() -> Result<(), BoxedError> {
    let (environment, tracing_level) = telemetry::get_logging_envs()?;

    let subscriber = telemetry::get_subscriber(
        "admin-api",
        env!("CARGO_PKG_VERSION"),
        &environment,
        &tracing_level,
        std::io::stdout,
    );

    info!("starting admin-api");

    telemetry::init_subscriber(subscriber);

    let provider = provider::init().await?;

    // 80 inside containers (aka routes VIRTUAL_HOST straight to it); override
    // (e.g. `PORT=3000 cargo run --bin admin-api`) on the host, where the proxy
    // owns port 80.
    let port = std::env::var("PORT").unwrap_or_else(|_| "80".to_string());

    server::start(provider, format!("0.0.0.0:{port}")).await?;

    Ok(())
}
