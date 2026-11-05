//! Thin, audited wrappers over the Linux syscalls r6 needs.
//! All `unsafe` in r6 lives in this module.

use std::ffi::CString;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::time::Duration;

fn cvt(r: libc::c_int) -> io::Result<libc::c_int> {
    if r < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(r)
    }
}

pub fn getpid() -> i32 {
    unsafe { libc::getpid() }
}

pub fn geteuid() -> u32 {
    unsafe { libc::geteuid() }
}

/// Block `sigs` for this thread and return a non-blocking, close-on-exec signalfd for them.
pub fn signalfd(sigs: &[i32]) -> io::Result<OwnedFd> {
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        for &s in sigs {
            libc::sigaddset(&mut set, s);
        }
        cvt(libc::pthread_sigmask(
            libc::SIG_BLOCK,
            &set,
            std::ptr::null_mut(),
        ))?;
        let fd = cvt(libc::signalfd(
            -1,
            &set,
            libc::SFD_NONBLOCK | libc::SFD_CLOEXEC,
        ))?;
        Ok(OwnedFd::from_raw_fd(fd))
    }
}

/// Read all pending signals from a signalfd.
pub fn read_signals(fd: &OwnedFd) -> Vec<i32> {
    let mut out = Vec::new();
    loop {
        let mut info: libc::signalfd_siginfo = unsafe { std::mem::zeroed() };
        let n = unsafe {
            libc::read(
                fd.as_raw_fd(),
                &mut info as *mut _ as *mut libc::c_void,
                std::mem::size_of::<libc::signalfd_siginfo>(),
            )
        };
        if n as usize != std::mem::size_of::<libc::signalfd_siginfo>() {
            break;
        }
        out.push(info.ssi_signo as i32);
    }
    out
}

/// Clear the signal mask and restore default dispositions. Async-signal-safe; used
/// between fork and exec.
pub fn reset_signals_in_child() {
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::pthread_sigmask(libc::SIG_SETMASK, &set, std::ptr::null_mut());
        for s in 1..32 {
            if s != libc::SIGKILL && s != libc::SIGSTOP {
                libc::signal(s, libc::SIG_DFL);
            }
        }
    }
}

pub enum Exit {
    Code(i32),
    Signal(i32),
}

impl Exit {
    pub fn success(&self) -> bool {
        matches!(self, Exit::Code(0))
    }
    pub fn describe(&self) -> String {
        match self {
            Exit::Code(c) => format!("exit {c}"),
            Exit::Signal(s) => format!("signal {s}"),
        }
    }
}

/// Reap one child without blocking. `None` when no child has exited.
pub fn reap_any() -> Option<(i32, Exit)> {
    let mut status = 0;
    let pid = unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG) };
    if pid <= 0 {
        return None;
    }
    let exit = if libc::WIFEXITED(status) {
        Exit::Code(libc::WEXITSTATUS(status))
    } else if libc::WIFSIGNALED(status) {
        Exit::Signal(libc::WTERMSIG(status))
    } else {
        Exit::Code(-1)
    };
    Some((pid, exit))
}

pub fn kill(pid: i32, sig: i32) -> io::Result<()> {
    cvt(unsafe { libc::kill(pid, sig) }).map(|_| ())
}

pub fn pipe() -> io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0; 2];
    cvt(unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) })?;
    unsafe { Ok((OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1]))) }
}

pub fn set_nonblocking(fd: RawFd) -> io::Result<()> {
    unsafe {
        let fl = cvt(libc::fcntl(fd, libc::F_GETFL))?;
        cvt(libc::fcntl(fd, libc::F_SETFL, fl | libc::O_NONBLOCK))?;
    }
    Ok(())
}

/// Non-blocking read. Ok(0) = EOF, Err(WouldBlock) = nothing available.
pub fn read(fd: RawFd, buf: &mut [u8]) -> io::Result<usize> {
    let n = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
    if n < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(n as usize)
    }
}

/// dup2 for use between fork and exec.
pub fn dup2(old: RawFd, new: RawFd) -> io::Result<()> {
    if old == new {
        // Clear CLOEXEC so the fd survives exec.
        unsafe {
            let fl = cvt(libc::fcntl(old, libc::F_GETFD))?;
            cvt(libc::fcntl(old, libc::F_SETFD, fl & !libc::FD_CLOEXEC))?;
        }
        return Ok(());
    }
    cvt(unsafe { libc::dup2(old, new) }).map(|_| ())
}

pub fn setsid() {
    unsafe {
        libc::setsid();
    }
}

pub struct PollFd {
    pub fd: RawFd,
    pub readable: bool,
    pub hangup: bool,
}

