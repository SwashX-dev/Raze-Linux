//! Service definitions and global configuration.
//!
//! A service is a TOML file `<name>.toml` in the service directory
//! (default `/etc/r6/services`). Every field except `command` is optional.
//!
//! ```toml
//! description = "DHCP client"
//! command = ["/bin/udhcpc", "-f", "-i", "eth0"]
//! type = "longrun"          # or "oneshot"
//! depends = ["udev"]        # hard dependency: started first, failure propagates
//! wants = ["syslog"]        # soft dependency: ordered first if enabled, failure ignored
//! ready = "immediate"       # "immediate" | "notify" (write "\n" to $R6_NOTIFY_FD)
//! restart = "on-failure"    # "always" | "on-failure" | "never"
//! enable = true             # start at boot
//! log = "file"              # "file" | "console" | "null"
//! ```

use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Longrun,
    Oneshot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Restart {
    Always,
    OnFailure,
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Ready {
    /// Ready as soon as exec() succeeds.
    Immediate,
    /// Ready when the service writes a newline to the fd named by `R6_NOTIFY_FD`
    /// (s6-style readiness notification).
    Notify,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogTarget {
    File,
    Console,
    Null,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawService {
    description: Option<String>,
    command: Vec<String>,
    #[serde(rename = "type")]
    kind: Option<Kind>,
    #[serde(default)]
    depends: Vec<String>,
    #[serde(default)]
    wants: Vec<String>,
    ready: Option<Ready>,
    ready_timeout_ms: Option<u64>,
    restart: Option<Restart>,
    restart_delay_ms: Option<u64>,
    restart_max: Option<u32>,
    restart_window_s: Option<u64>,
    stop_signal: Option<String>,
    stop_timeout_ms: Option<u64>,
    user: Option<String>,
    group: Option<String>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    working_directory: Option<PathBuf>,
    log: Option<LogTarget>,
    enable: Option<bool>,
    umask: Option<String>,
    rlimit_nofile: Option<u64>,
    oom_score_adjust: Option<i32>,
    #[serde(default)]
    capabilities: Vec<String>,
    runtime_directory: Option<String>,
    state_directory: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Service {
    pub name: String,
    pub description: String,
    pub command: Vec<String>,
    pub kind: Kind,
    pub depends: Vec<String>,
    pub wants: Vec<String>,
    pub ready: Ready,
    pub ready_timeout_ms: u64,
    pub restart: Restart,
    pub restart_delay_ms: u64,
    pub restart_max: u32,
    pub restart_window_s: u64,
    pub stop_signal: i32,
    pub stop_timeout_ms: u64,
    pub user: Option<String>,
    pub group: Option<String>,
    pub environment: BTreeMap<String, String>,
    pub working_directory: Option<PathBuf>,
    pub log: LogTarget,
    pub enable: bool,
    /// File mode creation mask (octal in the TOML: `umask = "027"`).
    pub umask: Option<u32>,
    /// RLIMIT_NOFILE soft and hard limit for the service.
    pub rlimit_nofile: Option<u64>,
    /// Value written to /proc/self/oom_score_adj (-1000..1000).
    pub oom_score_adjust: Option<i32>,
    /// Ambient capabilities kept after dropping to `user` (e.g. CAP_NET_ADMIN).
    pub capabilities: Vec<u32>,
    /// Created as /run/<name> (owned by the service user) before each start.
    pub runtime_directory: Option<String>,
    /// Created as /var/lib/<name> (owned by the service user) if missing.
    pub state_directory: Option<String>,
}

#[derive(Debug)]
pub enum ConfigError {
    Io(PathBuf, std::io::Error),
    Parse(PathBuf, String),
    Invalid(String, String),
    UnknownDependency { service: String, dependency: String },
    Cycle(Vec<String>),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Io(p, e) => write!(f, "{}: {}", p.display(), e),
            ConfigError::Parse(p, e) => write!(f, "{}: parse error: {}", p.display(), e),
            ConfigError::Invalid(s, e) => write!(f, "service '{s}': {e}"),
            ConfigError::UnknownDependency {
                service,
                dependency,
            } => {
                write!(
                    f,
                    "service '{service}' depends on unknown service '{dependency}'"
                )
            }
            ConfigError::Cycle(c) => write!(f, "dependency cycle: {}", c.join(" -> ")),
        }
    }
}

impl std::error::Error for ConfigError {}

pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b'@')
        && !name.starts_with('.')
}

