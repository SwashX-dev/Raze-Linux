//! r6-init: PID 1.
//!
//! Responsibilities are deliberately few: mount the API filesystems, run the optional
//! `rc.init` stage, start `r6-supervise`, reap every orphan, restart the supervisor if
//! it dies, and drive shutdown. Everything else (udev, networking, gettys…) is a
//! supervised service.
//!
//! Signals (PID 1):
//!   SIGUSR1, SIGPWR  -> poweroff
//!   SIGUSR2          -> halt
//!   SIGTERM, SIGINT  -> reboot (SIGINT is Ctrl-Alt-Del)
//!
//! When not running as PID 1 (tests, containers), r6-init becomes a child subreaper and
//! exits instead of calling reboot(2). It never signals processes it did not start.

use crate::sys::{self, Halt, PollFd};
use std::collections::VecDeque;
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

const DEFAULT_PATH: &str =
    "/run/current-system/sw/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin";

fn log(msg: &str) {
    eprintln!("r6-init: {msg}");
}

struct Init {
    pid1: bool,
    /// signalfd for SIGCHLD and the halt signals (-1 if unavailable).
    sigfd: std::os::fd::RawFd,
    /// A halt signal received while waiting for a stage script.
    pending_halt: Option<Halt>,
    supervisor: PathBuf,
    sup_pid: Option<i32>,
    crashes: VecDeque<Instant>,
    respawn_at: Option<Instant>,
}

fn env_or(var: &str, default: &str) -> PathBuf {
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| default.into())
}

fn find_supervisor() -> PathBuf {
    if let Some(p) = std::env::var_os("R6_SUPERVISOR").filter(|v| !v.is_empty()) {
        return p.into();
    }
    if let Ok(exe) = std::env::current_exe() {
        let sib = exe.with_file_name("r6-supervise");
        if sib.exists() {
            return sib;
        }
    }
    "/run/current-system/sw/bin/r6-supervise".into()
}

/// Per-step progress messages go to the console only with `r6.verbose` on the kernel
/// command line (or R6_VERBOSE set): console writes are synchronous and slow boot.
pub fn verbose() -> bool {
    std::env::var_os("R6_VERBOSE").is_some() || kernel_cmdline_has("r6.verbose")
}

pub fn kernel_cmdline_has(flag: &str) -> bool {
    std::fs::read_to_string("/proc/cmdline")
        .map(|c| c.split_whitespace().any(|w| w == flag))
        .unwrap_or(false)
}

/// Mount the kernel API filesystems if they are not mounted yet.
fn mount_api_filesystems() {
    let mounts = std::fs::read_to_string("/proc/self/mounts").unwrap_or_default();
    let mounted = |t: &str| {
        mounts
            .lines()
            .any(|l| l.split_whitespace().nth(1) == Some(t))
    };
    let nosuid_noexec_nodev = libc::MS_NOSUID | libc::MS_NOEXEC | libc::MS_NODEV;
    let table: [(&str, &str, &str, libc::c_ulong, Option<&str>); 7] = [
        ("proc", "/proc", "proc", nosuid_noexec_nodev, None),
        ("sysfs", "/sys", "sysfs", nosuid_noexec_nodev, None),
        (
            "devtmpfs",
            "/dev",
            "devtmpfs",
            libc::MS_NOSUID,
            Some("mode=0755"),
        ),
        (
            "devpts",
            "/dev/pts",
            "devpts",
            libc::MS_NOSUID | libc::MS_NOEXEC,
            Some("gid=3,mode=0620,ptmxmode=0666"),
        ),
        (
            "tmpfs",
            "/dev/shm",
            "tmpfs",
            libc::MS_NOSUID | libc::MS_NODEV,
            Some("mode=1777"),
        ),
        (
            "tmpfs",
            "/run",
            "tmpfs",
            libc::MS_NOSUID | libc::MS_NODEV,
            Some("mode=0755"),
        ),
        (
            "cgroup2",
            "/sys/fs/cgroup",
            "cgroup2",
            nosuid_noexec_nodev,
            None,
        ),
    ];
    for (src, target, fstype, flags, data) in table {
        if mounted(target) {
            continue;
        }
        let _ = std::fs::create_dir_all(target);
        if let Err(e) = sys::mount(src, target, fstype, flags, data) {
            // /proc may not have been readable before it was mounted; EBUSY = already there.
            if e.raw_os_error() != Some(libc::EBUSY) {
                log(&format!("mount {target}: {e}"));
            }
        }
    }
}

fn spawn_with_reset(path: &Path, args: &[&str]) -> std::io::Result<i32> {
    let mut cmd = Command::new(path);
    cmd.args(args);
    // SAFETY: only async-signal-safe calls between fork and exec.
    unsafe {
        cmd.pre_exec(|| {
            sys::reset_signals_in_child();
            Ok(())
        });
    }
    let child = cmd.spawn()?;
    Ok(child.id() as i32)
}

