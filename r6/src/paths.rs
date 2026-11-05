//! Filesystem locations. Every path can be overridden through the environment so the
//! whole suite can run unprivileged in tests and nested in other systems.

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Paths {
    /// `R6_SERVICE_DIR`, default `/etc/r6/services`. May list several directories
    /// separated by `:`; later ones override earlier ones. `service_dir` is the first.
    pub service_dir: PathBuf,
    pub service_dirs: Vec<PathBuf>,
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
        let dirs: Vec<PathBuf> = std::env::var("R6_SERVICE_DIR")
            .ok()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "/etc/r6/services".into())
            .split(':')
            .filter(|d| !d.is_empty())
            .map(PathBuf::from)
            .collect();
        Paths {
            service_dir: dirs
                .first()
                .cloned()
                .unwrap_or_else(|| "/etc/r6/services".into()),
            service_dirs: dirs,
            global_conf: env_path("R6_CONF", "/etc/r6/r6.toml"),
            run_dir: env_path("R6_RUN_DIR", "/run/r6"),
            state_dir: env_path("R6_STATE_DIR", "/var/lib/r6"),
            log_dir_override: std::env::var_os("R6_LOG_DIR")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from),
        }
    }
}

/// Locations for a per-user supervisor (`r6-user`, `r6 --user`). Explicit `R6_*`
/// variables still win, so tests and nested sessions can relocate everything.
pub fn user_paths() -> Result<Paths, String> {
    let runtime = std::env::var("XDG_RUNTIME_DIR")
        .ok()
        .filter(|v| !v.is_empty())
        .ok_or("XDG_RUNTIME_DIR is not set (no user session)")?;
    let home = std::env::var("HOME").map_err(|_| "HOME is not set")?;
    let config = std::env::var("XDG_CONFIG_HOME").unwrap_or_else(|_| format!("{home}/.config"));
    let state = std::env::var("XDG_STATE_HOME").unwrap_or_else(|_| format!("{home}/.local/state"));
    let set_default = |var: &str, val: String| {
        if std::env::var_os(var).filter(|v| !v.is_empty()).is_none() {
            // SAFETY: called before any threads are started.
            std::env::set_var(var, val);
        }
    };
    set_default(
        "R6_SERVICE_DIR",
        format!("/etc/r6/user-services:{config}/r6/services"),
    );
    set_default("R6_RUN_DIR", format!("{runtime}/r6"));
    set_default("R6_STATE_DIR", format!("{state}/r6"));
    set_default("R6_LOG_DIR", format!("{runtime}/r6/log"));
    set_default("R6_CONF", format!("{config}/r6/r6.toml"));
    Ok(Paths::from_env())
}