/// Linux capability numbers (include/uapi/linux/capability.h).
pub fn capability(name: &str) -> Option<u32> {
    let n = name.trim().to_ascii_uppercase();
    let n = n.strip_prefix("CAP_").unwrap_or(&n);
    Some(match n {
        "CHOWN" => 0,
        "DAC_OVERRIDE" => 1,
        "DAC_READ_SEARCH" => 2,
        "FOWNER" => 3,
        "FSETID" => 4,
        "KILL" => 5,
        "SETGID" => 6,
        "SETUID" => 7,
        "SETPCAP" => 8,
        "LINUX_IMMUTABLE" => 9,
        "NET_BIND_SERVICE" => 10,
        "NET_BROADCAST" => 11,
        "NET_ADMIN" => 12,
        "NET_RAW" => 13,
        "IPC_LOCK" => 14,
        "IPC_OWNER" => 15,
        "SYS_MODULE" => 16,
        "SYS_RAWIO" => 17,
        "SYS_CHROOT" => 18,
        "SYS_PTRACE" => 19,
        "SYS_PACCT" => 20,
        "SYS_ADMIN" => 21,
        "SYS_BOOT" => 22,
        "SYS_NICE" => 23,
        "SYS_RESOURCE" => 24,
        "SYS_TIME" => 25,
        "SYS_TTY_CONFIG" => 26,
        "MKNOD" => 27,
        "LEASE" => 28,
        "AUDIT_WRITE" => 29,
        "AUDIT_CONTROL" => 30,
        "SETFCAP" => 31,
        "MAC_OVERRIDE" => 32,
        "MAC_ADMIN" => 33,
        "SYSLOG" => 34,
        "WAKE_ALARM" => 35,
        "BLOCK_SUSPEND" => 36,
        "AUDIT_READ" => 37,
        "PERFMON" => 38,
        "BPF" => 39,
        "CHECKPOINT_RESTORE" => 40,
        _ => return None,
    })
}

pub fn parse_signal(s: &str) -> Option<i32> {
    let s = s.trim().trim_start_matches("SIG");
    Some(match s {
        "TERM" => libc::SIGTERM,
        "INT" => libc::SIGINT,
        "HUP" => libc::SIGHUP,
        "QUIT" => libc::SIGQUIT,
        "KILL" => libc::SIGKILL,
        "USR1" => libc::SIGUSR1,
        "USR2" => libc::SIGUSR2,
        _ => return s.parse().ok().filter(|n| *n > 0 && *n < 65),
    })
}

