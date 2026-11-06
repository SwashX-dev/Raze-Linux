//! r6-import: translate the boot-time units of a NixOS systemd unit tree into r6
//! services (run at system build time; see `r6::units`).
//!
//!   r6-import --units DIR --services DIR --out DIR [--report FILE]
//!             [--timers FILE --timers-path RUNTIME-PATH]
//!             [--alias UNIT=SERVICE]... [--skip UNIT]... [--static-user NAME]...
//!             [--base-want SERVICE]...
//!
//! `--services` holds the hand-written r6 services; imported services never shadow
//! them. The combined set is validated; dependency cycles introduced by imported
//! ordering hints are broken by dropping those hints. With timers, a `timers` service
//! running `r6-timers RUNTIME-PATH` is added.

use r6::config::{ConfigError, ServiceSet};
use r6::units::{import, Context};
use std::path::PathBuf;

fn die(msg: &str) -> ! {
    eprintln!("r6-import: {msg}");
    std::process::exit(1);
}

fn main() {
    let mut args = std::env::args().skip(1);
    let (mut units, mut services, mut out, mut timers, mut report) = (None, None, None, None, None);
    let mut timers_path = None;
    let mut ctx = Context::default();
    while let Some(a) = args.next() {
        let mut val = || {
            args.next()
                .unwrap_or_else(|| die(&format!("{a} needs a value")))
        };
        match a.as_str() {
            "--units" => units = Some(PathBuf::from(val())),
            "--services" => services = Some(PathBuf::from(val())),
            "--out" => out = Some(PathBuf::from(val())),
            "--timers" => timers = Some(PathBuf::from(val())),
            "--report" => report = Some(PathBuf::from(val())),
            "--timers-path" => timers_path = Some(val()),
            "--alias" => {
                let v = val();
                let (u, s) = v
                    .split_once('=')
                    .unwrap_or_else(|| die("--alias UNIT=SERVICE"));
                ctx.aliases.insert(u.into(), s.into());
            }
            "--skip" => {
                ctx.skip.insert(val());
            }
            "--static-user" => {
                ctx.static_users.insert(val());
            }
            "--base-want" => ctx.base_wants.push(val()),
            "--version" => {
                println!("r6-import {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            _ => die(&format!("unknown argument '{a}'")),
        }
    }
    let units = units.unwrap_or_else(|| die("--units is required"));
    let services = services.unwrap_or_else(|| die("--services is required"));
    let out = out.unwrap_or_else(|| die("--out is required"));

    let existing = ServiceSet::load_dirs(std::slice::from_ref(&services))
        .unwrap_or_else(|e| die(&format!("hand-written services: {e}")));
    for (n, s) in &existing.services {
        ctx.existing.insert(n.clone());
        if s.enable {
            ctx.existing_enabled.insert(n.clone());
        }
    }
    let rep = import(&units, &ctx);
    std::fs::create_dir_all(&out).unwrap_or_else(|e| die(&format!("{}: {e}", out.display())));
    let mut texts: std::collections::BTreeMap<String, String> = rep
        .imported
        .iter()
        .map(|i| (i.name.clone(), i.toml.clone()))
        .collect();
    if let (Some(path), false) = (&timers_path, rep.timers.is_empty()) {
        let exe = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("r6-timers")))
            .unwrap_or_else(|| die("cannot locate r6-timers"));
        if ctx.existing.contains("timers") {
            die("a hand-written service is called 'timers'");
        }
        texts.insert(
            "timers".into(),
            format!(
                "description = \"Timers ({})\"\ncommand = [{}, {}]\nrestart = \"always\"\nwants = [\"filesystems\"]\nenable = true\n",
                rep.timers.iter().map(|t| t.name.as_str()).collect::<Vec<_>>().join(", "),
                serde_json::to_string(&exe.display().to_string()).unwrap(),
                serde_json::to_string(path).unwrap()
            ),
        );
    }
    let write = |texts: &std::collections::BTreeMap<String, String>| {
        for (n, t) in texts {
            std::fs::write(out.join(format!("{n}.toml")), t)
                .unwrap_or_else(|e| die(&format!("{n}: {e}")));
        }
    };
    write(&texts);
    // Validate the combined set; break cycles caused by imported ordering hints.
    let base = format!(
        "wants = [{}]",
        ctx.base_wants
            .iter()
            .map(|w| serde_json::to_string(w).unwrap())
            .collect::<Vec<_>>()
            .join(", ")
    );
    for _ in 0..50 {
        match ServiceSet::load_dirs(&[services.clone(), out.clone()]) {
            Ok(_) => break,
            Err(ConfigError::Cycle(c)) => {
                let mut changed = false;
                for n in &c {
                    if let Some(t) = texts.get_mut(n) {
                        let lines: Vec<String> = t
                            .lines()
                            .map(|l| {
                                if l.starts_with("wants = ") {
                                    base.clone()
                                } else {
                                    l.to_string()
                                }
                            })
                            .collect();
                        let new = lines.join("\n") + "\n";
                        if &new != t {
                            *t = new;
                            changed = true;
                        }
                    }
                }
                if !changed {
                    die(&format!("dependency cycle: {}", c.join(" -> ")));
                }
                eprintln!(
                    "r6-import: dropped ordering hints of {} (cycle)",
                    c.join(", ")
                );
                write(&texts);
            }
            Err(e) => die(&format!("imported services: {e}")),
        }
    }

    if let Some(t) = timers {
        let mut s = String::new();
        for tm in &rep.timers {
            s.push_str(&format!(
                "[[timer]]\nname = {}\nservice = {}\ninterval_s = {}\nboot_delay_s = {}\npersistent = {}\n\n",
                serde_json::to_string(&tm.name).unwrap(),
                serde_json::to_string(&tm.service).unwrap(),
                tm.interval_s,
                tm.boot_delay_s,
                tm.persistent
            ));
        }
        std::fs::write(&t, s).unwrap_or_else(|e| die(&format!("{}: {e}", t.display())));
    }
    let mut text = String::new();
    for i in &rep.imported {
        text.push_str(&format!("imported {}", i.name));
        if !i.notes.is_empty() {
            text.push_str(&format!(" ({})", i.notes.join("; ")));
        }
        text.push('\n');
    }
    for t in &rep.timers {
        text.push_str(&format!(
            "timer {} -> {} every {} s\n",
            t.name, t.service, t.interval_s
        ));
    }
    for (u, why) in &rep.unsupported {
        text.push_str(&format!("unsupported {u}: {why}\n"));
    }
    match report {
        Some(p) => {
            std::fs::write(&p, &text).unwrap_or_else(|e| die(&format!("{}: {e}", p.display())))
        }
        None => print!("{text}"),
    }
}
