//! Filesystem locations. Every path can be overridden through the environment so the
//! whole suite can run unprivileged in tests and nested in other systems.

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Paths {
    /// `R6_SERVICE_DIR`, default `/etc/r6/services`.
    pub service_dir: PathBuf,
    /// `R6_CONF`, default `/etc/r6/r6.toml`.
    pub global_conf: PathBuf,
    /// `R6_RUN_DIR`, default `/run/r6` (volatile state, control socket).
    pub run_dir: PathBuf,
    /// `R6_STATE_DIR`, default `/var/lib/r6` (persistent enable/disable overrides).
    pub state_dir: PathBuf,
    /// `R6_LOG_DIR` overrides `log_dir` from r6.toml.
    pub log_dir_override: Option<PathBuf>,
}

fn env_path(var: &str, default: &str) -> PathBuf {
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| default.into())
}

impl Paths {
    pub fn from_env() -> Paths {
        Paths {
            service_dir: env_path("R6_SERVICE_DIR", "/etc/r6/services"),
            global_conf: env_path("R6_CONF", "/etc/r6/r6.toml"),
            run_dir: env_path("R6_RUN_DIR", "/run/r6"),
            state_dir: env_path("R6_STATE_DIR", "/var/lib/r6"),
            log_dir_override: std::env::var_os("R6_LOG_DIR")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from),
        }
    }
}