impl Service {
    pub fn parse(name: &str, text: &str, path: &Path) -> Result<Service, ConfigError> {
        if !valid_name(name) {
            return Err(ConfigError::Invalid(
                name.into(),
                "invalid service name".into(),
            ));
        }
        let raw: RawService =
            toml::from_str(text).map_err(|e| ConfigError::Parse(path.into(), e.to_string()))?;
        let inv = |msg: &str| ConfigError::Invalid(name.into(), msg.into());
        if raw.command.is_empty() || raw.command[0].is_empty() {
            return Err(inv("'command' must be a non-empty array"));
        }
        if !raw.command[0].starts_with('/') {
            return Err(inv("command[0] must be an absolute path"));
        }
        if raw.command.iter().any(|a| a.contains('\0')) {
            return Err(inv("command contains NUL byte"));
        }
        for d in raw.depends.iter().chain(raw.wants.iter()) {
            if !valid_name(d) {
                return Err(inv(&format!("invalid dependency name '{d}'")));
            }
            if d == name {
                return Err(inv("service depends on itself"));
            }
        }
        for k in raw.environment.keys() {
            if k.is_empty() || k.contains('=') || k.contains('\0') {
                return Err(inv(&format!("invalid environment variable name '{k}'")));
            }
        }
        let stop_signal = match raw.stop_signal.as_deref() {
            None => libc::SIGTERM,
            Some(s) => parse_signal(s).ok_or_else(|| inv(&format!("unknown stop_signal '{s}'")))?,
        };
        let kind = raw.kind.unwrap_or(Kind::Longrun);
        let restart = raw.restart.unwrap_or(match kind {
            Kind::Longrun => Restart::OnFailure,
            Kind::Oneshot => Restart::Never,
        });
        let ready = raw.ready.unwrap_or(Ready::Immediate);
        if kind == Kind::Oneshot && ready == Ready::Notify {
            return Err(inv(
                "oneshot services become ready on successful exit; 'ready = notify' is not allowed",
            ));
        }
        if kind == Kind::Oneshot && restart == Restart::Always {
            return Err(inv("oneshot services cannot use 'restart = always'"));
        }
        let umask = match raw.umask.as_deref() {
            None => None,
            Some(u) => Some(
                u32::from_str_radix(u.trim_start_matches("0o"), 8)
                    .ok()
                    .filter(|m| *m <= 0o777)
                    .ok_or_else(|| inv(&format!("invalid umask '{u}' (octal, e.g. \"022\")")))?,
            ),
        };
        if let Some(adj) = raw.oom_score_adjust {
            if !(-1000..=1000).contains(&adj) {
                return Err(inv("oom_score_adjust must be between -1000 and 1000"));
            }
        }
        let mut capabilities = Vec::new();
        for c in &raw.capabilities {
            capabilities
                .push(capability(c).ok_or_else(|| inv(&format!("unknown capability '{c}'")))?);
        }
        if !capabilities.is_empty() && raw.user.is_none() {
            return Err(inv("'capabilities' only applies to services with a 'user'"));
        }
        for d in raw
            .runtime_directory
            .iter()
            .chain(raw.state_directory.iter())
        {
            if d.is_empty() || d.starts_with('/') || d.split('/').any(|c| c == ".." || c.is_empty())
            {
                return Err(inv(&format!(
                    "directory '{d}' must be a relative name like \"nscd\""
                )));
            }
        }
        let restart_max = raw.restart_max.unwrap_or(5);
        let restart_window_s = raw.restart_window_s.unwrap_or(60);
        if restart_window_s == 0 {
            return Err(inv("restart_window_s must be > 0"));
        }
        Ok(Service {
            name: name.into(),
            description: raw.description.unwrap_or_default(),
            command: raw.command,
            kind,
            depends: dedup(raw.depends),
            wants: dedup(raw.wants),
            ready,
            ready_timeout_ms: raw.ready_timeout_ms.unwrap_or(30_000),
            restart,
            restart_delay_ms: raw.restart_delay_ms.unwrap_or(250),
            restart_max,
            restart_window_s,
            stop_signal,
            stop_timeout_ms: raw.stop_timeout_ms.unwrap_or(5_000),
            user: raw.user,
            group: raw.group,
            environment: raw.environment,
            working_directory: raw.working_directory,
            log: raw.log.unwrap_or(LogTarget::File),
            enable: raw.enable.unwrap_or(false),
            umask,
            rlimit_nofile: raw.rlimit_nofile,
            oom_score_adjust: raw.oom_score_adjust,
            capabilities,
            runtime_directory: raw.runtime_directory,
            state_directory: raw.state_directory,
        })
    }
}

fn dedup(v: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    v.into_iter().filter(|x| seen.insert(x.clone())).collect()
}

/// A fully loaded and validated set of services.
#[derive(Debug, Clone, Default)]
pub struct ServiceSet {
    pub services: BTreeMap<String, Service>,
}

impl ServiceSet {
    /// Load every `*.toml` file in `dir`. Invalid files fail the whole load so that a
    /// broken configuration is detected before anything is started.
    pub fn load_dir(dir: &Path) -> Result<ServiceSet, ConfigError> {
        Self::load_dirs(&[dir.to_path_buf()])
    }

