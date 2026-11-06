//! r6 — a small, fast init and service supervision suite for Linux, inspired by s6.
//!
//! Components:
//! - `r6-init`: PID 1. Mounts API filesystems, reaps orphans, runs the supervisor,
//!   coordinates shutdown.
//! - `r6-supervise`: supervises every service from a single event loop.
//! - `r6-activate`: validates the service graph and prints the parallel start plan.
//! - `r6-log`: size-rotated, timestamped log writer (library + stdin filter).
//! - `r6`: control CLI (`r6 status`, `r6 start <svc>`, …).

pub mod activation;
pub mod config;
pub mod init;
pub mod log;
pub mod paths;
pub mod proto;
pub mod supervisor;
pub mod sys;
pub mod tmpfiles;
pub mod units;
