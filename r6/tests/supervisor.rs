//! End-to-end tests: real r6-supervise / r6-init / r6 processes, unprivileged, in a
//! temporary directory tree.

use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

struct Env {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    sup: Option<Child>,
}

impl Env {
    fn new() -> Env {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        for d in ["services", "run", "state", "log", "out"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        Env {
            _tmp: tmp,
            root,
            sup: None,
        }
    }

    fn out(&self) -> PathBuf {
        self.root.join("out")
    }

    fn service(&self, name: &str, body: &str) {
        std::fs::write(
            self.root.join("services").join(format!("{name}.toml")),
            body,
        )
        .unwrap();
    }

    /// A long-running shell service that records its start in out/order.
    fn sh_service(&self, name: &str, script: &str, extra: &str) {
        let script = script.replace("$OUT", &self.out().display().to_string());
        self.service(
            name,
            &format!("command = [\"/bin/sh\", \"-c\", {:?}]\n{extra}\n", script),
        );
    }

    fn cmd(&self, bin: &str) -> Command {
        let mut c = Command::new(bin);
        c.env("R6_SERVICE_DIR", self.root.join("services"))
            .env("R6_RUN_DIR", self.root.join("run"))
            .env("R6_STATE_DIR", self.root.join("state"))
            .env("R6_LOG_DIR", self.root.join("log"))
            .env("R6_CONF", self.root.join("r6.toml"))
            .env("R6_QUIET", "1");
        c
    }

    fn start(&mut self) {
        let child = self
            .cmd(env!("CARGO_BIN_EXE_r6-supervise"))
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        self.sup = Some(child);
        wait_until(Duration::from_secs(5), || {
            self.root.join("run/control.sock").exists()
        });
    }

    fn r6(&self, args: &[&str]) -> Output {
        self.cmd(env!("CARGO_BIN_EXE_r6"))
            .args(args)
            .output()
            .unwrap()
    }

    fn state(&self, name: &str) -> String {
        let o = self.r6(&["--json", "status", name]);
        let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap_or_default();
        v[0]["state"].as_str().unwrap_or("?").to_string()
    }

    fn field(&self, name: &str, f: &str) -> serde_json::Value {
        let o = self.r6(&["--json", "status", name]);
        let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap_or_default();
        v[0][f].clone()
    }

    fn wait_state(&self, name: &str, want: &str, secs: u64) {
        let ok = wait_until(Duration::from_secs(secs), || self.state(name) == want);
        assert!(ok, "{name}: expected {want}, got {}", self.state(name));
    }

    fn read_out(&self, f: &str) -> String {
        std::fs::read_to_string(self.out().join(f)).unwrap_or_default()
    }

    fn stop_supervisor(&mut self) -> Option<i32> {
        let mut c = self.sup.take()?;
        unsafe { libc::kill(c.id() as i32, libc::SIGTERM) };
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(s) = c.try_wait().unwrap() {
                return s.code();
            }
            if Instant::now() > deadline {
                let _ = c.kill();
                panic!("supervisor did not exit");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        if let Some(mut c) = self.sup.take() {
            unsafe { libc::kill(c.id() as i32, libc::SIGTERM) };
            std::thread::sleep(Duration::from_millis(200));
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

fn wait_until(timeout: Duration, mut f: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    f()
}

const LONG: &str = "exec sleep 1000";

#[test]
fn boot_starts_enabled_services_in_dependency_order() {
    let mut e = Env::new();
    e.sh_service(
        "a",
        &format!("echo a >> $OUT/order; {LONG}"),
        "enable = false",
    );
    e.sh_service(
        "b",
        &format!("echo b >> $OUT/order; {LONG}"),
        "depends = [\"a\"]",
    );
    e.sh_service(
        "c",
        &format!("echo c >> $OUT/order; {LONG}"),
        "depends = [\"b\"]\nenable = true",
    );
    e.sh_service("unrelated", &format!("echo x >> $OUT/order; {LONG}"), "");
    e.start();
    e.wait_state("c", "up", 5);
    assert_eq!(e.state("unrelated"), "down");
    assert!(wait_until(Duration::from_secs(2), || e
        .read_out("order")
        .lines()
        .count()
        == 3));
    assert_eq!(e.read_out("order"), "a\nb\nc\n");
    assert!(wait_until(Duration::from_secs(2), || e
        .root
        .join("run/boot-complete")
        .exists()));
}

#[test]
fn notify_readiness_gates_dependents() {
    let mut e = Env::new();
    e.sh_service(
        "db",
        "sleep 0.4; echo ready-db >> $OUT/order; echo >&3; exec sleep 1000",
        "ready = \"notify\"",
    );
    e.sh_service(
        "app",
        &format!("echo app >> $OUT/order; {LONG}"),
        "depends = [\"db\"]\nenable = true",
    );
    e.start();
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(e.state("db"), "starting");
    assert_eq!(e.state("app"), "down");
    e.wait_state("app", "up", 5);
    assert_eq!(e.read_out("order"), "ready-db\napp\n");
}

#[test]
fn readiness_timeout_counts_as_failure() {
    let mut e = Env::new();
    e.sh_service(
        "never",
        LONG,
        "ready = \"notify\"\nready_timeout_ms = 200\nrestart = \"never\"\nenable = true",
    );
    e.start();
    e.wait_state("never", "failed", 5);
}

#[test]
fn crash_loop_hits_restart_limit() {
    let mut e = Env::new();
    e.sh_service(
        "crashy",
        "echo run >> $OUT/runs; exit 3",
        "restart = \"on-failure\"\nrestart_max = 3\nrestart_delay_ms = 10\nenable = true",
    );
    e.start();
    e.wait_state("crashy", "failed", 10);
    assert_eq!(
        e.read_out("runs").lines().count(),
        4,
        "1 start + 3 restarts"
    );
    let failure = e.field("crashy", "failure");
    assert!(
        failure.as_str().unwrap().contains("restart limit"),
        "{failure}"
    );
    // An explicit start clears the failure and tries again.
    let o = e.r6(&["start", "--no-block", "crashy"]);
    assert!(o.status.success());
    assert!(wait_until(Duration::from_secs(5), || e
        .read_out("runs")
        .lines()
        .count()
        > 4));
}

#[test]
fn always_restart_recovers_from_kill() {
    let mut e = Env::new();
    e.sh_service(
        "svc",
        LONG,
        "restart = \"always\"\nrestart_delay_ms = 10\nenable = true",
    );
    e.start();
    e.wait_state("svc", "up", 5);
    let pid1 = e.field("svc", "pid").as_i64().unwrap();
    unsafe { libc::kill(pid1 as i32, libc::SIGKILL) };
    assert!(wait_until(Duration::from_secs(5), || {
        e.field("svc", "pid")
            .as_i64()
            .map(|p| p != pid1)
            .unwrap_or(false)
            && e.state("svc") == "up"
    }));
    assert_eq!(e.field("svc", "restarts").as_u64(), Some(1));
}

#[test]
fn dependency_failure_propagates() {
    let mut e = Env::new();
    // base dies before signalling readiness, so dependents must never start.
    e.sh_service(
        "base",
        "sleep 0.1; exit 1",
        "ready = \"notify\"\nrestart = \"never\"",
    );
    e.sh_service("top", LONG, "depends = [\"base\"]\nenable = true");
    e.service(
        "noexec",
        "command = [\"/nonexistent/binary\"]\nrestart = \"never\"",
    );
    e.sh_service("top2", LONG, "depends = [\"noexec\"]\nenable = true");
    e.sh_service("soft", LONG, "wants = [\"base\"]\nenable = true");
    e.start();
    e.wait_state("top", "failed", 5);
    assert!(e
        .field("top", "failure")
        .as_str()
        .unwrap()
        .contains("dependency 'base'"));
    e.wait_state("top2", "failed", 5);
    assert!(e
        .field("noexec", "failure")
        .as_str()
        .unwrap()
        .contains("exec /nonexistent/binary"));
    assert_eq!(e.field("top", "pid"), serde_json::Value::Null);
    // Soft dependency failures are tolerated.
    e.wait_state("soft", "up", 5);
}

#[test]
fn oneshot_completes_before_dependents() {
    let mut e = Env::new();
    e.sh_service(
        "setup",
        "sleep 0.2; echo setup >> $OUT/order",
        "type = \"oneshot\"",
    );
    e.sh_service(
        "svc",
        &format!("echo svc >> $OUT/order; {LONG}"),
        "depends = [\"setup\"]\nenable = true",
    );
    e.start();
    e.wait_state("svc", "up", 5);
    assert_eq!(e.state("setup"), "done");
    assert_eq!(e.read_out("order"), "setup\nsvc\n");
}

#[test]
fn stop_takes_dependents_down_first() {
    let mut e = Env::new();
    let trap = |n: &str| {
        format!("trap 'echo stop-{n} >> $OUT/order; exit 0' TERM; while :; do sleep 0.05; done")
    };
    e.sh_service("a", &trap("a"), "");
    e.sh_service("b", &trap("b"), "depends = [\"a\"]");
    e.sh_service("c", &trap("c"), "depends = [\"b\"]\nenable = true");
    e.start();
    e.wait_state("c", "up", 5);
    let o = e.r6(&["stop", "a"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    for n in ["a", "b", "c"] {
        e.wait_state(n, "down", 5);
    }
    assert_eq!(e.read_out("order"), "stop-c\nstop-b\nstop-a\n");
}

#[test]
fn stop_timeout_escalates_to_sigkill() {
    let mut e = Env::new();
    e.sh_service(
        "stubborn",
        "trap '' TERM; while :; do sleep 0.05; done",
        "stop_timeout_ms = 300\nenable = true",
    );
    e.start();
    e.wait_state("stubborn", "up", 5);
    let t = Instant::now();
    assert!(e.r6(&["stop", "stubborn"]).status.success());
    let el = t.elapsed();
    assert!(
        el >= Duration::from_millis(280) && el < Duration::from_secs(3),
        "{el:?}"
    );
    assert_eq!(e.field("stubborn", "last_exit").as_str(), Some("signal 9"));
}

#[test]
fn restart_command_replaces_process() {
    let mut e = Env::new();
    e.sh_service("svc", LONG, "enable = true");
    e.start();
    e.wait_state("svc", "up", 5);
    let before = e.field("svc", "pid").as_i64().unwrap();
    let o = e.r6(&["restart", "svc"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(wait_until(Duration::from_secs(5), || {
        e.state("svc") == "up"
            && e.field("svc", "pid")
                .as_i64()
                .map(|p| p != before)
                .unwrap_or(false)
    }));
}

#[test]
fn invalid_configuration_is_rejected() {
    let e = Env::new();
    e.service("bad", "command = [\"relative\"]");
    let o = e.cmd(env!("CARGO_BIN_EXE_r6-supervise")).output().unwrap();
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("absolute path"));

    let e = Env::new();
    e.service("x", "command = [\"/bin/true\"]\ndepends = [\"y\"]");
    e.service("y", "command = [\"/bin/true\"]\ndepends = [\"x\"]");
    let o = e
        .cmd(env!("CARGO_BIN_EXE_r6"))
        .arg("check")
        .output()
        .unwrap();
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("cycle"));
    let o = e.cmd(env!("CARGO_BIN_EXE_r6-activate")).output().unwrap();
    assert!(!o.status.success());
}

#[test]
fn reload_adds_services_and_rejects_bad_config() {
    let mut e = Env::new();
    e.sh_service("one", LONG, "enable = true");
    e.start();
    e.wait_state("one", "up", 5);
    e.sh_service("two", LONG, "");
    let o = e.r6(&["reload"]);
    assert!(o.status.success());
    assert!(e.r6(&["start", "two"]).status.success());
    assert_eq!(e.state("two"), "up");
    e.service("broken", "command = []");
    let o = e.r6(&["reload"]);
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("keeping current"));
    assert_eq!(e.state("one"), "up");
    // Removing a running service stops it.
    std::fs::remove_file(e.root.join("services/broken.toml")).unwrap();
    std::fs::remove_file(e.root.join("services/two.toml")).unwrap();
    assert!(e.r6(&["reload"]).status.success());
    assert!(wait_until(Duration::from_secs(5), || !e
        .r6(&["status", "two"])
        .status
        .success()));
}

#[test]
fn enable_persists_across_supervisor_restarts() {
    let mut e = Env::new();
    e.sh_service("svc", LONG, "");
    e.start();
    assert_eq!(e.state("svc"), "down");
    assert!(e.r6(&["enable", "svc", "--now"]).status.success());
    e.wait_state("svc", "up", 5);
    assert_eq!(e.stop_supervisor(), Some(0));
    e.start();
    e.wait_state("svc", "up", 5);
    assert!(e.r6(&["disable", "svc"]).status.success());
    assert_eq!(e.field("svc", "enabled").as_bool(), Some(false));
    assert_eq!(e.stop_supervisor(), Some(0));
    e.start();
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(e.state("svc"), "down");
}

#[test]
fn service_output_is_logged() {
    let mut e = Env::new();
    e.sh_service(
        "talker",
        "echo hello-out; echo hello-err >&2; exec sleep 1000",
        "enable = true",
    );
    e.start();
    e.wait_state("talker", "up", 5);
    assert!(wait_until(Duration::from_secs(3), || {
        let o = e.r6(&["logs", "talker", "-n", "10"]);
        let s = String::from_utf8_lossy(&o.stdout).to_string();
        s.contains("hello-out") && s.contains("hello-err") && s.contains("r6: started")
    }));
}

#[test]
fn shutdown_stops_in_reverse_order_and_exits_cleanly() {
    let mut e = Env::new();
    let trap = |n: &str| {
        format!("trap 'echo stop-{n} >> $OUT/order; exit 0' TERM; while :; do sleep 0.05; done")
    };
    e.sh_service("a", &trap("a"), "");
    e.sh_service("b", &trap("b"), "depends = [\"a\"]\nenable = true");
    e.start();
    e.wait_state("b", "up", 5);
    assert_eq!(e.stop_supervisor(), Some(0));
    assert_eq!(e.read_out("order"), "stop-b\nstop-a\n");
}

#[test]
fn many_services_start_concurrently() {
    let mut e = Env::new();
    for i in 0..50 {
        e.sh_service(
            &format!("s{i}"),
            "sleep 0.3; echo >&3; exec sleep 1000",
            "ready = \"notify\"\nenable = true",
        );
    }
    let t = Instant::now();
    e.start();
    assert!(wait_until(Duration::from_secs(10), || e
        .root
        .join("run/boot-complete")
        .exists()));
    // 50 services each taking 300 ms to become ready: parallel activation must finish far
    // below the 15 s a serial start would take.
    assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
}

fn init_env(e: &Env) -> Command {
    let mut c = e.cmd(env!("CARGO_BIN_EXE_r6-init"));
    c.env("R6_SUPERVISOR", env!("CARGO_BIN_EXE_r6-supervise"))
        .env("R6_RC_INIT", e.root.join("rc.init"))
        .env("R6_RC_SHUTDOWN", e.root.join("rc.shutdown"))
        .stderr(Stdio::null());
    c
}

fn supervisor_pid(e: &Env) -> Option<i64> {
    let o = e.r6(&["--json", "info"]);
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).ok()?;
    v["pid"].as_i64()
}

#[test]
fn init_runs_stages_restarts_supervisor_and_shuts_down() {
    let e = Env::new();
    let out = e.out();
    std::fs::write(
        e.root.join("rc.init"),
        format!("#!/bin/sh\necho init >> {}/stages\n", out.display()),
    )
    .unwrap();
    std::fs::write(
        e.root.join("rc.shutdown"),
        format!("#!/bin/sh\necho shutdown >> {}/stages\n", out.display()),
    )
    .unwrap();
    for f in ["rc.init", "rc.shutdown"] {
        Command::new("chmod")
            .arg("+x")
            .arg(e.root.join(f))
            .status()
            .unwrap();
    }
    e.sh_service(
        "svc",
        "trap 'echo svc-stopped >> $OUT/stages; exit 0' TERM; while :; do sleep 0.05; done",
        "enable = true",
    );
    let mut init = init_env(&e).spawn().unwrap();
    e.wait_state("svc", "up", 5);
    assert_eq!(e.read_out("stages"), "init\n");

    // Kill the supervisor: init must bring it back.
    let sup = supervisor_pid(&e).unwrap();
    unsafe { libc::kill(sup as i32, libc::SIGKILL) };
    assert!(wait_until(Duration::from_secs(10), || supervisor_pid(&e)
        .map(|p| p != sup)
        .unwrap_or(false)));

    // The orphaned instance left by the killed supervisor is terminated by the new
    // supervisor (pidfile + start-time check) before a fresh instance starts.
    e.wait_state("svc", "up", 5);
    assert!(wait_until(Duration::from_secs(5), || e
        .read_out("stages")
        .contains("svc-stopped")));
    let running = Command::new("pgrep")
        .arg("-f")
        .arg(e.root.join("out").display().to_string())
        .output();
    if let Ok(o) = running {
        let n = String::from_utf8_lossy(&o.stdout).lines().count();
        assert!(n <= 1, "duplicate service instances: {n}");
    }

    unsafe { libc::kill(init.id() as i32, libc::SIGTERM) };
    let ok = wait_until(Duration::from_secs(20), || {
        init.try_wait().unwrap().is_some()
    });
    assert!(ok, "init did not exit");
    assert!(init.wait().unwrap().success());
    let stages = e.read_out("stages");
    assert!(stages.starts_with("init\n"), "{stages}");
    assert_eq!(stages, "init\nsvc-stopped\nsvc-stopped\nshutdown\n");
}

#[test]
fn info_reports_memory_and_timing() {
    let mut e = Env::new();
    e.sh_service("svc", LONG, "enable = true");
    e.start();
    e.wait_state("svc", "up", 5);
    assert!(wait_until(Duration::from_secs(2), || {
        let o = e.r6(&["--json", "info"]);
        let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap_or_default();
        v["rss_kib"].as_u64().unwrap_or(0) > 0 && v["boot_complete_ms"].is_number()
    }));
}