    /// Load several service directories in order; a service defined in a later
    /// directory replaces one of the same name from an earlier directory (for example
    /// `/etc/r6/user-services` overlaid by `~/.config/r6/services`).
    pub fn load_dirs(dirs: &[PathBuf]) -> Result<ServiceSet, ConfigError> {
        let mut services = BTreeMap::new();
        for dir in dirs {
            Self::load_into(dir, &mut services)?;
        }
        let set = ServiceSet { services };
        set.validate()?;
        Ok(set)
    }

    fn load_into(dir: &Path, services: &mut BTreeMap<String, Service>) -> Result<(), ConfigError> {
        let rd = match std::fs::read_dir(dir) {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(ConfigError::Io(dir.into(), e)),
        };
        let mut paths: Vec<PathBuf> = rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().map(|x| x == "toml").unwrap_or(false))
            .collect();
        paths.sort();
        for path in paths {
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            let text =
                std::fs::read_to_string(&path).map_err(|e| ConfigError::Io(path.clone(), e))?;
            let svc = Service::parse(&name, &text, &path)?;
            services.insert(name, svc);
        }
        Ok(())
    }

    pub fn from_services(list: Vec<Service>) -> Result<ServiceSet, ConfigError> {
        let set = ServiceSet {
            services: list.into_iter().map(|s| (s.name.clone(), s)).collect(),
        };
        set.validate()?;
        Ok(set)
    }

    /// Check that hard dependencies exist and the graph is acyclic.
    /// Soft dependencies (`wants`) on missing services are ignored.
    pub fn validate(&self) -> Result<(), ConfigError> {
        for s in self.services.values() {
            for d in &s.depends {
                if !self.services.contains_key(d) {
                    return Err(ConfigError::UnknownDependency {
                        service: s.name.clone(),
                        dependency: d.clone(),
                    });
                }
            }
        }
        crate::activation::Graph::new(self).topo_order().map(|_| ())
    }

    /// Ordering edges of a service: hard deps plus soft deps that exist.
    pub fn order_deps<'a>(&'a self, s: &'a Service) -> impl Iterator<Item = &'a String> + 'a {
        s.depends
            .iter()
            .chain(s.wants.iter().filter(|w| self.services.contains_key(*w)))
    }
}

/// Global settings from `/etc/r6/r6.toml` (all optional).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Global {
    pub log_dir: PathBuf,
    pub log_max_bytes: u64,
    pub log_keep: u32,
    pub shutdown_timeout_ms: u64,
}

impl Default for Global {
    fn default() -> Self {
        Global {
            log_dir: PathBuf::from("/run/r6/log"),
            log_max_bytes: 512 * 1024,
            log_keep: 3,
            shutdown_timeout_ms: 15_000,
        }
    }
}

