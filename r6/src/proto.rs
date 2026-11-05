//! Control protocol between `r6` (client) and `r6-supervise`.
//!
//! One JSON request line per connection on `<run_dir>/control.sock`, one JSON
//! response line back. The socket is created mode 0600, so only root (or the user
//! running the supervisor) can control services.

use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "lowercase")]
pub enum Request {
    Status {
        service: Option<String>,
    },
    Start {
        service: String,
    },
    Stop {
        service: String,
    },
    Restart {
        service: String,
    },
    Enable {
        service: String,
        now: bool,
    },
    Disable {
        service: String,
        now: bool,
    },
    Reload,
    Info,
    Shutdown,
    /// Import variables into the environment of services started afterwards
    /// (graphical sessions import WAYLAND_DISPLAY, DISPLAY, …).
    SetEnv {
        vars: std::collections::BTreeMap<String, String>,
    },
    UnsetEnv {
        names: Vec<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceStatus {
    pub name: String,
    pub description: String,
    pub kind: String,
    pub state: String,
    pub pid: Option<i32>,
    /// Milliseconds in the current state.
    pub since_ms: u64,
    pub restarts: u32,
    pub last_exit: Option<String>,
    pub failure: Option<String>,
    pub enabled: bool,
    pub wanted: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Info {
    pub pid: i32,
    pub rss_kib: Option<u64>,
    pub services: usize,
    pub running: usize,
    /// CLOCK_BOOTTIME timestamps in milliseconds.
    pub init_start_ms: Option<f64>,
    pub supervisor_start_ms: f64,
    pub boot_complete_ms: Option<f64>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Response {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub services: Vec<ServiceStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub info: Option<Info>,
}

impl Response {
    pub fn ok(msg: impl Into<String>) -> Response {
        Response {
            ok: true,
            message: Some(msg.into()),
            ..Default::default()
        }
    }
    pub fn err(msg: impl Into<String>) -> Response {
        Response {
            ok: false,
            error: Some(msg.into()),
            ..Default::default()
        }
    }
}

pub fn socket_path(run_dir: &Path) -> PathBuf {
    run_dir.join("control.sock")
}

/// Send one request and wait for the response.
pub fn call(run_dir: &Path, req: &Request) -> std::io::Result<Response> {
    let mut s = UnixStream::connect(socket_path(run_dir))?;
    s.set_read_timeout(Some(Duration::from_secs(30)))?;
    let mut line = serde_json::to_string(req)?;
    line.push('\n');
    s.write_all(line.as_bytes())?;
    let mut r = BufReader::new(s);
    let mut resp = String::new();
    r.read_line(&mut resp)?;
    serde_json::from_str(&resp).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}
