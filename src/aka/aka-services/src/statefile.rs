//! The `~/.aka/state.json` bridge between the proxy daemon and the CLI's
//! `status` / `routes` commands.

use aka_kernel::BoxedError;
use aka_kernel::state::DaemonState;
use std::path::Path;

pub fn now_label() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

pub fn write(path: &Path, state: &DaemonState) -> Result<(), BoxedError> {
    let json = serde_json::to_string_pretty(state)?;
    std::fs::write(path, json)?;
    Ok(())
}

pub fn read(path: &Path) -> Result<Option<DaemonState>, BoxedError> {
    match std::fs::read_to_string(path) {
        Ok(json) => Ok(Some(serde_json::from_str(&json)?)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_label_is_sane() {
        let label = now_label();
        assert!(label.ends_with('Z'), "{label}");
        let year: i64 = label[..4].parse().expect("year");
        assert!((2024..2100).contains(&year), "{label}");
    }

    #[test]
    fn round_trips_state() {
        let dir = std::env::temp_dir().join(format!("aka-state-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");

        let state = DaemonState {
            daemon_pid: 42,
            config_path: String::new(),
            updated_at: now_label(),
            proxy: aka_kernel::state::ProxyInfo {
                name: "aka_proxy".into(),
                ip: Some("172.20.0.2".into()),
                last_reload_ok: true,
            },
            table: Default::default(),
        };

        write(&path, &state).unwrap();
        let read = read(&path).unwrap().expect("state");
        assert_eq!(read.daemon_pid, 42);
        assert_eq!(read.proxy.ip.as_deref(), Some("172.20.0.2"));

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
