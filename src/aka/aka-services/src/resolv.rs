//! Host resolver configuration (public entry point). macOS: one
//! `/etc/resolver/<domain>` file per served domain (dory does the same);
//! Linux: a managed chunk inside `/etc/resolv.conf`. The privileged work
//! is done by the hidden `aka _privileged` verb via `sudo -n` — password-
//! free once `aka install-sudo-rule` has run; see `crate::privileged`.

use std::path::PathBuf;

use aka_kernel::BoxedError;
use aka_kernel::config::AkaConfig;
use tracing::info;

use crate::privileged;

pub const MARKER: &str = "added by aka";

pub fn configure(cfg: &AkaConfig) -> Result<Vec<PathBuf>, BoxedError> {
    if cfg.dns.domains.is_empty() {
        return Ok(Vec::new());
    }

    let mut args = vec![
        "_privileged".to_string(),
        "write".into(),
        "--nameserver".into(),
        cfg.resolv.nameserver.clone(),
        "--port".into(),
        port(cfg).to_string(),
    ];
    for domain in &cfg.dns.domains {
        args.push("--domain".into());
        args.push(domain.domain.clone());
    }

    privileged::run_privileged(&args)?;
    let paths = privileged::target_paths(&domain_names(cfg));
    for path in &paths {
        info!("wrote {}", path.display());
    }
    Ok(paths)
}

pub fn clean(cfg: &AkaConfig) -> Result<(), BoxedError> {
    if cfg.dns.domains.is_empty() {
        return Ok(());
    }

    let mut args = vec!["_privileged".to_string(), "clean".into()];
    for domain in &cfg.dns.domains {
        args.push("--domain".into());
        args.push(domain.domain.clone());
    }
    privileged::run_privileged(&args)?;
    info!("removed aka resolver entries");
    Ok(())
}

fn domain_names(cfg: &AkaConfig) -> Vec<String> {
    cfg.dns.domains.iter().map(|d| d.domain.clone()).collect()
}

fn port(cfg: &AkaConfig) -> u16 {
    cfg.resolv.port.unwrap_or(cfg.dns.port)
}
