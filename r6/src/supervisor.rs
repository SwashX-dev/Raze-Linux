//! r6-supervise: a single-process, single-threaded service supervisor.
//!
//! Unlike s6, which runs one `s6-supervise` process per service (plus one `s6-log`
//! per logged service), r6 supervises every service from one event loop. Service
//! output pipes and readiness pipes are multiplexed with poll(2); child exits arrive
//! through a signalfd. This keeps the process count and idle memory low and makes
//! dependency ordering a local decision rather than an IPC protocol.
//!
//! State machine per service:
//!
//! ```text
//!   Down ──spawn──▶ Starting ──ready──▶ Up
//!    ▲                 │                │ unexpected exit
//!    │                 ▼                ▼
//!    │              (oneshot ok) ─▶ Done   Backoff ──delay──▶ Down
//!    │                                  │ too many restarts
//!    └──── stopped ◀── Stopping          ▼
//!                                     Failed (until `r6 start`)
//! ```

use crate::activation::Graph;
use crate::config::{Global, Kind, LogTarget, Ready, Restart, ServiceSet};
use crate::log::LogWriter;
use crate::paths::Paths;
use crate::proto::{Info, Request, Response, ServiceStatus};
use crate::sys::{self, Exit, PollFd};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Down,
    Starting,
    Up,
    Stopping,
    Backoff,
    Failed,
    Done,
}

impl State {
    fn name(self) -> &'static str {
        match self {
            State::Down => "down",
            State::Starting => "starting",
            State::Up => "up",
            State::Stopping => "stopping",
            State::Backoff => "backoff",
            State::Failed => "failed",
            State::Done => "done",
        }
    }
    fn running(self) -> bool {
        matches!(self, State::Starting | State::Up | State::Stopping)
    }
    fn satisfied(self) -> bool {
        matches!(self, State::Up | State::Done)
    }
}

struct Unit {
    state: State,
    wanted: bool,
    pid: Option<i32>,
    since: Instant,
    restarts: u32,
    recent: VecDeque<Instant>,
    timer: Option<Instant>,
    killed: bool,
    notify: Option<OwnedFd>,
    notify_buf: Vec<u8>,
    out: Option<OwnedFd>,
    log: Option<LogWriter>,
    last_exit: Option<String>,
    failure: Option<String>,
    /// Restart requested: respawn as soon as the current process is gone.
    restart_pending: bool,
}

impl Unit {
    fn new() -> Unit {
        Unit {
            state: State::Down,
            wanted: false,
            pid: None,
            since: Instant::now(),
            restarts: 0,
            recent: VecDeque::new(),
            timer: None,
            killed: false,
            notify: None,
            notify_buf: Vec::new(),
            out: None,
            log: None,
            last_exit: None,
            failure: None,
            restart_pending: false,
        }
    }
    fn set(&mut self, s: State) {
        if self.state != s {
            self.state = s;
            self.since = Instant::now();
        }
    }
}

struct Client {
    stream: UnixStream,
    buf: Vec<u8>,
}

pub struct Supervisor {
    paths: Paths,
    global: Global,
    log_dir: PathBuf,
    set: ServiceSet,
    retired: BTreeSet<String>,
    units: BTreeMap<String, Unit>,
    pids: BTreeMap<i32, String>,
    listener: UnixListener,
    clients: Vec<Client>,
    sigfd: OwnedFd,
    shutting_down: Option<Instant>,
    boot_targets: Option<BTreeSet<String>>,
    started_ms: f64,
    boot_complete_ms: Option<f64>,
    verbose: bool,
}

const ENV_PATH: &str = "/run/wrappers/bin:/run/current-system/sw/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

impl Supervisor {
    pub fn new(paths: Paths) -> Result<Supervisor, String> {
        let started_ms = sys::boottime_ms();
        let global = Global::load(&paths.global_conf).map_err(|e| e.to_string())?;
        let set = ServiceSet::load_dir(&paths.service_dir).map_err(|e| e.to_string())?;
        let log_dir = paths
            .log_dir_override
            .clone()
            .unwrap_or_else(|| global.log_dir.clone());
        std::fs::create_dir_all(&paths.run_dir)
            .map_err(|e| format!("{}: {e}", paths.run_dir.display()))?;
        let sock = crate::proto::socket_path(&paths.run_dir);
        let _ = std::fs::remove_file(&sock);
        let old = unsafe { libc::umask(0o077) };
        let listener = UnixListener::bind(&sock).map_err(|e| format!("{}: {e}", sock.display()));
        unsafe { libc::umask(old) };
        let listener = listener?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let sigfd = sys::signalfd(&[libc::SIGCHLD, libc::SIGTERM, libc::SIGINT, libc::SIGHUP])
            .map_err(|e| format!("signalfd: {e}"))?;
        // Adopt orphans of daemonizing services so they are reaped here, not leaked.
        let _ = sys::set_child_subreaper();
        let units = set
            .services
            .keys()
            .map(|k| (k.clone(), Unit::new()))
            .collect();
        Ok(Supervisor {
            paths,
            global,
            log_dir,
            set,
            retired: BTreeSet::new(),
            units,
            pids: BTreeMap::new(),
            listener,
            clients: Vec::new(),
            sigfd,
            shutting_down: None,
            boot_targets: None,
            started_ms,
            boot_complete_ms: None,
            verbose: std::env::var_os("R6_QUIET").is_none(),
        })
    }

