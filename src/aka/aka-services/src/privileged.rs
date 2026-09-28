//! Root-side operations behind the hidden `aka _privileged` verb, which
//! `aka up` / `aka down` shell out to. This code runs as root, so every
//! argument is validated here. `aka install-sudo-rule` installs the
//! sudoers drop-in that makes those calls passwordless — after that, aka
//! never asks for a password.

use std::io::Write;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use aka_kernel::BoxedError;

use crate::resolv::MARKER;

pub const SUDOERS_PATH: &str = "/etc/sudoers.d/com.freshbrewlabs.aka";

/// The sudoers drop-in content: passwordless root for `_privileged` only.
pub fn sudoers_text(user: &str, exe: &Path) -> String {
    format!(
        "# aka: passwordless host-resolver management (aka install-sudo-rule)\n\
         {user} ALL=(root) NOPASSWD: {} _privileged *\n",
        exe.display()
    )
}

/// Test hook so the root-side writes can be exercised without root.
fn resolver_dir() -> PathBuf {
    std::env::var_os("AKA_RESOLVER_DIR")
        .map_or_else(|| PathBuf::from("/etc/resolver"), PathBuf::from)
}

#[cfg(not(target_os = "macos"))]
fn resolv_conf() -> PathBuf {
    std::env::var_os("AKA_RESOLV_CONF")
        .map_or_else(|| PathBuf::from("/etc/resolv.conf"), PathBuf::from)
}

#[derive(Debug, Clone)]
pub struct Request {
    pub domains: Vec<String>,
    pub nameserver: IpAddr,
    pub port: u16,
}

/// Domains are argv from an unprivileged caller; only conservative DNS
/// label characters and the literal `#` wildcard marker survive.
fn check_domain(d: &str) -> Result<(), BoxedError> {
    let ok = d == "#"
        || (!d.is_empty()
            && d.len() <= 253
            && !d.starts_with('.')
            && !d.starts_with('-')
            && !d.contains("..")
            && d.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-'));
    if ok {
        Ok(())
    } else {
        Err(format!("invalid domain in privileged request: {d:?}").into())
    }
}

/// Where the resolver entries would live (for reporting; root does the
/// writing in `write`).
pub fn target_paths(domains: &[String]) -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        domains
            .iter()
            .filter(|d| d.as_str() != "#")
            .map(|d| resolver_dir().join(d))
            .collect()
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = domains;
        vec![resolv_conf()]
    }
}

pub fn write(req: &Request) -> Result<Vec<PathBuf>, BoxedError> {
    for d in &req.domains {
        check_domain(d)?;
    }

    #[cfg(target_os = "macos")]
    {
        std::fs::create_dir_all(resolver_dir()).ok();
        let mut written = Vec::new();
        for d in &req.domains {
            if d == "#" {
                // a catch-all wildcard cannot be expressed as a resolver file
                continue;
            }
            let path = resolver_dir().join(d);
            let contents = format!(
                "#{MARKER}\nnameserver {}\nport {}\n",
                req.nameserver, req.port
            );
            std::fs::write(&path, contents)?;
            written.push(path);
        }
        Ok(written)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let path = resolv_conf();
        let current = std::fs::read(&path).unwrap_or_default();
        let mut wanted = chunk(req);
        wanted.extend_from_slice(&strip_chunk(&current));
        std::fs::write(&path, &wanted)?;
        Ok(vec![path])
    }
}

pub fn clean(domains: &[String]) -> Result<(), BoxedError> {
    for d in domains {
        check_domain(d)?;
    }

    #[cfg(target_os = "macos")]
    {
        for d in domains {
            if d == "#" {
                continue;
            }
            let path = resolver_dir().join(d);
            // only remove files we ourselves wrote
            let ours = std::fs::read_to_string(&path)
                .map(|c| c.contains(MARKER))
                .unwrap_or(false);
            if ours {
                std::fs::remove_file(&path).ok();
            }
        }
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = domains;
        let path = resolv_conf();
        let Ok(current) = std::fs::read(&path) else {
            return Ok(());
        };
        if String::from_utf8_lossy(&current).contains(MARKER) {
            std::fs::write(&path, strip_chunk(&current))?;
        }
        Ok(())
    }
}

/// `sudo [-n] <args...>`, stdin optional; stdout/stderr inherited only on
/// the interactive attempt so a password prompt can reach the terminal.
fn run_sudo(args: &[&str], stdin: Option<&[u8]>) -> Result<(), BoxedError> {
    for non_interactive in [true, false] {
        let mut cmd = Command::new("sudo");
        if non_interactive {
            cmd.arg("-n");
            cmd.stdout(Stdio::null()).stderr(Stdio::null());
        }
        cmd.args(args);
        if stdin.is_some() {
            cmd.stdin(Stdio::piped());
        }

        let mut child = match cmd.spawn() {
            Ok(child) => child,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Err("sudo is not available; run aka commands as root".into());
            }
            Err(err) => return Err(err.into()),
        };
        if let Some(contents) = stdin {
            child
                .stdin
                .as_mut()
                .expect("stdin piped")
                .write_all(contents)?;
        }
        if child.wait()?.success() {
            return Ok(());
        }
    }
    Err("privileged command failed; run `aka install-sudo-rule` once to make aka sudo-free".into())
}

