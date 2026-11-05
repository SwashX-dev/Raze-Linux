//! r6: control CLI for the r6 service supervisor (r6-control).

use r6::paths::Paths;
use r6::proto::{call, Request, Response, ServiceStatus};
use std::path::Path;
use std::time::{Duration, Instant};

const USAGE: &str = "usage: r6 [--json] <command> [args]

commands:
  status [SERVICE]          show service state (all services if none given)
  list                      list services, one per line
  start SERVICE...          start services (and their dependencies), wait until ready
  stop SERVICE...           stop services (and their dependents)
  restart SERVICE...        restart services
  enable SERVICE [--now]    start at boot (persistent); --now also starts it
  disable SERVICE [--now]   do not start at boot; --now also stops it
  logs SERVICE [-n N] [-f]  show the last N log lines (default 50); -f follows
  reload                    re-read service definitions
  check                     validate service definitions without a running supervisor
  info                      supervisor statistics and boot timing
  reboot | poweroff | halt  shut the system down via r6-init

options:
  --no-block                for start/stop/restart: do not wait
  --json                    machine-readable output";

fn die(msg: &str) -> ! {
    eprintln!("r6: {msg}");
    std::process::exit(1);
}

fn request(paths: &Paths, req: &Request) -> Response {
    match call(&paths.run_dir, req) {
        Ok(r) => r,
        Err(e) => die(&format!(
            "cannot reach supervisor at {}: {e}",
            r6::proto::socket_path(&paths.run_dir).display()
        )),
    }
}

fn fmt_dur(ms: u64) -> String {
    let s = ms / 1000;
    if s < 60 {
        format!("{}.{:01}s", s, (ms % 1000) / 100)
    } else if s < 3600 {
        format!("{}m{}s", s / 60, s % 60)
    } else if s < 86400 {
        format!("{}h{}m", s / 3600, (s % 3600) / 60)
    } else {
        format!("{}d{}h", s / 86400, (s % 86400) / 3600)
    }
}

fn print_table(services: &[ServiceStatus]) {
    let w = services
        .iter()
        .map(|s| s.name.len())
        .max()
        .unwrap_or(4)
        .max(7);
    println!(
        "{:<w$}  {:<9} {:<8} {:>7}  {:>8}  {:>3}  INFO",
        "SERVICE", "STATE", "ENABLED", "PID", "SINCE", "RST"
    );
    for s in services {
        let info = s
            .failure
            .clone()
            .or_else(|| s.last_exit.clone().map(|e| format!("last: {e}")))
            .unwrap_or_default();
        println!(
            "{:<w$}  {:<9} {:<8} {:>7}  {:>8}  {:>3}  {}",
            s.name,
            s.state,
            if s.enabled { "yes" } else { "no" },
            s.pid.map(|p| p.to_string()).unwrap_or_else(|| "-".into()),
            fmt_dur(s.since_ms),
            s.restarts,
            info
        );
    }
}

fn print_detail(s: &ServiceStatus) {
    println!(
        "● {} — {}",
        s.name,
        if s.description.is_empty() {
            "(no description)"
        } else {
            &s.description
        }
    );
    println!("   state:    {} for {}", s.state, fmt_dur(s.since_ms));
    println!("   type:     {}", s.kind);
    println!("   enabled:  {}", if s.enabled { "yes" } else { "no" });
    println!("   wanted:   {}", if s.wanted { "yes" } else { "no" });
    if let Some(p) = s.pid {
        println!("   pid:      {p}");
    }
    println!("   restarts: {}", s.restarts);
    if let Some(e) = &s.last_exit {
        println!("   last:     {e}");
    }
    if let Some(f) = &s.failure {
        println!("   failure:  {f}");
    }
}

