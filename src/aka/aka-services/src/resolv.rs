//! Host resolver configuration. macOS: one `/etc/resolver/<domain>` file per
//! served domain (dory does the same). Linux: a managed chunk inside
//! `/etc/resolv.conf`. Writes go through `sudo tee`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use aka_kernel::BoxedError;
use aka_kernel::config::AkaConfig;
use tracing::info;

pub const MARKER: &str = "added by aka";

pub fn configure(cfg: &AkaConfig) -> Result<Vec<PathBuf>, BoxedError> {
    if cfg.dns.domains.is_empty() {
        return Ok(Vec::new());
    }

    #[cfg(target_os = "macos")]
    {
        configure_macos(cfg)
    }
    #[cfg(not(target_os = "macos"))]
    {
        configure_linux(cfg)
    }
}

pub fn clean(cfg: &AkaConfig) -> Result<(), BoxedError> {
    #[cfg(target_os = "macos")]
    {
        clean_macos(cfg)
    }
    #[cfg(not(target_os = "macos"))]
    {
        clean_linux(cfg)
    }
}

fn port(cfg: &AkaConfig) -> u16 {
    cfg.resolv.port.unwrap_or(cfg.dns.port)
}

/// `sudo tee <path>` with `contents` on stdin.
pub fn sudo_tee(path: &Path, contents: &[u8]) -> Result<(), BoxedError> {
    let mut child = Command::new("sudo")
        .arg("tee")
        .arg(path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()?;

    child
        .stdin
        .as_mut()
        .expect("stdin piped")
        .write_all(contents)?;

    let status = child.wait()?;
    if !status.success() {
        return Err(format!("sudo tee {} failed ({status})", path.display()).into());
    }
    Ok(())
}

// ------------------------------------------------------------------- macOS

#[cfg(target_os = "macos")]
const RESOLVER_DIR: &str = "/etc/resolver";

#[cfg(target_os = "macos")]
fn configure_macos(cfg: &AkaConfig) -> Result<Vec<PathBuf>, BoxedError> {
    std::fs::create_dir_all(RESOLVER_DIR).ok();

    let mut written = Vec::new();
    for domain in &cfg.dns.domains {
        if domain.domain == "#" {
            // a catch-all wildcard cannot be expressed as a resolver file
            continue;
        }
        let path = PathBuf::from(RESOLVER_DIR).join(&domain.domain);
        let contents = format!(
            "#{MARKER}\nnameserver {}\nport {}\n",
            cfg.resolv.nameserver,
            port(cfg)
        );
        sudo_tee(&path, contents.as_bytes())?;
        info!("wrote {}", path.display());
        written.push(path);
    }
    Ok(written)
}

#[cfg(target_os = "macos")]
fn clean_macos(cfg: &AkaConfig) -> Result<(), BoxedError> {
    for domain in &cfg.dns.domains {
        let path = PathBuf::from(RESOLVER_DIR).join(&domain.domain);
        // /etc/resolver files are world-readable
        let ours = std::fs::read_to_string(&path)
            .map(|c| c.contains(MARKER))
            .unwrap_or(false);
        if ours {
            Command::new("sudo")
                .arg("rm")
                .arg("-f")
                .arg(&path)
                .status()?;
            info!("removed {}", path.display());
        }
    }
    Ok(())
}

// -------------------------------------------------------------------- linux

#[cfg(not(target_os = "macos"))]
fn configure_linux(cfg: &AkaConfig) -> Result<Vec<PathBuf>, BoxedError> {
    const RESOLV_CONF: &str = "/etc/resolv.conf";

    let current = std::fs::read(RESOLV_CONF).unwrap_or_default();
    let mut wanted = chunk(cfg);
    wanted.extend_from_slice(&strip_chunk(&current));

    sudo_tee(Path::new(RESOLV_CONF), &wanted)?;
    info!("updated {RESOLV_CONF}");
    Ok(vec![PathBuf::from(RESOLV_CONF)])
}

#[cfg(not(target_os = "macos"))]
fn clean_linux(_cfg: &AkaConfig) -> Result<(), BoxedError> {
    const RESOLV_CONF: &str = "/etc/resolv.conf";

    let Ok(current) = std::fs::read(RESOLV_CONF) else {
        return Ok(());
    };
    if !String::from_utf8_lossy(&current).contains(MARKER) {
        return Ok(());
    }
    sudo_tee(Path::new(RESOLV_CONF), &strip_chunk(&current))?;
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn chunk(cfg: &AkaConfig) -> Vec<u8> {
    format!(
        "# begin {MARKER}\nnameserver {}\n# end {MARKER}\n",
        cfg.resolv.nameserver
    )
    .into_bytes()
}

#[cfg(not(target_os = "macos"))]
fn strip_chunk(contents: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(contents).into_owned();
    let mut out = String::new();
    let mut inside = false;

    for line in text.lines() {
        if line.contains(&format!("# begin {MARKER}")) {
            inside = true;
            continue;
        }
        if line.contains(&format!("# end {MARKER}")) {
            inside = false;
            continue;
        }
        if !inside {
            out.push_str(line);
            out.push('\n');
        }
    }

    out.into_bytes()
}

#[cfg(test)]
#[cfg(not(target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn strip_chunk_removes_only_our_block() {
        let input =
            b"# begin added by aka\nnameserver 127.0.0.1\n# end added by aka\nnameserver 8.8.8.8\n";
        assert_eq!(strip_chunk(input), b"nameserver 8.8.8.8\n");
    }

    #[test]
    fn strip_chunk_idempotent() {
        let input = b"nameserver 1.1.1.1\n";
        assert_eq!(strip_chunk(input), input);
    }
}