impl Global {
    pub fn load(path: &Path) -> Result<Global, ConfigError> {
        match std::fs::read_to_string(path) {
            Ok(t) => toml::from_str(&t).map_err(|e| ConfigError::Parse(path.into(), e.to_string())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Global::default()),
            Err(e) => Err(ConfigError::Io(path.into(), e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(name: &str, t: &str) -> Result<Service, ConfigError> {
        Service::parse(name, t, Path::new("test.toml"))
    }

    #[test]
    fn minimal_service_defaults() {
        let s = p("a", r#"command = ["/bin/true"]"#).unwrap();
        assert_eq!(s.kind, Kind::Longrun);
        assert_eq!(s.restart, Restart::OnFailure);
        assert_eq!(s.ready, Ready::Immediate);
        assert_eq!(s.stop_signal, libc::SIGTERM);
        assert!(!s.enable);
    }

    #[test]
    fn rejects_relative_command_and_unknown_fields() {
        assert!(p("a", r#"command = ["true"]"#).is_err());
        assert!(p("a", r#"command = []"#).is_err());
        assert!(p("a", "command = [\"/bin/true\"]\nbogus = 1").is_err());
        assert!(p("bad/name", r#"command = ["/bin/true"]"#).is_err());
        assert!(p("a", "command = [\"/bin/true\"]\ndepends = [\"a\"]").is_err());
    }

    #[test]
    fn oneshot_constraints() {
        assert!(p(
            "a",
            "command = [\"/bin/true\"]\ntype = \"oneshot\"\nready = \"notify\""
        )
        .is_err());
        assert!(p(
            "a",
            "command = [\"/bin/true\"]\ntype = \"oneshot\"\nrestart = \"always\""
        )
        .is_err());
        let s = p("a", "command = [\"/bin/true\"]\ntype = \"oneshot\"").unwrap();
        assert_eq!(s.restart, Restart::Never);
    }

    #[test]
    fn umask_rlimit_oom() {
        let s = p("a", "command = [\"/bin/true\"]\numask = \"027\"\nrlimit_nofile = 65536\noom_score_adjust = -500").unwrap();
        assert_eq!(s.umask, Some(0o027));
        assert_eq!(s.rlimit_nofile, Some(65536));
        assert_eq!(s.oom_score_adjust, Some(-500));
        assert!(p("a", "command = [\"/bin/true\"]\numask = \"999\"").is_err());
        assert!(p("a", "command = [\"/bin/true\"]\noom_score_adjust = 2000").is_err());
    }

    #[test]
    fn capabilities_and_directories() {
        let s = p("a", "command = [\"/bin/true\"]\nuser = \"nobody\"\ncapabilities = [\"CAP_NET_ADMIN\", \"net_raw\"]\nruntime_directory = \"a\"\nstate_directory = \"a/b\"").unwrap();
        assert_eq!(s.capabilities, vec![12, 13]);
        assert!(p(
            "a",
            "command = [\"/bin/true\"]\nuser = \"nobody\"\ncapabilities = [\"CAP_NOPE\"]"
        )
        .is_err());
        assert!(p(
            "a",
            "command = [\"/bin/true\"]\ncapabilities = [\"CAP_NET_ADMIN\"]"
        )
        .is_err());
        assert!(p(
            "a",
            "command = [\"/bin/true\"]\nruntime_directory = \"../etc\""
        )
        .is_err());
        assert!(p(
            "a",
            "command = [\"/bin/true\"]\nstate_directory = \"/var/lib/x\""
        )
        .is_err());
    }

    #[test]
    fn later_directories_override_earlier_ones() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        std::fs::write(a.path().join("x.toml"), "command = [\"/bin/a\"]").unwrap();
        std::fs::write(a.path().join("y.toml"), "command = [\"/bin/y\"]").unwrap();
        std::fs::write(b.path().join("x.toml"), "command = [\"/bin/b\"]").unwrap();
        let set = ServiceSet::load_dirs(&[a.path().into(), b.path().into(), "/nonexistent".into()])
            .unwrap();
        assert_eq!(set.services["x"].command[0], "/bin/b");
        assert!(set.services.contains_key("y"));
    }

    #[test]
    fn signals() {
        assert_eq!(parse_signal("SIGHUP"), Some(libc::SIGHUP));
        assert_eq!(parse_signal("KILL"), Some(libc::SIGKILL));
        assert_eq!(parse_signal("15"), Some(15));
        assert_eq!(parse_signal("NOPE"), None);
        assert_eq!(parse_signal("99"), None);
    }

    #[test]
    fn unknown_dependency_and_cycle() {
        let a = p("a", "command = [\"/bin/true\"]\ndepends = [\"b\"]").unwrap();
        assert!(matches!(
            ServiceSet::from_services(vec![a.clone()]),
            Err(ConfigError::UnknownDependency { .. })
        ));
        let b = p("b", "command = [\"/bin/true\"]\ndepends = [\"a\"]").unwrap();
        assert!(matches!(
            ServiceSet::from_services(vec![a, b]),
            Err(ConfigError::Cycle(_))
        ));
    }

    #[test]
    fn missing_soft_dependency_is_ignored() {
        let a = p("a", "command = [\"/bin/true\"]\nwants = [\"ghost\"]").unwrap();
        let set = ServiceSet::from_services(vec![a]).unwrap();
        assert_eq!(set.order_deps(&set.services["a"]).count(), 0);
    }
}
