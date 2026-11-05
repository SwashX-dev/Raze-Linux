//! JSON IPC over a Unix socket (`$RIFT_SOCKET`), used by Rift Shell and `rift msg`.
//!
//! Each request is one line of text; each reply is one line of JSON. Requests:
//!   version | outputs | workspaces | windows | focused-window | state
//!   action <action as in the [binds] table>   e.g. `action workspace 2`
//!   subscribe                                 stream `state` on every change
//!   night-light <kelvin>|off                  colour temperature of every output
//!   frames                                    (headless) frames rendered so far

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{json, Value};
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{Interest, LoopHandle, Mode, PostAction};

use crate::state::Rift;

pub struct Ipc {
    pub path: PathBuf,
    subscribers: Vec<UnixStream>,
    dirty: bool,
}

impl Ipc {
    pub fn new(handle: &LoopHandle<'static, Rift>, socket_name: &std::ffi::OsStr) -> Ipc {
        let dir = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let path = dir.join(format!(
            "rift.{}.{}.sock",
            socket_name.to_string_lossy(),
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        match UnixListener::bind(&path) {
            Ok(listener) => {
                let _ = listener.set_nonblocking(true);
                let source = Generic::new(listener, Interest::READ, Mode::Level);
                let r = handle.insert_source(source, |_, listener, state: &mut Rift| {
                    while let Ok((stream, _)) = listener.accept() {
                        state.ipc_accept(stream);
                    }
                    Ok(PostAction::Continue)
                });
                if let Err(e) = r {
                    tracing::warn!("IPC disabled: {e}");
                }
            }
            Err(e) => tracing::warn!("IPC disabled: cannot bind {}: {e}", path.display()),
        }
        Ipc {
            path,
            subscribers: Vec::new(),
            dirty: true,
        }
    }
}

impl Drop for Ipc {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

impl Rift {
    fn ipc_accept(&mut self, stream: UnixStream) {
        let _ = stream.set_nonblocking(false);
        let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
        let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
        let mut reader = BufReader::new(match stream.try_clone() {
            Ok(s) => s,
            Err(_) => return,
        });
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        let request = line.trim();
        let mut stream = stream;
        if request == "subscribe" {
            let state = self.ipc_snapshot();
            if writeln!(stream, "{state}").is_ok() {
                self.ipc.subscribers.push(stream);
            }
            return;
        }
        let reply = self.ipc_handle(request);
        let _ = writeln!(stream, "{reply}");
    }

    pub fn ipc_handle(&mut self, request: &str) -> Value {
        let (cmd, arg) = request
            .split_once(' ')
            .map(|(a, b)| (a, b.trim()))
            .unwrap_or((request, ""));
        match cmd {
            "version" => {
                json!({ "version": env!("CARGO_PKG_VERSION"), "backend": self.backend.name() })
            }
            "outputs" => self.ipc_outputs(),
            "workspaces" => json!(self.layout.workspaces()),
            "windows" => self.ipc_windows(),
            "focused-window" => self
                .layout
                .focused_window()
                .map(|w| {
                    let (app_id, title) = Self::app_id_title(w);
                    // Geometry in the global (xdg-output) coordinate space, as grim expects.
                    let geo = self.space.element_geometry(w).unwrap_or_default();
                    let output = self.space.outputs_for_element(w).first().map(|o| o.name());
                    json!({
                        "app_id": app_id, "title": title, "output": output,
                        "x": geo.loc.x, "y": geo.loc.y, "width": geo.size.w, "height": geo.size.h,
                    })
                })
                .unwrap_or(Value::Null),
            "night-light" => {
                let temp = match arg {
                    "off" | "" => None,
                    t => match t.parse::<u32>() {
                        Ok(k) if (1000..=10000).contains(&k) => Some(k),
                        _ => {
                            return json!({ "error": "night-light expects a temperature in kelvin (1000-10000) or off" })
                        }
                    },
                };
                self.night_light = temp;
                let applied = self.apply_night_light();
                json!({ "ok": true, "applied": applied })
            }
            "state" => self.ipc_snapshot(),
            "frames" => match &self.backend {
                crate::backend::Backend::Headless(h) => json!({ "frames": h.frames }),
                _ => json!({ "error": "only available on the headless backend" }),
            },
            "action" => match crate::config::Action::parse(arg) {
                Ok(action) => {
                    self.do_action(action);
                    json!({ "ok": true })
                }
                Err(e) => json!({ "error": e }),
            },
            "config-error" => json!({ "error": self.config_error }),
            _ => json!({ "error": format!("unknown request {cmd:?}") }),
        }
    }

    fn ipc_outputs(&self) -> Value {
        Value::Array(
            self.outputs
                .iter()
                .map(|o| {
                    let mode = o.output.current_mode();
                    let size = self.output_logical_size(&o.output);
                    json!({
                        "name": o.output.name(),
                        "mode": mode.map(|m| json!({ "width": m.size.w, "height": m.size.h, "refresh": m.refresh })),
                        "scale": o.output.current_scale().fractional_scale(),
                        "logical": { "width": size.w, "height": size.h },
                        "powered": o.powered,
                    })
                })
                .collect(),
        )
    }

    fn ipc_windows(&self) -> Value {
        let focused = self.focused_window();
        Value::Array(
            self.layout
                .windows()
                .map(|w| (w, self.layout.workspace_of(w), false))
                .chain(
                    self.floating
                        .iter()
                        .map(|f| (&f.window, Some(f.workspace), true)),
                )
                .map(|(w, workspace, floating)| {
                    let (app_id, title) = Self::app_id_title(w);
                    json!({
                        "app_id": app_id,
                        "title": title,
                        "workspace": workspace,
                        "focused": Some(w) == focused,
                        "floating": floating,
                    })
                })
                .collect(),
        )
    }

    fn ipc_snapshot(&self) -> Value {
        let focused = self.focused_window().map(|w| {
            let (app_id, title) = Self::app_id_title(w);
            json!({ "app_id": app_id, "title": title })
        });
        json!({
            "workspaces": self.layout.workspaces(),
            "windows": self.ipc_windows(),
            "focused": focused,
            "outputs": self.ipc_outputs(),
            "locked": self.locked,
        })
    }

    /// Note that the published state may have changed (sent from `refresh`).
    pub fn ipc_state_changed(&mut self) {
        self.ipc.dirty = true;
    }

    pub fn ipc_flush(&mut self) {
        if !self.ipc.dirty {
            return;
        }
        self.ipc.dirty = false;
        if self.ipc.subscribers.is_empty() {
            return;
        }
        let snapshot = self.ipc_snapshot().to_string();
        let mut h = DefaultHasher::new();
        snapshot.hash(&mut h);
        let hash = h.finish();
        if hash == self.last_state_hash {
            return;
        }
        self.last_state_hash = hash;
        self.ipc
            .subscribers
            .retain_mut(|s| writeln!(s, "{snapshot}").is_ok());
    }
}

/// `rift msg ...`: send one request and print the reply (or the stream, for `subscribe`).
pub fn client(args: &[String]) -> i32 {
    let Some(path) = std::env::var_os("RIFT_SOCKET") else {
        eprintln!("rift msg: RIFT_SOCKET is not set (is Rift running?)");
        return 1;
    };
    let mut stream = match UnixStream::connect(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "rift msg: cannot connect to {}: {e}",
                PathBuf::from(path).display()
            );
            return 1;
        }
    };
    let request = if args.is_empty() {
        "state".to_string()
    } else {
        args.join(" ")
    };
    if writeln!(stream, "{request}").is_err() {
        return 1;
    }
    let reader = BufReader::new(stream);
    let mut code = 0;
    for line in reader.lines() {
        let Ok(line) = line else { break };
        if serde_json::from_str::<Value>(&line)
            .ok()
            .and_then(|v| v.get("error").cloned())
            .is_some_and(|e| !e.is_null())
        {
            code = 1;
        }
        println!("{line}");
        if request != "subscribe" {
            break;
        }
    }
    code
}