impl Init {
    fn start_supervisor(&mut self) {
        match spawn_with_reset(&self.supervisor, &[]) {
            Ok(pid) => {
                self.sup_pid = Some(pid);
            }
            Err(e) => {
                log(&format!("cannot start {}: {e}", self.supervisor.display()));
                self.supervisor_died();
            }
        }
    }

    /// Respawn with backoff; after repeated crashes drop to an emergency shell.
    fn supervisor_died(&mut self) {
        self.sup_pid = None;
        let now = Instant::now();
        while self
            .crashes
            .front()
            .map(|t| now.duration_since(*t) > Duration::from_secs(60))
            .unwrap_or(false)
        {
            self.crashes.pop_front();
        }
        self.crashes.push_back(now);
        if self.crashes.len() >= 5 && self.pid1 {
            log("supervisor keeps failing; starting emergency shell (exit it to retry)");
            self.emergency_shell();
            self.crashes.clear();
        }
        let delay = Duration::from_millis(200 * (1 << self.crashes.len().min(5)));
        self.respawn_at = Some(now + delay);
    }

    fn emergency_shell(&mut self) {
        let shell = env_or("R6_EMERGENCY_SHELL", "/bin/sh");
        match spawn_with_reset(&shell, &[]) {
            Ok(pid) => {
                // Wait for the shell specifically, still reaping orphans meanwhile.
                loop {
                    let mut status = 0;
                    let r = unsafe { libc::waitpid(-1, &mut status, 0) };
                    if r == pid
                        || (r < 0
                            && std::io::Error::last_os_error().raw_os_error() == Some(libc::ECHILD))
                    {
                        break;
                    }
                }
            }
            Err(e) => log(&format!("emergency shell {}: {e}", shell.display())),
        }
    }

    /// Wait for a specific child to exit while reaping everything else, up to `timeout`.
    fn wait_for(&mut self, pid: i32, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            while let Some((p, _)) = sys::reap_any() {
                if p == pid {
                    return true;
                }
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            // Sleep until SIGCHLD (or a halt signal) arrives instead of polling on a timer.
            let wait = (deadline - now).min(Duration::from_millis(500));
            if self.sigfd >= 0 {
                let mut fds = [PollFd {
                    fd: self.sigfd,
                    readable: false,
                    hangup: false,
                }];
                let _ = sys::poll(&mut fds, Some(wait));
                if fds[0].readable {
                    let fd = unsafe { std::os::fd::BorrowedFd::borrow_raw(self.sigfd) };
                    for sig in sys::read_signals_borrowed(fd) {
                        if let Some(h) = halt_for(sig) {
                            self.pending_halt = Some(h);
                        }
                    }
                }
            } else {
                std::thread::sleep(wait.min(Duration::from_millis(10)));
            }
        }
    }

    fn run_stage(&mut self, var: &str, default: &str, timeout: Duration) {
        let script = env_or(var, default);
        if !script.exists() {
            return;
        }
        match spawn_with_reset(&script, &[]) {
            Ok(pid) => {
                if !self.wait_for(pid, timeout) {
                    log(&format!(
                        "{} did not finish within {:?}; continuing",
                        script.display(),
                        timeout
                    ));
                }
            }
            Err(e) => log(&format!("{}: {e}", script.display())),
        }
    }

    fn shutdown(&mut self, how: Halt) -> ! {
        log(&format!("shutdown requested ({how:?})"));
        if let Some(pid) = self.sup_pid {
            let _ = sys::kill(pid, libc::SIGTERM);
            let budget = crate::config::Global::load(&env_or("R6_CONF", "/etc/r6/r6.toml"))
                .map(|g| g.shutdown_timeout_ms)
                .unwrap_or(15_000)
                + 5_000;
            if !self.wait_for(pid, Duration::from_millis(budget)) {
                log("supervisor did not stop in time");
                let _ = sys::kill(pid, libc::SIGKILL);
                self.wait_for(pid, Duration::from_secs(2));
            }
        }
        self.run_stage(
            "R6_RC_SHUTDOWN",
            "/etc/r6/rc.shutdown",
            Duration::from_secs(30),
        );
        if !self.pid1 {
            log(&format!("not PID 1: exiting instead of {how:?}"));
            std::process::exit(0);
        }
        // Remaining processes: polite, then forceful.
        let _ = sys::kill(-1, libc::SIGTERM);
        let _ = sys::kill(-1, libc::SIGCONT);
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            while sys::reap_any().is_some() {}
            // kill(-1, 0) cannot be used here: it also "succeeds" for kernel threads.
            if !sys::user_processes_remaining() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = sys::kill(-1, libc::SIGKILL);
        while sys::reap_any().is_some() {}
        sys::sync();
        unmount_all();
        sys::sync();
        let e = sys::reboot(how);
        log(&format!("reboot(2) failed: {e}; halting in a loop"));
        loop {
            std::thread::sleep(Duration::from_secs(3600));
        }
    }
}

