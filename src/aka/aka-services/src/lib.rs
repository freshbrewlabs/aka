//! aka control-plane: config, route discovery, angie/dnsmasq rendering,
//! resolver configuration, container lifecycle and the proxy daemon.

pub mod angie;
pub mod certs;
pub mod config;
pub mod conflicts;
pub mod discover;
pub mod dnsmasq;
pub mod lifecycle;
pub mod paths;
pub mod proxyd;
pub mod resolv;
pub mod statefile;

pub use config::{load_config, load_or_default, write_default_config};
pub use paths::AkaPaths;