/// poll(2) for readability. Retries on EINTR is the caller's loop's job; we return empty.
pub fn poll(fds: &mut [PollFd], timeout: Option<Duration>) -> io::Result<usize> {
    let mut raw: Vec<libc::pollfd> = fds
        .iter()
        .map(|f| libc::pollfd {
            fd: f.fd,
            events: libc::POLLIN,
            revents: 0,
        })
        .collect();
    let ms = match timeout {
        None => -1,
        Some(d) => d.as_millis().min(i32::MAX as u128) as i32,
    };
    let r = unsafe { libc::poll(raw.as_mut_ptr(), raw.len() as libc::nfds_t, ms) };
    if r < 0 {
        let e = io::Error::last_os_error();
        if e.kind() == io::ErrorKind::Interrupted {
            return Ok(0);
        }
        return Err(e);
    }
    for (f, r) in fds.iter_mut().zip(raw.iter()) {
        f.readable = r.revents & libc::POLLIN != 0;
        f.hangup = r.revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0;
    }
    Ok(r as usize)
}

/// Milliseconds since boot (CLOCK_BOOTTIME), including time spent in the kernel before PID 1.
pub fn boottime_ms() -> f64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut ts) };
    ts.tv_sec as f64 * 1000.0 + ts.tv_nsec as f64 / 1e6
}

pub fn mount(
    src: &str,
    target: &str,
    fstype: &str,
    flags: libc::c_ulong,
    data: Option<&str>,
) -> io::Result<()> {
    let src = CString::new(src)?;
    let target = CString::new(target)?;
    let fstype = CString::new(fstype)?;
    let data = data.map(CString::new).transpose()?;
    cvt(unsafe {
        libc::mount(
            src.as_ptr(),
            target.as_ptr(),
            fstype.as_ptr(),
            flags,
            data.as_ref()
                .map(|d| d.as_ptr() as *const libc::c_void)
                .unwrap_or(std::ptr::null()),
        )
    })
    .map(|_| ())
}

pub fn remount_ro(target: &str) -> io::Result<()> {
    let target = CString::new(target)?;
    cvt(unsafe {
        libc::mount(
            std::ptr::null(),
            target.as_ptr(),
            std::ptr::null(),
            libc::MS_REMOUNT | libc::MS_RDONLY,
            std::ptr::null(),
        )
    })
    .map(|_| ())
}

pub fn umount_detach(target: &str) -> io::Result<()> {
    let target = CString::new(target)?;
    cvt(unsafe { libc::umount2(target.as_ptr(), libc::MNT_DETACH) }).map(|_| ())
}

pub fn sync() {
    unsafe { libc::sync() }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Halt {
    Poweroff,
    Reboot,
    Halt,
}

/// reboot(2). Only returns on failure.
pub fn reboot(how: Halt) -> io::Error {
    let cmd = match how {
        Halt::Poweroff => libc::RB_POWER_OFF,
        Halt::Reboot => libc::RB_AUTOBOOT,
        Halt::Halt => libc::RB_HALT_SYSTEM,
    };
    unsafe { libc::reboot(cmd) };
    io::Error::last_os_error()
}

/// Make Ctrl-Alt-Del send SIGINT to PID 1 instead of rebooting immediately.
pub fn disable_cad() {
    unsafe {
        libc::reboot(libc::RB_DISABLE_CAD);
    }
}

pub fn set_child_subreaper() -> io::Result<()> {
    cvt(unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) }).map(|_| ())
}

pub fn sethostname(name: &str) -> io::Result<()> {
    cvt(unsafe { libc::sethostname(name.as_ptr() as *const libc::c_char, name.len()) }).map(|_| ())
}

/// Resolve a user name to (uid, primary gid).
pub fn lookup_user(name: &str) -> Option<(u32, u32)> {
    if let Ok(uid) = name.parse::<u32>() {
        return Some((uid, uid));
    }
    let c = CString::new(name).ok()?;
    let mut pw: libc::passwd = unsafe { std::mem::zeroed() };
    let mut buf = vec![0 as libc::c_char; 16384];
    let mut res: *mut libc::passwd = std::ptr::null_mut();
    let r = unsafe { libc::getpwnam_r(c.as_ptr(), &mut pw, buf.as_mut_ptr(), buf.len(), &mut res) };
    if r != 0 || res.is_null() {
        return None;
    }
    Some((pw.pw_uid, pw.pw_gid))
}