/// Unmount everything except kernel API filesystems, deepest first; remount / read-only.
fn unmount_all() {
    const PSEUDO: [&str; 12] = [
        "proc",
        "sysfs",
        "devtmpfs",
        "devpts",
        "cgroup2",
        "cgroup",
        "securityfs",
        "debugfs",
        "efivarfs",
        "bpf",
        "tracefs",
        "pstore",
    ];
    let mounts = std::fs::read_to_string("/proc/self/mounts").unwrap_or_default();
    let mut targets: Vec<(String, String)> = mounts
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let _src = it.next()?;
            let target = it.next()?.replace("\\040", " ");
            let fstype = it.next()?.to_string();
            Some((target, fstype))
        })
        .collect();
    targets.reverse();
    for (target, fstype) in &targets {
        if target == "/"
            || PSEUDO.contains(&fstype.as_str())
            || target == "/run"
            || target == "/dev/shm"
        {
            continue;
        }
        if let Err(e) = sys::umount_detach(target) {
            log(&format!("umount {target}: {e}"));
        }
    }
    if let Err(e) = sys::remount_ro("/") {
        log(&format!("remount / read-only: {e}"));
    }
}

fn halt_for(sig: i32) -> Option<Halt> {
    match sig {
        libc::SIGUSR1 | libc::SIGPWR => Some(Halt::Poweroff),
        libc::SIGUSR2 => Some(Halt::Halt),
        libc::SIGTERM | libc::SIGINT => Some(Halt::Reboot),
        _ => None,
    }
}

pub fn main() -> ! {
    let start_ms = sys::boottime_ms();
    let pid1 = sys::getpid() == 1;
    // SAFETY: single-threaded at this point.
    std::env::set_var("R6_INIT_START_MS", format!("{start_ms:.3}"));
    if pid1 {
        sys::ensure_stdio();
        unsafe { libc::umask(0o022) };
        if std::env::var_os("PATH").is_none() {
            std::env::set_var("PATH", DEFAULT_PATH);
        }
        mount_api_filesystems();
        sys::disable_cad();
        if let Ok(h) = std::fs::read_to_string("/etc/hostname") {
            let h = h.trim();
            if !h.is_empty() {
                let _ = sys::sethostname(h);
            }
        }
    } else {
        let _ = sys::set_child_subreaper();
    }

    let sigfd = match sys::signalfd(&[
        libc::SIGCHLD,
        libc::SIGTERM,
        libc::SIGINT,
        libc::SIGUSR1,
        libc::SIGUSR2,
        libc::SIGPWR,
        libc::SIGHUP,
    ]) {
        Ok(fd) => fd,
        Err(e) => {
            log(&format!("signalfd failed: {e}"));
            if !pid1 {
                std::process::exit(1);
            }
            // Without a signalfd we can still reap; poll on a timer below.
            loop {
                while sys::reap_any().is_some() {}
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    };

    let mut init = Init {
        pid1,
        sigfd: sigfd.as_raw_fd(),
        pending_halt: None,
        supervisor: find_supervisor(),
        sup_pid: None,
        crashes: VecDeque::new(),
        respawn_at: None,
    };

    if pid1 && kernel_cmdline_has("r6.emergency") {
        log("r6.emergency on kernel command line: starting emergency shell");
        init.emergency_shell();
    }
    init.run_stage("R6_RC_INIT", "/etc/r6/rc.init", Duration::from_secs(300));
    if verbose() {
        log(&format!(
            "started in {:.2} ms; launching supervisor",
            sys::boottime_ms() - start_ms
        ));
    }
    init.start_supervisor();

    loop {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let timeout = init
                .respawn_at
                .map(|t| t.saturating_duration_since(Instant::now()));
            let mut fds = [PollFd {
                fd: sigfd.as_raw_fd(),
                readable: false,
                hangup: false,
            }];
            let _ = sys::poll(&mut fds, timeout);
            let mut halt = init.pending_halt.take();
            if fds[0].readable {
                for sig in sys::read_signals(&sigfd) {
                    halt = halt_for(sig).or(halt);
                }
            }
            while let Some((pid, exit)) = sys::reap_any() {
                if Some(pid) == init.sup_pid {
                    log(&format!("supervisor exited ({})", exit.describe()));
                    init.supervisor_died();
                }
            }
            if let Some(how) = halt {
                init.shutdown(how);
            }
            if init.sup_pid.is_none()
                && init
                    .respawn_at
                    .map(|t| Instant::now() >= t)
                    .unwrap_or(false)
            {
                init.respawn_at = None;
                log("restarting supervisor");
                init.start_supervisor();
            }
        }));
        if r.is_err() {
            log("internal error (recovered); continuing");
        }
    }
}
