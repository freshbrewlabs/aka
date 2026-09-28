use std::sync::Arc;

use provider::Provider;
use tracing::info;

use admin_services::{GetStatusService, GetStatusUseCase};
use aka_docker::AkaDocker;
use aka_kernel::config::AkaConfig;
use aka_services::paths::AkaPaths;

use crate::BoxedError;

pub fn init(provider: &mut Provider) -> Result<(), BoxedError> {
    info!("setting up provider");

    // The docker client has exactly one consumer (the status use case), which
    // owns it; nothing else in this stack talks to the engine.
    let docker = AkaDocker::connect()?;

    let service = Arc::new(GetStatusUseCase::new(
        provider.fetch::<AkaConfig>()?.clone(),
        provider.fetch::<AkaPaths>()?.clone(),
        docker,
    ));

    provider.store::<GetStatusService>(service);

    Ok(())
}