pub fn lookup_group(name: &str) -> Option<u32> {
    if let Ok(gid) = name.parse::<u32>() {
        return Some(gid);
    }
    let c = CString::new(name).ok()?;
    let mut gr: libc::group = unsafe { std::mem::zeroed() };
    let mut buf = vec![0 as libc::c_char; 16384];
    let mut res: *mut libc::group = std::ptr::null_mut();
    let r = unsafe { libc::getgrnam_r(c.as_ptr(), &mut gr, buf.as_mut_ptr(), buf.len(), &mut res) };
    if r != 0 || res.is_null() {
        return None;
    }
    Some(gr.gr_gid)
}

/// Resident set size of this process in KiB (from /proc/self/statm).
pub fn rss_kib() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/self/statm").ok()?;
    let pages: u64 = s.split_whitespace().nth(1)?.parse().ok()?;
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as u64;
    Some(pages * page / 1024)
}

/// Open /dev/console (or /dev/null) onto stdin/stdout/stderr if they are closed.
pub fn ensure_stdio() {
    for fd in 0..3 {
        let ok = unsafe { libc::fcntl(fd, libc::F_GETFD) } >= 0;
        if !ok {
            let path = CString::new("/dev/console").unwrap();
            let mut f = unsafe { libc::open(path.as_ptr(), libc::O_RDWR | libc::O_NOCTTY) };
            if f < 0 {
                let null = CString::new("/dev/null").unwrap();
                f = unsafe { libc::open(null.as_ptr(), libc::O_RDWR) };
            }
            if f >= 0 && f != fd {
                unsafe {
                    libc::dup2(f, fd);
                    libc::close(f);
                }
            }
        }
    }
}

/// Supplementary groups of `user` (including `gid`). Call in the parent, not after fork.
pub fn grouplist(user: &str, gid: u32) -> Vec<u32> {
    let c = match CString::new(user) {
        Ok(c) => c,
        Err(_) => return vec![gid],
    };
    let mut n: libc::c_int = 64;
    loop {
        let mut groups = vec![0 as libc::gid_t; n as usize];
        let mut cnt = n;
        let r = unsafe { libc::getgrouplist(c.as_ptr(), gid, groups.as_mut_ptr(), &mut cnt) };
        if r >= 0 {
            groups.truncate(cnt as usize);
            return groups;
        }
        if cnt <= n || n > 65536 {
            return vec![gid];
        }
        n = cnt;
    }
}

/// setgroups + setgid + setuid. Async-signal-safe; used between fork and exec.
pub fn drop_privileges(groups: &[u32], gid: u32, uid: u32) -> io::Result<()> {
    cvt(unsafe { libc::setgroups(groups.len(), groups.as_ptr()) })?;
    cvt(unsafe { libc::setgid(gid) })?;
    cvt(unsafe { libc::setuid(uid) })?;
    Ok(())
}

/// Process start time (clock ticks since boot, field 22 of /proc/<pid>/stat).
/// Together with the pid this identifies a process across pid reuse.
pub fn proc_starttime(pid: i32) -> Option<u64> {
    let s = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let rest = &s[s.rfind(')')? + 2..];
    rest.split_whitespace().nth(19)?.parse().ok()
}

/// True if any userspace process other than the caller and PID 1 is still alive.
/// Kernel threads (PF_KTHREAD) are ignored; zombies count until they are reaped.
pub fn user_processes_remaining() -> bool {
    const PF_KTHREAD: u64 = 0x0020_0000;
    let me = getpid();
    let Ok(rd) = std::fs::read_dir("/proc") else {
        return false;
    };
    for e in rd.flatten() {
        let Some(pid) = e.file_name().to_str().and_then(|s| s.parse::<i32>().ok()) else {
            continue;
        };
        if pid == 1 || pid == me {
            continue;
        }
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            continue;
        };
        let Some(rest) = stat.rfind(')').map(|i| &stat[i + 2..]) else {
            continue;
        };
        // Fields after "comm": state(0) ppid(1) pgrp(2) session(3) tty(4) tpgid(5) flags(6)
        let flags: u64 = rest
            .split_whitespace()
            .nth(6)
            .and_then(|f| f.parse().ok())
            .unwrap_or(0);
        if flags & PF_KTHREAD == 0 {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    #[test]
    fn kernel_threads_are_not_user_processes() {
        // The test runner's own parent (cargo) is a user process, so this is true here;
        // the important property is that it does not panic on /proc parsing.
        let _ = super::user_processes_remaining();
        // kthreadd (pid 2) is visible on normal hosts and must be classified as a kthread.
        if let Ok(stat) = std::fs::read_to_string("/proc/2/stat") {
            if stat.contains("(kthreadd)") {
                let rest = &stat[stat.rfind(')').unwrap() + 2..];
                let flags: u64 = rest.split_whitespace().nth(6).unwrap().parse().unwrap();
                assert_ne!(flags & 0x0020_0000, 0);
            }
        }
    }
}