    fn say(&self, msg: &str) {
        if self.verbose {
            eprintln!("r6: {msg}");
        }
    }

    // ---------------------------------------------------------------- enablement

    fn override_path(&self, kind: &str, name: &str) -> PathBuf {
        self.paths.state_dir.join(kind).join(name)
    }

    fn is_enabled(&self, name: &str) -> bool {
        if self.override_path("disabled", name).exists() {
            return false;
        }
        if self.override_path("enabled", name).exists() {
            return true;
        }
        self.set
            .services
            .get(name)
            .map(|s| s.enable)
            .unwrap_or(false)
    }

    fn set_enabled(&self, name: &str, on: bool) -> std::io::Result<()> {
        let (add, del) = if on {
            ("enabled", "disabled")
        } else {
            ("disabled", "enabled")
        };
        let _ = std::fs::remove_file(self.override_path(del, name));
        let def = self
            .set
            .services
            .get(name)
            .map(|s| s.enable)
            .unwrap_or(false);
        if def == on {
            // Matches the declarative default: no override needed.
            let _ = std::fs::remove_file(self.override_path(add, name));
            return Ok(());
        }
        let p = self.override_path(add, name);
        std::fs::create_dir_all(p.parent().unwrap())?;
        std::fs::write(p, b"")
    }

    // ---------------------------------------------------------------- wanted-set changes

    /// Want `name` and everything it needs.
    fn want(&mut self, name: &str) {
        let g = Graph::new(&self.set);
        let Some(i) = g.index(name) else { return };
        let names: Vec<String> = g
            .closure([i])
            .into_iter()
            .map(|j| g.names[j].to_string())
            .collect();
        for n in names {
            if let Some(u) = self.units.get_mut(&n) {
                u.wanted = true;
                if u.state == State::Failed || (u.state == State::Done && n == name) {
                    // Explicit start clears a failure / re-runs a finished oneshot.
                    u.set(State::Down);
                    u.recent.clear();
                    u.failure = None;
                }
            }
        }
    }

    /// Stop `name` and everything that needs it.
    fn unwant(&mut self, name: &str) {
        let g = Graph::new(&self.set);
        let Some(i) = g.index(name) else { return };
        let mut names: Vec<String> = g
            .dependents(i)
            .into_iter()
            .map(|j| g.names[j].to_string())
            .collect();
        names.push(name.to_string());
        for n in names {
            if let Some(u) = self.units.get_mut(&n) {
                u.wanted = false;
                u.restart_pending = false;
            }
        }
    }

    pub fn activate_boot_set(&mut self) {
        let enabled: Vec<String> = self
            .set
            .services
            .keys()
            .filter(|n| self.is_enabled(n))
            .cloned()
            .collect();
        for n in &enabled {
            self.want(n);
        }
        let targets: BTreeSet<String> = self
            .units
            .iter()
            .filter(|(_, u)| u.wanted)
            .map(|(n, _)| n.clone())
            .collect();
        self.say(&format!("activating {} service(s)", targets.len()));
        self.boot_targets = Some(targets);
    }

    // ---------------------------------------------------------------- process control