fn wait_for(paths: &Paths, name: &str, want: &[&str], timeout: Duration) -> Result<String, String> {
    let deadline = Instant::now() + timeout;
    loop {
        let r = request(
            paths,
            &Request::Status {
                service: Some(name.into()),
            },
        );
        let s = r
            .services
            .first()
            .ok_or_else(|| r.error.unwrap_or_default())?;
        if want.contains(&s.state.as_str()) {
            return Ok(s.state.clone());
        }
        if s.state == "failed" && !want.contains(&"failed") {
            return Err(format!(
                "{name} failed: {}",
                s.failure.clone().unwrap_or_default()
            ));
        }
        if Instant::now() >= deadline {
            return Err(format!("timed out waiting for {name} (state: {})", s.state));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn signal_init(sig: i32) -> ! {
    let pid: i32 = std::env::var("R6_INIT_PID")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    match r6::sys::kill(pid, sig) {
        Ok(()) => std::process::exit(0),
        Err(e) => die(&format!("cannot signal init (pid {pid}): {e}")),
    }
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut flag = |f: &str| {
        let had = args.iter().any(|a| a == f);
        args.retain(|a| a != f);
        had
    };
    let json = flag("--json");
    let no_block = flag("--no-block");
    let now = flag("--now");
    let follow = flag("-f");
    if flag("--help") || flag("-h") {
        println!("{USAGE}");
        return;
    }
    if flag("--version") {
        println!("r6 {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let paths = Paths::from_env();
    let cmd = args.first().cloned().unwrap_or_else(|| "status".into());
    let rest: Vec<String> = args.iter().skip(1).cloned().collect();
    let need = |n: usize| {
        if rest.len() < n {
            die(&format!("'{cmd}' needs a service name\n\n{USAGE}"));
        }
    };

    match cmd.as_str() {
        "status" | "list" => {
            let r = request(
                &paths,
                &Request::Status {
                    service: rest.first().cloned(),
                },
            );
            if !r.ok {
                die(&r.error.unwrap_or_default());
            }
            if json {
                println!("{}", serde_json::to_string_pretty(&r.services).unwrap());
            } else if cmd == "list" {
                for s in &r.services {
                    println!(
                        "{} {} {}",
                        s.name,
                        s.state,
                        if s.enabled { "enabled" } else { "disabled" }
                    );
                }
            } else if rest.is_empty() {
                print_table(&r.services);
            } else {
                print_detail(&r.services[0]);
            }
            let failed = !rest.is_empty() && r.services.iter().any(|s| s.state == "failed");
            std::process::exit(if failed { 3 } else { 0 });
        }
        "start" | "stop" | "restart" => {
            need(1);
            let mut code = 0;
            for name in &rest {
                let req = match cmd.as_str() {
                    "start" => Request::Start {
                        service: name.clone(),
                    },
                    "stop" => Request::Stop {
                        service: name.clone(),
                    },
                    _ => Request::Restart {
                        service: name.clone(),
                    },
                };
                let r = request(&paths, &req);
                if !r.ok {
                    eprintln!("r6: {}", r.error.unwrap_or_default());
                    code = 1;
                    continue;
                }
                if no_block {
                    continue;
                }
                if cmd == "restart" {
                    // Let the stop take effect before waiting for "up".
                    std::thread::sleep(Duration::from_millis(30));
                }
                let target: &[&str] = if cmd == "stop" {
                    &["down", "failed", "done"]
                } else {
                    &["up", "done"]
                };
                match wait_for(&paths, name, target, Duration::from_secs(90)) {
                    Ok(state) => println!("{name}: {state}"),
                    Err(e) => {
                        eprintln!("r6: {e}");
                        code = 1;
                    }
                }
            }
            std::process::exit(code);
        }
        "enable" | "disable" => {
            need(1);
            let req = if cmd == "enable" {
                Request::Enable {
                    service: rest[0].clone(),
                    now,
                }
            } else {
                Request::Disable {
                    service: rest[0].clone(),
                    now,
                }
            };
            let r = request(&paths, &req);
            if !r.ok {
                die(&r.error.unwrap_or_default());
            }
            println!("{}", r.message.unwrap_or_default());
        }
        "reload" => {
            let r = request(&paths, &Request::Reload);
            if !r.ok {
                die(&r.error.unwrap_or_default());
            }
            println!("{}", r.message.unwrap_or_default());
        }
        "info" => {
            let r = request(&paths, &Request::Info);
            let i = r.info.unwrap_or_default();
            if json {
                println!("{}", serde_json::to_string_pretty(&i).unwrap());
                return;
            }
            println!("supervisor pid:     {}", i.pid);
            println!(
                "supervisor RSS:     {} KiB",
                i.rss_kib.map(|v| v.to_string()).unwrap_or("?".into())
            );
            println!(
                "services:           {} defined, {} running",
                i.services, i.running
            );
            if let Some(init) = i.init_start_ms {
                println!("kernel -> r6-init:  {:.1} ms", init);
                println!("r6-init -> superv.: {:.1} ms", i.supervisor_start_ms - init);
            }
            if let Some(done) = i.boot_complete_ms {
                println!("superv. -> booted:  {:.1} ms", done - i.supervisor_start_ms);
                println!("total to services:  {:.1} ms since kernel start", done);
            } else {
                println!("boot:               still activating");
            }
        }
        "logs" => {
            need(1);
            let mut n = 50usize;
            if let Some(pos) = rest.iter().position(|a| a == "-n") {
                n = rest.get(pos + 1).and_then(|v| v.parse().ok()).unwrap_or(n);
            }
            let global = r6::config::Global::load(&paths.global_conf).unwrap_or_default();
            let dir = paths
                .log_dir_override
                .clone()
                .unwrap_or(global.log_dir)
                .join(&rest[0]);
            let show = |n: usize| {
                r6::log::tail(Path::new(&dir), n)
                    .unwrap_or_else(|e| die(&format!("{}: {e}", dir.display())))
            };
            let mut last = show(n);
            for l in &last {
                println!("{l}");
            }
            if !follow {
                return;
            }
            loop {
                std::thread::sleep(Duration::from_millis(250));
                let cur = show(200);
                // Print lines after the last one we have shown.
                let start = match last.last() {
                    Some(l) => cur.iter().rposition(|x| x == l).map(|p| p + 1).unwrap_or(0),
                    None => 0,
                };
                for l in &cur[start..] {
                    println!("{l}");
                }
                if start < cur.len() {
                    last = cur;
                }
            }
        }
        "check" => match r6::config::ServiceSet::load_dir(&paths.service_dir) {
            Ok(s) => println!(
                "ok: {} service(s) in {}",
                s.services.len(),
                paths.service_dir.display()
            ),
            Err(e) => die(&format!("invalid configuration: {e}")),
        },
        "poweroff" => signal_init(libc::SIGUSR1),
        "halt" => signal_init(libc::SIGUSR2),
        "reboot" => signal_init(libc::SIGTERM),
        other => die(&format!("unknown command '{other}'\n\n{USAGE}")),
    }
}
