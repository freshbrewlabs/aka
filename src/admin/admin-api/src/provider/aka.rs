//! aka's own wiring: config, runtime home and the docker client.
//!
//! The API reads exactly what the CLI reads, so a dashboard row and a
//! `aka status` line can never disagree about where state came from.

use std::path::PathBuf;

use provider::Provider;
use tracing::info;

use aka_services::load_or_default;
use aka_services::paths::AkaPaths;

use crate::BoxedError;

pub fn init(provider: &mut Provider) -> Result<(), BoxedError> {
    // `AKA_CONFIG` names a config file explicitly (useful when this API runs
    // against a non-default aka setup, e.g. from a container); otherwise aka's
    // own discovery applies: `./.aka.toml` upward from the cwd, then
    // `~/.aka.toml`, then built-in defaults.
    let explicit = std::env::var("AKA_CONFIG")
        .ok()
        .filter(|path| !path.is_empty())
        .map(PathBuf::from);

    let start = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    let config = load_or_default(&start, explicit.as_deref())?;

    // `AKA_HOME` relocates the runtime dir (state.json, proxyd.pid, logs).
    let paths = AkaPaths::resolve()?;

    info!(
        message = "aka config resolved",
        dns = config.dns.container_name,
        proxy = config.proxy.container_name,
        network = config.proxy.network,
        home = %paths.home.display(),
    );

    provider.store(config);
    provider.store(paths);

    Ok(())
}