    fn spawn(&mut self, name: &str) {
        let svc = self.set.services[name].clone();
        let mut cmd = Command::new(&svc.command[0]);
        cmd.args(&svc.command[1..]);
        cmd.env_clear();
        let path = std::env::var("PATH").unwrap_or_else(|_| ENV_PATH.into());
        cmd.env("PATH", path);
        if let Ok(lang) = std::env::var("LANG") {
            cmd.env("LANG", lang);
        }
        cmd.env("R6_SERVICE", name);
        for (k, v) in &svc.environment {
            cmd.env(k, v);
        }
        cmd.current_dir(svc.working_directory.clone().unwrap_or_else(|| "/".into()));
        cmd.stdin(Stdio::null());

        let mut out_read = None;
        match svc.log {
            LogTarget::File => match sys::pipe() {
                Ok((r, w)) => {
                    let w2 = w.try_clone().ok();
                    cmd.stdout(Stdio::from(w));
                    match w2 {
                        Some(w2) => cmd.stderr(Stdio::from(w2)),
                        None => cmd.stderr(Stdio::null()),
                    };
                    let _ = sys::set_nonblocking(r.as_raw_fd());
                    out_read = Some(r);
                }
                Err(_) => {
                    cmd.stdout(Stdio::null()).stderr(Stdio::null());
                }
            },
            LogTarget::Console => {
                cmd.stdout(Stdio::inherit()).stderr(Stdio::inherit());
            }
            LogTarget::Null => {
                cmd.stdout(Stdio::null()).stderr(Stdio::null());
            }
        }

        let mut notify_pair = None;
        if svc.kind == Kind::Longrun && svc.ready == Ready::Notify {
            match sys::pipe() {
                Ok(p) => {
                    cmd.env("R6_NOTIFY_FD", "3");
                    notify_pair = Some(p);
                }
                Err(e) => return self.on_spawn_error(name, format!("notify pipe: {e}")),
            }
        }

        let creds = match &svc.user {
            None => None,
            Some(user) => match sys::lookup_user(user) {
                Some((uid, pgid)) => {
                    let gid = match &svc.group {
                        Some(g) => match sys::lookup_group(g) {
                            Some(gid) => gid,
                            None => {
                                return self.on_spawn_error(name, format!("unknown group '{g}'"))
                            }
                        },
                        None => pgid,
                    };
                    Some((sys::grouplist(user, gid), gid, uid))
                }
                None => return self.on_spawn_error(name, format!("unknown user '{user}'")),
            },
        };
        let notify_w = notify_pair.as_ref().map(|(_, w)| w.as_raw_fd());
        // SAFETY: the closure only calls async-signal-safe functions (setsid, sigprocmask,
        // signal, dup2, fcntl, setgroups, setgid, setuid) and allocates nothing.
        unsafe {
            cmd.pre_exec(move || {
                sys::setsid();
                sys::reset_signals_in_child();
                if let Some(w) = notify_w {
                    sys::dup2(w, 3)?;
                }
                if let Some((groups, gid, uid)) = &creds {
                    sys::drop_privileges(groups, *gid, *uid)?;
                }
                Ok(())
            });
        }

        match cmd.spawn() {
            Ok(child) => {
                let pid = child.id() as i32;
                drop(child); // We reap via waitpid ourselves.
                drop(cmd); // Close our copies of the child's pipe ends.
                let log = if out_read.is_some() {
                    Some(LogWriter::new(
                        &self.log_dir.join(name),
                        self.global.log_max_bytes,
                        self.global.log_keep,
                    ))
                } else {
                    None
                };
                self.pids.insert(pid, name.to_string());
                self.write_pidfile(name, pid);
                let u = self.units.get_mut(name).unwrap();
                u.pid = Some(pid);
                u.killed = false;
                u.out = out_read;
                if u.log.is_none() {
                    u.log = log;
                }
                if let Some(l) = u.log.as_mut() {
                    l.note(&format!("started (pid {pid})"));
                }
                u.set(State::Starting);
                u.timer = None;
                match (svc.kind, svc.ready) {
                    (Kind::Longrun, Ready::Immediate) => u.set(State::Up),
                    (Kind::Longrun, Ready::Notify) => {
                        let (r, _w) = notify_pair.unwrap(); // _w dropped: only the child keeps it.
                        let _ = sys::set_nonblocking(r.as_raw_fd());
                        u.notify = Some(r);
                        u.notify_buf.clear();
                        u.timer =
                            Some(Instant::now() + Duration::from_millis(svc.ready_timeout_ms));
                    }
                    (Kind::Oneshot, _) => {}
                }
                self.say(&format!("started {name} (pid {pid})"));
            }
            Err(e) => self.on_spawn_error(name, format!("exec {}: {e}", svc.command[0])),
        }
    }

    fn pidfile(&self, name: &str) -> PathBuf {
        self.paths.run_dir.join("pids").join(name)
    }

    fn write_pidfile(&self, name: &str, pid: i32) {
        let p = self.pidfile(name);
        let _ = std::fs::create_dir_all(p.parent().unwrap());
        if let Some(st) = sys::proc_starttime(pid) {
            let _ = std::fs::write(p, format!("{pid} {st}\n"));
        }
    }