/// Run one of aka's own `_privileged` verbs as root (passwordless with
/// the sudoers rule installed, prompted otherwise).
pub fn run_privileged(args: &[String]) -> Result<(), BoxedError> {
    let exe = std::env::current_exe()?;

    for non_interactive in [true, false] {
        let mut cmd = Command::new("sudo");
        if non_interactive {
            cmd.arg("-n");
            cmd.stdout(Stdio::null()).stderr(Stdio::null());
        }
        cmd.arg(&exe);
        cmd.args(args);
        if cmd.spawn()?.wait()?.success() {
            return Ok(());
        }
        if !non_interactive {
            return Err(
                "privileged operation failed; run `aka install-sudo-rule` once so aka never asks for a password".into(),
            );
        }
    }
    unreachable!()
}

/// One-time: install the passwordless sudoers rule (this itself uses sudo).
pub fn install_sudo_rule() -> Result<(), BoxedError> {
    let exe = std::env::current_exe()?.canonicalize()?;
    let user = std::env::var("USER").map_err(|_| "USER is unset; run from a login shell")?;
    #[cfg(target_os = "macos")]
    let group = "wheel";
    #[cfg(not(target_os = "macos"))]
    let group = "root";

    run_sudo(
        &["tee", SUDOERS_PATH],
        Some(sudoers_text(&user, &exe).as_bytes()),
    )?;
    run_sudo(&["chmod", "440", SUDOERS_PATH], None)?;
    run_sudo(&["chown", &format!("root:{group}"), SUDOERS_PATH], None)?;

    match run_sudo(&["visudo", "-cf", SUDOERS_PATH], None) {
        Ok(()) => println!("installed {SUDOERS_PATH}: aka up/down are now sudo-free"),
        Err(err) => {
            run_sudo(&["rm", "-f", SUDOERS_PATH], None).ok();
            return Err(format!("sudoers rule rejected by visudo and removed: {err}").into());
        }
    }
    Ok(())
}

// -------------------------------------------------------------------- linux

#[cfg(not(target_os = "macos"))]
fn chunk(req: &Request) -> Vec<u8> {
    format!(
        "# begin {MARKER}\nnameserver {}\n# end {MARKER}\n",
        req.nameserver
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
mod tests {
    use super::*;

    fn req() -> Request {
        Request {
            domains: vec!["docker".into(), "test".into()],
            nameserver: "127.0.0.1".parse().unwrap(),
            port: 53,
        }
    }

    #[test]
    fn sudoers_rule_scoped_to_privileged_verb() {
        let text = sudoers_text("alice", Path::new("/Users/alice/.cargo/bin/aka"));
        assert!(
            text.contains("alice ALL=(root) NOPASSWD: /Users/alice/.cargo/bin/aka _privileged *")
        );
        assert_eq!(text.lines().count(), 2, "single rule + comment");
    }

    #[test]
    fn domain_validation() {
        for good in ["docker", "test", "my-app.test", "localhost", "#"] {
            assert!(check_domain(good).is_ok(), "{good}");
        }
        for bad in [
            "",
            "#etc/passwd",
            "../evil",
            ".hidden",
            "-x",
            "a..b",
            "a b",
            "a/b",
            "café",
        ] {
            assert!(check_domain(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn privileged_write_writes_only_validated_targets() {
        let dir = std::env::temp_dir().join(format!("aka-resolver-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        // SAFETY: this is the only test touching AKA_RESOLVER_DIR
        unsafe { std::env::set_var("AKA_RESOLVER_DIR", &dir) };
        let written = write(&req()).unwrap();
        if cfg!(target_os = "macos") {
            assert_eq!(written.len(), 2);
            let docker = std::fs::read_to_string(dir.join("docker")).unwrap();
            assert!(docker.contains("nameserver 127.0.0.1") && docker.contains("port 53"));
            // clean removes ours and leaves foreign files
            std::fs::write(dir.join("test"), "do not touch").unwrap();
            clean(&["docker".into(), "test".into(), "missing".into()]).unwrap();
            assert!(!dir.join("docker").exists());
            assert_eq!(
                std::fs::read_to_string(dir.join("test")).unwrap(),
                "do not touch"
            );
        }
        unsafe { std::env::remove_var("AKA_RESOLVER_DIR") };
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn privileged_write_rejects_injected_domain() {
        assert!(
            write(&Request {
                domains: vec!["../../x".into()],
                ..req()
            })
            .is_err()
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn chunk_round_trip() {
        let chunked = format!(
            "# other\nnameserver 1.1.1.1\n{}\nmine",
            String::from_utf8_lossy(&chunk(&req()))
        );
        let stripped = String::from_utf8(strip_chunk(chunked.as_bytes())).unwrap();
        assert!(!stripped.contains("nameserver 127.0.0.1"));
        assert!(stripped.contains("nameserver 1.1.1.1"));
        assert!(stripped.contains("mine"));
    }
}
