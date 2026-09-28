use std::path::PathBuf;

use aka_kernel::BoxedError;

/// Runtime home for aka: `~/.aka` (override with `AKA_HOME`, used by tests
/// and the dev loop).
#[derive(Debug, Clone)]
pub struct AkaPaths {
    pub home: PathBuf,
}

impl AkaPaths {
    pub fn resolve() -> Result<Self, BoxedError> {
        if let Ok(home) = std::env::var("AKA_HOME") {
            return Ok(Self {
                home: PathBuf::from(home),
            });
        }

        let home = std::env::var("HOME")
            .map(PathBuf::from)
            .ok()
            .ok_or("HOME is not set and AKA_HOME is unset")?;
        Ok(Self {
            home: home.join(".aka"),
        })
    }

    /// Host directory rendered http configs live in (mounted into the proxy).
    pub fn http_dir(&self) -> PathBuf {
        self.home.join("proxy.d").join("http")
    }

    /// Host directory rendered stream configs live in.
    pub fn stream_dir(&self) -> PathBuf {
        self.home.join("proxy.d").join("stream")
    }

    pub fn state_file(&self) -> PathBuf {
        self.home.join("state.json")
    }

    pub fn pid_file(&self) -> PathBuf {
        self.home.join("proxyd.pid")
    }

    pub fn log_file(&self) -> PathBuf {
        self.home.join("proxyd.log")
    }

    pub fn default_config_path(&self) -> PathBuf {
        self.home.join("aka.yml")
    }

    pub fn ensure_dirs(&self) -> Result<(), BoxedError> {
        std::fs::create_dir_all(&self.home)?;
        std::fs::create_dir_all(self.http_dir())?;
        std::fs::create_dir_all(self.stream_dir())?;
        Ok(())
    }
}

/// Default config file location: `~/.aka/aka.yml` (aka's `~/.dory.yml`).
pub fn default_config_path() -> PathBuf {
    AkaPaths::resolve()
        .map(|p| p.default_config_path())
        .unwrap_or_else(|_| PathBuf::from(".aka.yml"))
}
