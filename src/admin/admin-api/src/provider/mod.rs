pub mod aka;
pub mod services;

use provider::Provider;

use crate::BoxedError;

pub async fn init() -> Result<Provider, BoxedError> {
    let mut provider = Provider::new();

    aka::init(&mut provider)?;
    services::init(&mut provider)?;

    Ok(provider)
}
