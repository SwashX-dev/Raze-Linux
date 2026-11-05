//! On-demand XWayland: Rift owns the X11 display sockets and starts
//! xwayland-satellite (which starts Xwayland) only when an X client connects.
//! Until then no X server runs; if it exits, Rift listens again.

use std::os::fd::{AsFd, AsRawFd, OwnedFd};
use std::os::linux::net::SocketAddrExt;
use std::os::unix::net::{SocketAddr, UnixListener};
use std::process::{Command, Stdio};

use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{Interest, Mode, PostAction, RegistrationToken};

use crate::state::Rift;

pub struct Xwayland {
    pub display: u32,
    /// Filesystem (/tmp/.X11-unix/Xn) and abstract (@/tmp/.X11-unix/Xn) sockets.
    sockets: Vec<OwnedFd>,
    watches: Vec<RegistrationToken>,
    /// PID of the running xwayland-satellite, if any.
    pub child: Option<u32>,
}

impl Drop for Xwayland {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(format!("/tmp/.X11-unix/X{}", self.display));
        let _ = std::fs::remove_file(format!("/tmp/.X{}-lock", self.display));
    }
}

/// Claim the first free X display: lock file (as X servers do) and both sockets.
fn claim_display() -> Option<(u32, Vec<OwnedFd>)> {
    let _ = std::fs::create_dir_all("/tmp/.X11-unix");
    for n in 0..32u32 {
        let lock = format!("/tmp/.X{n}-lock");
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock);
        let Ok(mut file) = file else { continue };
        use std::io::Write;
        let _ = writeln!(file, "{:>10}", std::process::id());
        let path = format!("/tmp/.X11-unix/X{n}");
        let _ = std::fs::remove_file(&path);
        let fs = UnixListener::bind(&path);
        let abs = SocketAddr::from_abstract_name(path.as_bytes())
            .and_then(|a| UnixListener::bind_addr(&a));
        match (fs, abs) {
            (Ok(fs), Ok(abs)) => return Some((n, vec![fs.into(), abs.into()])),
            _ => {
                let _ = std::fs::remove_file(&lock);
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    None
}

impl Rift {
    /// Reserve an X display and wait for X clients (if `xwayland` is enabled and
    /// xwayland-satellite is installed).
    pub fn start_xwayland(&mut self) {
        if !self.config.xwayland {
            return;
        }
        let supported = Command::new("xwayland-satellite")
            .arg("--test-listenfd-support")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !supported {
            tracing::info!(
                "XWayland disabled: xwayland-satellite (with -listenfd) is not installed"
            );
            return;
        }
        let Some((x_display, sockets)) = claim_display() else {
            tracing::warn!("XWayland disabled: no free X display");
            return;
        };
        self.xwayland_display = Some(format!(":{x_display}"));
        self.xwayland = Some(Xwayland {
            display: x_display,
            sockets,
            watches: Vec::new(),
            child: None,
        });
        self.watch_x_sockets();
        tracing::info!("DISPLAY=:{x_display}; Xwayland starts when an X client connects");
    }

    fn watch_x_sockets(&mut self) {
        let Some(x) = &mut self.xwayland else { return };
        let mut tokens = Vec::new();
        for fd in &x.sockets {
            let Ok(dup) = fd.try_clone() else { continue };
            let source = Generic::new(dup, Interest::READ, Mode::Level);
            let token = self
                .loop_handle
                .insert_source(source, |_, _, state: &mut Rift| {
                    // Removes both watches (this one included) before starting the child.
                    state.spawn_xwayland_satellite();
                    Ok(PostAction::Continue)
                });
            if let Ok(t) = token {
                tokens.push(t);
            }
        }
        x.watches = tokens;
    }

    fn spawn_xwayland_satellite(&mut self) {
        let env = self.child_env();
        let Some(x) = &mut self.xwayland else { return };
        for t in x.watches.drain(..) {
            self.loop_handle.remove(t);
        }
        if x.child.is_some() {
            return;
        }
        let fds: Vec<i32> = x.sockets.iter().map(|f| f.as_fd().as_raw_fd()).collect();
        let mut cmd = Command::new("xwayland-satellite");
        cmd.arg(format!(":{}", x.display)).stdin(Stdio::null());
        for fd in &fds {
            cmd.arg("-listenfd").arg(fd.to_string());
        }
        for (k, v) in env {
            if k != "DISPLAY" {
                cmd.env(k, v);
            }
        }
        // The listening sockets are inherited by the child (they are CLOEXEC here).
        unsafe {
            use std::os::unix::process::CommandExt;
            cmd.pre_exec(move || {
                for fd in &fds {
                    let flags = libc::fcntl(*fd, libc::F_GETFD);
                    libc::fcntl(*fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC);
                }
                Ok(())
            });
        }
        match cmd.spawn() {
            Ok(child) => {
                tracing::info!(
                    "X client connecting: started xwayland-satellite (pid {})",
                    child.id()
                );
                x.child = Some(child.id());
                self.children.push(child);
            }
            Err(e) => {
                tracing::warn!("cannot start xwayland-satellite: {e}");
                self.watch_x_sockets();
            }
        }
    }

    /// Called when a child exited: listen again if it was xwayland-satellite.
    pub fn xwayland_child_exited(&mut self, pid: u32) {
        if let Some(x) = &mut self.xwayland {
            if x.child == Some(pid) {
                tracing::info!("xwayland-satellite exited; waiting for X clients again");
                x.child = None;
                self.watch_x_sockets();
            }
        }
    }
}