    /// Terminate service instances left behind by a previous supervisor that crashed or
    /// was killed, so a restarted supervisor never runs duplicates. A pidfile only counts
    /// if the process start time still matches, which rules out pid reuse.
    pub fn reap_stale(&mut self) {
        let dir = self.paths.run_dir.join("pids");
        let Ok(rd) = std::fs::read_dir(&dir) else {
            return;
        };
        let mut victims = Vec::new();
        for ent in rd.flatten() {
            let text = std::fs::read_to_string(ent.path()).unwrap_or_default();
            let _ = std::fs::remove_file(ent.path());
            let mut it = text.split_whitespace();
            let (Some(pid), Some(st)) = (
                it.next().and_then(|v| v.parse::<i32>().ok()),
                it.next().and_then(|v| v.parse::<u64>().ok()),
            ) else {
                continue;
            };
            if pid > 1 && sys::proc_starttime(pid) == Some(st) {
                let name = ent.file_name().to_string_lossy().into_owned();
                self.say(&format!("stopping stale instance of {name} (pid {pid})"));
                let sig = self
                    .set
                    .services
                    .get(&name)
                    .map(|s| s.stop_signal)
                    .unwrap_or(libc::SIGTERM);
                Self::signal_group(pid, sig);
                victims.push((pid, st));
            }
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline
            && victims
                .iter()
                .any(|(p, st)| sys::proc_starttime(*p) == Some(*st))
        {
            while sys::reap_any().is_some() {}
            std::thread::sleep(Duration::from_millis(10));
        }
        for (p, st) in victims {
            if sys::proc_starttime(p) == Some(st) {
                Self::signal_group(p, libc::SIGKILL);
            }
        }
    }

    fn on_spawn_error(&mut self, name: &str, reason: String) {
        self.say(&format!("{name}: {reason}"));
        let u = self.units.get_mut(name).unwrap();
        u.last_exit = Some(reason.clone());
        self.after_exit(name, false, reason);
    }

    fn signal_group(pid: i32, sig: i32) {
        // Services run in their own session; signal the whole process group so shell
        // wrappers and their children stop together. Fall back to the pid alone.
        if sys::kill(-pid, sig).is_err() {
            let _ = sys::kill(pid, sig);
        }
    }

    fn begin_stop(&mut self, name: &str) {
        let timeout = self.set.services[name].stop_timeout_ms;
        let sig = self.set.services[name].stop_signal;
        let u = self.units.get_mut(name).unwrap();
        if let Some(pid) = u.pid {
            Self::signal_group(pid, sig);
            if sig != libc::SIGKILL {
                let _ = sys::kill(pid, libc::SIGCONT);
            }
            u.set(State::Stopping);
            u.timer = Some(Instant::now() + Duration::from_millis(timeout));
        } else {
            u.set(State::Down);
        }
    }

    fn drain_output(u: &mut Unit) {
        let Some(fd) = u.out.as_ref() else { return };
        let mut buf = [0u8; 8192];
        loop {
            match sys::read(fd.as_raw_fd(), &mut buf) {
                Ok(0) => {
                    u.out = None;
                    break;
                }
                Ok(n) => {
                    if let Some(l) = u.log.as_mut() {
                        let _ = l.write(&buf[..n]);
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
    }

    fn on_exit(&mut self, pid: i32, exit: Exit) {
        let Some(name) = self.pids.remove(&pid) else {
            return;
        };
        let _ = std::fs::remove_file(self.pidfile(&name));
        let kind = self.set.services.get(&name).map(|s| s.kind);
        let u = match self.units.get_mut(&name) {
            Some(u) => u,
            None => return,
        };
        Self::drain_output(u);
        u.out = None;
        u.notify = None;
        u.pid = None;
        u.timer = None;
        let desc = exit.describe();
        if let Some(l) = u.log.as_mut() {
            let _ = l.flush_partial();
            l.note(&format!("exited ({desc})"));
        }
        u.last_exit = Some(desc.clone());
        if u.state == State::Stopping {
            u.set(State::Down);
            if u.restart_pending {
                u.restart_pending = false;
            }
            self.say(&format!("stopped {name} ({desc})"));
            return;
        }
        let ok = exit.success();
        if kind == Some(Kind::Oneshot) && ok {
            u.set(State::Done);
            self.say(&format!("{name} completed"));
            return;
        }
        self.say(&format!("{name} exited unexpectedly ({desc})"));
        self.after_exit(&name, ok, desc);
    }

    /// Apply the restart policy after a failed spawn or an unexpected exit.
    fn after_exit(&mut self, name: &str, success: bool, reason: String) {
        let svc = &self.set.services[name];
        let restart = match svc.restart {
            Restart::Always => true,
            Restart::OnFailure => !success,
            Restart::Never => false,
        };
        let (delay_ms, max, window) = (
            svc.restart_delay_ms,
            svc.restart_max,
            Duration::from_secs(svc.restart_window_s),
        );
        let shutting = self.shutting_down.is_some();
        let u = self.units.get_mut(name).unwrap();
        u.pid = None;
        if !restart || shutting || !u.wanted {
            if success {
                u.set(State::Done);
            } else {
                u.failure = Some(reason);
                u.set(State::Failed);
            }
            return;
        }
        let now = Instant::now();
        while u
            .recent
            .front()
            .map(|t| now.duration_since(*t) > window)
            .unwrap_or(false)
        {
            u.recent.pop_front();
        }
        if u.recent.len() as u32 >= max {
            let msg = format!(
                "restart limit reached ({} restarts in {}s); last: {reason}",
                u.recent.len(),
                window.as_secs()
            );
            if let Some(l) = u.log.as_mut() {
                l.note(&msg);
            }
            u.failure = Some(msg.clone());
            u.set(State::Failed);
            self.say(&format!("{name}: {msg}"));
            return;
        }
        // Exponential backoff, capped at 30 s.
        let factor = 1u64 << u.recent.len().min(7);
        let delay = Duration::from_millis((delay_ms * factor).min(30_000));
        u.recent.push_back(now);
        u.restarts += 1;
        u.timer = Some(now + delay);
        u.set(State::Backoff);
    }

    // ---------------------------------------------------------------- reconciliation

    /// Drive every service toward its wanted state. Iterates to a fixed point because
    /// one change (e.g. a dependency becoming ready) can enable others.
    fn reconcile(&mut self) {
        let now = Instant::now();
        let order: Vec<String> = {
            let g = Graph::new(&self.set);
            match g.topo_order() {
                Ok(o) => o.into_iter().map(|i| g.names[i].to_string()).collect(),
                Err(_) => self.set.services.keys().cloned().collect(),
            }
        };
        for name in &self.retired {
            if let Some(u) = self.units.get_mut(name) {
                u.wanted = false;
            }
        }
        for _ in 0..(order.len() + 2) {
            let mut changed = false;
            for name in &order {
                changed |= self.step(name, now);
            }
            for name in order.iter().rev() {
                changed |= self.step_down(name);
            }
            if !changed {
                break;
            }
        }
        // Forget retired services once they are no longer running.
        let gone: Vec<String> = self
            .retired
            .iter()
            .filter(|n| {
                self.units
                    .get(*n)
                    .map(|u| !u.state.running() && u.pid.is_none())
                    .unwrap_or(true)
            })
            .cloned()
            .collect();
        for n in gone {
            self.retired.remove(&n);
            self.units.remove(&n);
            self.set.services.remove(&n);
        }
        self.check_boot_complete();
    }

    /// Upward transitions and timers for one service. Returns true on change.
    fn step(&mut self, name: &str, now: Instant) -> bool {
        let Some(svc) = self.set.services.get(name) else {
            return false;
        };
        let u = &self.units[name];
        let state = u.state;
        let timer_due = u.timer.map(|t| now >= t).unwrap_or(false);

        // Timers.
        if timer_due {
            match state {
                State::Backoff => {
                    let u = self.units.get_mut(name).unwrap();
                    u.timer = None;
                    u.set(State::Down);
                    return true;
                }
                State::Stopping if !u.killed => {
                    let pid = u.pid;
                    let u = self.units.get_mut(name).unwrap();
                    u.killed = true;
                    u.timer = None;
                    if let Some(pid) = pid {
                        self.say(&format!("{name}: stop timeout, sending SIGKILL"));
                        Self::signal_group(pid, libc::SIGKILL);
                    }
                    return true;
                }
                State::Starting => {
                    // Readiness timeout: kill; the exit will be handled as a failure.
                    let pid = u.pid;
                    let u = self.units.get_mut(name).unwrap();
                    u.timer = None;
                    if let Some(pid) = pid {
                        self.say(&format!("{name}: readiness timeout"));
                        Self::signal_group(pid, libc::SIGKILL);
                    }
                    return true;
                }
                _ => {}
            }
        }

        if !u.wanted {
            return false;
        }
        match state {
            State::Down => {
                // Hard dependency failure propagates deterministically.
                for d in &svc.depends {
                    if self
                        .units
                        .get(d)
                        .map(|x| x.state == State::Failed)
                        .unwrap_or(true)
                    {
                        let msg = format!("dependency '{d}' failed");
                        let u = self.units.get_mut(name).unwrap();
                        u.failure = Some(msg);
                        u.set(State::Failed);
                        return true;
                    }
                }
                let hard_ok = svc.depends.iter().all(|d| self.units[d].state.satisfied());
                let soft_ok = svc.wants.iter().all(|w| match self.units.get(w) {
                    None => true,
                    Some(x) if !x.wanted => true,
                    Some(x) => x.state.satisfied() || x.state == State::Failed,
                });
                if hard_ok && soft_ok {
                    let n = name.to_string();
                    self.spawn(&n);
                    return true;
                }
                false
            }
            _ => false,
        }
    }

    /// Downward transitions: stop unwanted services once nothing running depends on them.
    fn step_down(&mut self, name: &str) -> bool {
        let Some(u) = self.units.get(name) else {
            return false;
        };
        if u.restart_pending && u.state.running() && u.state != State::Stopping {
            let n = name.to_string();
            self.begin_stop(&n);
            return true;
        }
        if u.wanted {
            return false;
        }
        match u.state {
            State::Starting | State::Up => {
                let dependents_running = self.set.services.values().any(|s| {
                    self.set.order_deps(s).any(|d| d == name)
                        && self
                            .units
                            .get(&s.name)
                            .map(|x| x.state.running())
                            .unwrap_or(false)
                });
                if dependents_running {
                    return false;
                }
                let n = name.to_string();
                self.begin_stop(&n);
                true
            }
            State::Backoff | State::Done => {
                let u = self.units.get_mut(name).unwrap();
                u.timer = None;
                u.set(State::Down);
                true
            }
            _ => false,
        }
    }

    fn check_boot_complete(&mut self) {
        if self.boot_complete_ms.is_some() {
            return;
        }
        let Some(targets) = &self.boot_targets else {
            return;
        };
        let settled = targets.iter().all(|n| {
            self.units
                .get(n)
                .map(|u| matches!(u.state, State::Up | State::Done | State::Failed))
                .unwrap_or(true)
        });
        if settled {
            let now = sys::boottime_ms();
            self.boot_complete_ms = Some(now);
            let failed: Vec<&String> = targets
                .iter()
                .filter(|n| {
                    self.units
                        .get(*n)
                        .map(|u| u.state == State::Failed)
                        .unwrap_or(false)
                })
                .collect();
            let init = init_start_ms();
            let msg = format!(
                "boot complete: {} service(s) in {:.1} ms (supervisor), {:.1} ms since r6-init, {:.1} ms since kernel start{}",
                targets.len(),
                now - self.started_ms,
                init.map(|i| now - i).unwrap_or(f64::NAN),
                now,
                if failed.is_empty() { String::new() } else { format!("; FAILED: {failed:?}") }
            );
            self.say(&msg);
            let _ = std::fs::write(self.paths.run_dir.join("boot-complete"), format!("{msg}\n"));
        }
    }

    // ---------------------------------------------------------------- control

    fn status_of(&self, name: &str) -> Option<ServiceStatus> {
        let s = self.set.services.get(name)?;
        let u = self.units.get(name)?;
        Some(ServiceStatus {
            name: name.into(),
            description: s.description.clone(),
            kind: match s.kind {
                Kind::Longrun => "longrun".into(),
                Kind::Oneshot => "oneshot".into(),
            },
            state: u.state.name().into(),
            pid: u.pid,
            since_ms: u.since.elapsed().as_millis() as u64,
            restarts: u.restarts,
            last_exit: u.last_exit.clone(),
            failure: u.failure.clone(),
            enabled: self.is_enabled(name),
            wanted: u.wanted,
        })
    }

    fn handle(&mut self, req: Request) -> Response {
        let known =
            |s: &Supervisor, n: &str| s.set.services.contains_key(n) && !s.retired.contains(n);
        if self.shutting_down.is_some() && !matches!(req, Request::Status { .. } | Request::Info) {
            return Response::err("system is shutting down");
        }
        match req {
            Request::Status { service: None } => Response {
                ok: true,
                services: self
                    .set
                    .services
                    .keys()
                    .filter_map(|n| self.status_of(n))
                    .collect(),
                ..Default::default()
            },
            Request::Status { service: Some(n) } => match self.status_of(&n) {
                Some(s) => Response {
                    ok: true,
                    services: vec![s],
                    ..Default::default()
                },
                None => Response::err(format!("unknown service '{n}'")),
            },
            Request::Start { service } if known(self, &service) => {
                self.want(&service);
                Response::ok(format!("starting {service}"))
            }
            Request::Stop { service } if known(self, &service) => {
                self.unwant(&service);
                Response::ok(format!("stopping {service}"))
            }
            Request::Restart { service } if known(self, &service) => {
                let running = self.units[&service].state.running();
                self.want(&service);
                if running {
                    self.units.get_mut(&service).unwrap().restart_pending = true;
                }
                Response::ok(format!("restarting {service}"))
            }
            Request::Enable { service, now } if known(self, &service) => {
                match self.set_enabled(&service, true) {
                    Ok(()) => {
                        if now {
                            self.want(&service);
                        }
                        Response::ok(format!("enabled {service}"))
                    }
                    Err(e) => Response::err(format!("cannot persist: {e}")),
                }
            }
            Request::Disable { service, now } if known(self, &service) => {
                match self.set_enabled(&service, false) {
                    Ok(()) => {
                        if now {
                            self.unwant(&service);
                        }
                        Response::ok(format!("disabled {service}"))
                    }
                    Err(e) => Response::err(format!("cannot persist: {e}")),
                }
            }
            Request::Start { service }
            | Request::Stop { service }
            | Request::Restart { service }
            | Request::Enable { service, .. }
            | Request::Disable { service, .. } => {
                Response::err(format!("unknown service '{service}'"))
            }
            Request::Reload => self.reload(),
            Request::Info => Response {
                ok: true,
                info: Some(Info {
                    pid: sys::getpid(),
                    rss_kib: sys::rss_kib(),
                    services: self.set.services.len(),
                    running: self.units.values().filter(|u| u.pid.is_some()).count(),
                    init_start_ms: init_start_ms(),
                    supervisor_start_ms: self.started_ms,
                    boot_complete_ms: self.boot_complete_ms,
                }),
                ..Default::default()
            },
            Request::Shutdown => {
                self.begin_shutdown();
                Response::ok("shutting down")
            }
        }
    }

    fn reload(&mut self) -> Response {
        let new = match ServiceSet::load_dir(&self.paths.service_dir) {
            Ok(s) => s,
            Err(e) => {
                return Response::err(format!("configuration rejected, keeping current: {e}"))
            }
        };
        let mut added = 0;
        let mut removed = 0;
        for name in self.set.services.keys() {
            if !new.services.contains_key(name) {
                self.retired.insert(name.clone());
                removed += 1;
            }
        }
        for (name, svc) in new.services {
            if !self.units.contains_key(&name) {
                self.units.insert(name.clone(), Unit::new());
                added += 1;
            }
            self.retired.remove(&name);
            self.set.services.insert(name, svc);
        }
        Response::ok(format!(
            "reloaded: {added} added, {removed} removed (changes apply on next start)"
        ))
    }

    pub fn begin_shutdown(&mut self) {
        if self.shutting_down.is_some() {
            return;
        }
        self.say("stopping all services");
        self.shutting_down =
            Some(Instant::now() + Duration::from_millis(self.global.shutdown_timeout_ms));
        for u in self.units.values_mut() {
            u.wanted = false;
            u.restart_pending = false;
        }
    }

    fn all_stopped(&self) -> bool {
        self.units.values().all(|u| u.pid.is_none())
    }

    fn serve_client(&mut self, idx: usize) -> bool {
        let mut tmp = [0u8; 4096];
        let c = &mut self.clients[idx];
        loop {
            match c.stream.read(&mut tmp) {
                Ok(0) => return true,
                Ok(n) => {
                    c.buf.extend_from_slice(&tmp[..n]);
                    if c.buf.len() > 65536 {
                        return true;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => return true,
            }
        }
        let Some(nl) = c.buf.iter().position(|&b| b == b'\n') else {
            return false;
        };
        let line = c.buf[..nl].to_vec();
        let resp = match serde_json::from_slice::<Request>(&line) {
            Ok(req) => self.handle(req),
            Err(e) => Response::err(format!("bad request: {e}")),
        };
        let mut out = serde_json::to_vec(&resp).unwrap_or_default();
        out.push(b'\n');
        let c = &mut self.clients[idx];
        let _ = c.stream.set_nonblocking(false);
        let _ = c.stream.set_write_timeout(Some(Duration::from_secs(1)));
        let _ = c.stream.write_all(&out);
        true
    }

    // ---------------------------------------------------------------- main loop

    pub fn run(&mut self) -> i32 {
        loop {
            self.reconcile();
            if let Some(deadline) = self.shutting_down {
                if self.all_stopped() {
                    self.say("all services stopped");
                    let _ = std::fs::remove_file(crate::proto::socket_path(&self.paths.run_dir));
                    return 0;
                }
                if Instant::now() >= deadline {
                    self.say("shutdown timeout: killing remaining services");
                    for (pid, _) in self.pids.clone() {
                        Self::signal_group(pid, libc::SIGKILL);
                    }
                    self.shutting_down = Some(Instant::now() + Duration::from_secs(3600));
                }
            }

            let now = Instant::now();
            let mut next = self.units.values().filter_map(|u| u.timer).min();
            if let Some(d) = self.shutting_down {
                next = Some(next.map(|n| n.min(d)).unwrap_or(d));
            }
            let timeout = next.map(|t| t.saturating_duration_since(now));

            // Poll set: [sigfd, listener, clients..., notify..., out...]
            let mut fds = vec![
                PollFd {
                    fd: self.sigfd.as_raw_fd(),
                    readable: false,
                    hangup: false,
                },
                PollFd {
                    fd: self.listener.as_raw_fd(),
                    readable: false,
                    hangup: false,
                },
            ];
            for c in &self.clients {
                fds.push(PollFd {
                    fd: c.stream.as_raw_fd(),
                    readable: false,
                    hangup: false,
                });
            }
            let mut owners: Vec<(String, bool)> = Vec::new();
            for (name, u) in &self.units {
                if let Some(fd) = &u.notify {
                    fds.push(PollFd {
                        fd: fd.as_raw_fd(),
                        readable: false,
                        hangup: false,
                    });
                    owners.push((name.clone(), true));
                }
                if let Some(fd) = &u.out {
                    fds.push(PollFd {
                        fd: fd.as_raw_fd(),
                        readable: false,
                        hangup: false,
                    });
                    owners.push((name.clone(), false));
                }
            }
            if let Err(e) = sys::poll(&mut fds, timeout) {
                self.say(&format!("poll: {e}"));
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }

            if fds[0].readable {
                for sig in sys::read_signals(&self.sigfd) {
                    match sig {
                        libc::SIGCHLD => {}
                        libc::SIGTERM | libc::SIGINT => self.begin_shutdown(),
                        libc::SIGHUP => {
                            let r = self.reload();
                            self.say(r.message.as_deref().or(r.error.as_deref()).unwrap_or(""));
                        }
                        _ => {}
                    }
                }
                while let Some((pid, exit)) = sys::reap_any() {
                    self.on_exit(pid, exit);
                }
            }
            if fds[1].readable {
                while let Ok((s, _)) = self.listener.accept() {
                    let _ = s.set_nonblocking(true);
                    self.clients.push(Client {
                        stream: s,
                        buf: Vec::new(),
                    });
                }
            }
            let nclients = fds.len() - 2 - owners.len();
            let mut done = Vec::new();
            for i in 0..nclients {
                if (fds[2 + i].readable || fds[2 + i].hangup) && self.serve_client(i) {
                    done.push(i);
                }
            }
            for (k, (name, is_notify)) in owners.iter().enumerate() {
                let f = &fds[2 + nclients + k];
                if !(f.readable || f.hangup) {
                    continue;
                }
                let Some(u) = self.units.get_mut(name) else {
                    continue;
                };
                if *is_notify {
                    let mut buf = [0u8; 256];
                    let Some(fd) = u.notify.as_ref() else {
                        continue;
                    };
                    match sys::read(fd.as_raw_fd(), &mut buf) {
                        Ok(0) => u.notify = None,
                        Ok(n) => {
                            u.notify_buf.extend_from_slice(&buf[..n]);
                            if u.notify_buf.contains(&b'\n') {
                                u.notify = None;
                                if u.state == State::Starting {
                                    u.timer = None;
                                    u.set(State::Up);
                                    if let Some(l) = u.log.as_mut() {
                                        l.note("ready");
                                    }
                                }
                            }
                        }
                        Err(_) => {}
                    }
                } else {
                    Self::drain_output(u);
                }
            }
            for i in done.into_iter().rev() {
                self.clients.remove(i);
            }
        }
    }
}

/// CLOCK_BOOTTIME (ms) at which r6-init started, passed down via the environment.
pub fn init_start_ms() -> Option<f64> {
    std::env::var("R6_INIT_START_MS")
        .ok()
        .and_then(|v| v.parse().ok())
}
