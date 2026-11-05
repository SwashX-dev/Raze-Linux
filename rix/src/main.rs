//! rix: the Raze Linux package manager CLI.
//!
//! Native (Rust): search index, installed-package listing, generations, rollback,
//! NAR/file hashing, store-path verification, flake.lock inspection, diagnostics.
//! Delegated to Nix (printed as `rix: → nix …`): evaluation, building, substitution,
//! profile installs/removals/upgrades, flake updates, garbage collection, system builds.

use rix::{flakelock, index, nar, nix32, profile, store};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

const SYSTEM_FLAKE: &str = "/etc/nixos";
/// The package source: the `nixpkgs` registry entry (pinned on Raze systems), or
/// any flake reference given in RIX_NIXPKGS.
fn pkgs_ref() -> String {
    std::env::var("RIX_NIXPKGS")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "nixpkgs".into())
}

const USAGE: &str = "usage: rix <command> [args]

packages (user profile):
  search TERM...            search packages (native index; built on first use)
  info PACKAGE              show package details from the index
  install PACKAGE...        install packages            [delegated: nix profile add]
  remove PACKAGE...         remove installed packages   [delegated: nix profile remove]
  list                      list installed packages     (native)
  upgrade                   upgrade installed packages  [delegated: nix profile upgrade]
  update                    rebuild the search index for the pinned nixpkgs
  generations               list profile generations    (native)
  rollback [--to N]         switch to the previous (or N-th) generation (native, atomic)

system (needs root):
  system switch|boot|test   rebuild the system from /etc/nixos   [delegated: nixos-rebuild]
  system generations        list system generations (native)
  system rollback [--to N]  boot the previous system generation now (native switch)
  system update             update /etc/nixos/flake.lock          [delegated: nix flake update]

development:
  build|develop|shell|run ARGS...    [delegated: nix build|develop|shell|run]
  flake update|lock [ARGS]           [delegated]
  flake info [DIR]                   show locked inputs (native)

store:
  gc [--older-than 30d]     collect garbage             [delegated: nix-collect-garbage]
  hash path|file PATH...    Nix-compatible SHA-256 (native; --sri for SRI form)
  verify STOREPATH...       re-hash store paths and compare with the Nix database (native hashing)
  doctor                    check the Nix setup (native)

options: --json (search, list, generations), --limit N (search), --verbose";

fn eprint_delegate(cmd: &[String]) {
    eprintln!("\x1b[2mrix: → {}\x1b[0m", cmd.join(" "));
}

fn nix_bin(name: &str) -> String {
    std::env::var("RIX_NIX_BIN_DIR")
        .map(|d| format!("{d}/{name}"))
        .unwrap_or_else(|_| name.to_string())
}

/// Run a delegated command with inherited stdio; return its exit code.
fn delegate(prog: &str, args: &[String]) -> ExitCode {
    let mut full = vec![prog.to_string()];
    full.extend(args.iter().cloned());
    eprint_delegate(&full);
    match Command::new(nix_bin(prog)).args(args).status() {
        Ok(s) => ExitCode::from(s.code().unwrap_or(1).clamp(0, 255) as u8),
        Err(e) => die(&format!("cannot run {prog}: {e}")),
    }
}

fn capture(prog: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(nix_bin(prog))
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::inherit())
        .output()
        .map_err(|e| format!("cannot run {prog}: {e}"))?;
    if !out.status.success() {
        return Err(format!("{prog} {} failed", args.join(" ")));
    }
    String::from_utf8(out.stdout).map_err(|e| e.to_string())
}

fn capture_quiet(prog: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(nix_bin(prog))
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!("{prog} failed"));
    }
    String::from_utf8(out.stdout).map_err(|e| e.to_string())
}

fn die(msg: &str) -> ! {
    eprintln!("rix: {msg}");
    std::process::exit(1);
}

/// Identify the nixpkgs the registry resolves to (its narHash), cheaply.
fn nixpkgs_key() -> Result<String, String> {
    let out = capture(
        "nix",
        &[
            "flake",
            "metadata",
            &pkgs_ref(),
            "--json",
            "--no-write-lock-file",
        ],
    )?;
    let v: serde_json::Value = serde_json::from_str(&out).map_err(|e| e.to_string())?;
    v["locked"]["narHash"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| "nixpkgs has no narHash".into())
}

fn build_index(key: &str) -> Result<index::Index, String> {
    eprintln!("rix: building package index for {} ({key}); this evaluates nixpkgs once and takes a minute…", pkgs_ref());
    eprint_delegate(&[
        "nix".into(),
        "search".into(),
        "--json".into(),
        pkgs_ref(),
        "^".into(),
    ]);
    let json = capture(
        "nix",
        &["search", "--json", "--no-write-lock-file", &pkgs_ref(), "^"],
    )?;
    let idx = index::Index::from_nix_search_json(key, &json)?;
    let path = index::index_path(key);
    idx.save(&path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    eprintln!(
        "rix: indexed {} packages → {}",
        idx.entries.len(),
        path.display()
    );
    Ok(idx)
}

/// Load the index for the current nixpkgs, building it if needed.
fn load_index(rebuild: bool) -> index::Index {
    // Fast path: the most recent index, unless a rebuild was requested. Checking the
    // key costs a `nix flake metadata` call, so it is done only by `rix update`.
    if !rebuild {
        if let Some(p) = newest_index() {
            if let Ok(i) = index::Index::load(&p) {
                return i;
            }
        }
    }
    let key = nixpkgs_key().unwrap_or_else(|e| die(&e));
    if !rebuild {
        if let Ok(i) = index::Index::load(&index::index_path(&key)) {
            return i;
        }
    }
    build_index(&key).unwrap_or_else(|e| die(&e))
}

fn newest_index() -> Option<PathBuf> {
    let rd = std::fs::read_dir(index::cache_dir()).ok()?;
    rd.flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("index-"))
        .filter(|e| e.file_name().to_string_lossy().ends_with(".tsv"))
        .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok())
        .map(|e| e.path())
}

fn fmt_time(t: Option<std::time::SystemTime>) -> String {
    let Some(t) = t else { return "?".into() };
    let secs = t
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    // Civil-from-days (UTC), no external crates.
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}",
        rem / 3600,
        (rem % 3600) / 60
    )
}

fn print_generations(p: &profile::Profile, json: bool) -> ExitCode {
    let gens = p
        .generations()
        .unwrap_or_else(|e| die(&format!("{}: {e}", p.path.display())));
    if json {
        let v: Vec<_> = gens
            .iter()
            .map(|g| serde_json::json!({"generation": g.number, "current": g.current, "target": g.target, "created": fmt_time(g.created)}))
            .collect();
        println!("{}", serde_json::to_string_pretty(&v).unwrap());
        return ExitCode::SUCCESS;
    }
    if gens.is_empty() {
        println!("no generations in {}", p.path.display());
    }
    for g in &gens {
        let target = std::fs::read_link(&g.link)
            .ok()
            .map(|t| t.display().to_string())
            .unwrap_or_default();
        println!(
            "{} {:>4}  {}  {}",
            if g.current { "*" } else { " " },
            g.number,
            fmt_time(g.created),
            target
        );
    }
    ExitCode::SUCCESS
}

fn parse_to(args: &[String]) -> Option<u64> {
    args.iter()
        .position(|a| a == "--to")
        .and_then(|i| args.get(i + 1))
        .map(|v| {
            v.parse()
                .unwrap_or_else(|_| die("--to needs a generation number"))
        })
}

fn resolve_installable(pkg: &str, idx: Option<&index::Index>) -> String {
    if pkg.contains('#') || pkg.contains(':') || pkg.starts_with('/') || pkg.starts_with('.') {
        return pkg.to_string();
    }
    if let Some(idx) = idx {
        if idx.get(pkg).is_none() {
            let sugg: Vec<String> = idx
                .search(&[pkg.to_string()], 5)
                .iter()
                .map(|e| e.attr.clone())
                .collect();
            if sugg.is_empty() {
                die(&format!("no package '{pkg}' in {}", pkgs_ref()));
            }
            die(&format!(
                "no package '{pkg}' in {}; did you mean: {}?",
                pkgs_ref(),
                sugg.join(", ")
            ));
        }
    }
    format!("{}#{pkg}", pkgs_ref())
}

fn cmd_verify(paths: &[String]) -> ExitCode {
    let mut bad = 0;
    for p in paths {
        let sp = match store::StorePath::parse(p) {
            Ok(sp) => sp,
            Err(e) => {
                eprintln!("{p}: {e}");
                bad += 1;
                continue;
            }
        };
        let full = sp.to_string();
        // Newer Nix wants an explicit JSON format; older Nix rejects the flag.
        let out = match capture_quiet("nix", &["path-info", "--json", "--json-format", "1", &full])
            .or_else(|_| capture("nix", &["path-info", "--json", &full]))
        {
            Ok(o) => o,
            Err(e) => {
                eprintln!("{full}: {e}");
                bad += 1;
                continue;
            }
        };
        let v: serde_json::Value = serde_json::from_str(&out).unwrap_or_default();
        // `nix path-info --json` returns either an object keyed by path or an array.
        let info = if v.is_array() {
            v[0].clone()
        } else {
            v[&full].clone()
        };
        let expected = info["narHash"].as_str().unwrap_or("").to_string();
        let (hash, size) = match nar::nar_hash(Path::new(&full)) {
            Ok(h) => h,
            Err(e) => {
                eprintln!("{full}: {e}");
                bad += 1;
                continue;
            }
        };
        let got_sri = nar::sri(&hash);
        let got_nix32 = format!("sha256:{}", nix32::encode(&hash));
        if expected == got_sri || expected == got_nix32 {
            println!("ok   {full} ({size} bytes)");
        } else {
            println!("FAIL {full}: expected {expected}, got {got_sri}");
            bad += 1;
        }
    }
    if bad == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

fn cmd_doctor() -> ExitCode {
    let mut ok = true;
    let mut check = |name: &str, pass: bool, hint: &str| {
        println!(
            "{} {name}{}",
            if pass { "✓" } else { "✗" },
            if pass {
                String::new()
            } else {
                format!(" — {hint}")
            }
        );
        ok &= pass;
    };
    check(
        "/nix/store exists",
        Path::new("/nix/store").is_dir(),
        "Nix is not installed",
    );
    check(
        "nix-daemon socket",
        Path::new("/nix/var/nix/daemon-socket/socket").exists(),
        "start the daemon (systemctl start nix-daemon, or r6 start nix-daemon)",
    );
    let nix_ok = Command::new(nix_bin("nix"))
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    check("nix executable", nix_ok, "nix not found in PATH");
    check(
        "system flake /etc/nixos/flake.nix",
        Path::new("/etc/nixos/flake.nix").exists(),
        "system is not flake-managed",
    );
    check(
        "package index",
        newest_index().is_some(),
        "run `rix update`",
    );
    let gens = profile::Profile::system()
        .generations()
        .map(|g| g.len())
        .unwrap_or(0);
    check("system generations", gens > 0, "no system profile found");
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut take = |f: &str| {
        let had = args.iter().any(|a| a == f);
        args.retain(|a| a != f);
        had
    };
    let json = take("--json");
    let sri = take("--sri");
    let _verbose = take("--verbose");
    if take("--version") || take("-V") {
        println!("rix {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    let help = take("--help") | take("-h");
    if help || args.is_empty() {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let mut limit = 25usize;
    if let Some(i) = args.iter().position(|a| a == "--limit") {
        limit = args
            .get(i + 1)
            .and_then(|v| v.parse().ok())
            .unwrap_or(limit);
        args.drain(i..(i + 2).min(args.len()));
    }
    let cmd = args.remove(0);
    let rest = args;

    match cmd.as_str() {
        "search" => {
            if rest.is_empty() {
                die("search needs at least one term");
            }
            let idx = load_index(false);
            let hits = idx.search(&rest, limit);
            if json {
                let v: Vec<_> = hits
                    .iter()
                    .map(|e| serde_json::json!({"attr": e.attr, "pname": e.pname, "version": e.version, "description": e.description}))
                    .collect();
                println!("{}", serde_json::to_string_pretty(&v).unwrap());
            } else {
                for e in &hits {
                    println!("\x1b[1m* {}\x1b[0m ({})", e.attr, e.version);
                    if !e.description.is_empty() {
                        println!("  {}", e.description);
                    }
                }
                if hits.is_empty() {
                    eprintln!("rix: no results");
                    return ExitCode::from(1);
                }
            }
            ExitCode::SUCCESS
        }
        "info" => {
            let name = rest
                .first()
                .unwrap_or_else(|| die("info needs a package name"));
            let idx = load_index(false);
            match idx.get(name) {
                Some(e) => {
                    println!("attribute:   {}\npackage:     {}\nversion:     {}\ndescription: {}\ninstall:     rix install {}", e.attr, e.pname, e.version, e.description, e.attr);
                    ExitCode::SUCCESS
                }
                None => die(&format!("no package '{name}'")),
            }
        }
        "update" => {
            if rest.first().map(|s| s.as_str()) == Some("--system") {
                return delegate(
                    "nix",
                    &[
                        "flake".into(),
                        "update".into(),
                        "--flake".into(),
                        SYSTEM_FLAKE.into(),
                    ],
                );
            }
            load_index(true);
            ExitCode::SUCCESS
        }
        "install" | "add" => {
            if rest.is_empty() {
                die("install needs a package name");
            }
            let idx = newest_index().and_then(|p| index::Index::load(&p).ok());
            let installables: Vec<String> = rest
                .iter()
                .map(|p| resolve_installable(p, idx.as_ref()))
                .collect();
            let mut a = vec!["profile".to_string(), "add".to_string()];
            a.extend(installables);
            delegate("nix", &a)
        }
        "remove" | "uninstall" => {
            if rest.is_empty() {
                die("remove needs a package name");
            }
            let mut a = vec!["profile".to_string(), "remove".to_string()];
            a.extend(rest);
            delegate("nix", &a)
        }
        "upgrade" => {
            if rest.first().map(|s| s.as_str()) == Some("--system") {
                return delegate(
                    "nixos-rebuild",
                    &[
                        "switch".into(),
                        "--flake".into(),
                        format!("{SYSTEM_FLAKE}#raze"),
                    ],
                );
            }
            delegate("nix", &["profile".into(), "upgrade".into(), "--all".into()])
        }
        "list" => {
            let p = profile::Profile::user_default();
            let target = std::fs::canonicalize(&p.path).unwrap_or_else(|_| p.path.clone());
            let elems = profile::read_manifest(&target).unwrap_or_else(|e| die(&e.to_string()));
            if json {
                let v: Vec<_> = elems.iter().map(|e| serde_json::json!({"name": e.name, "attrPath": e.attr_path, "url": e.original_url, "storePaths": e.store_paths})).collect();
                println!("{}", serde_json::to_string_pretty(&v).unwrap());
            } else if elems.is_empty() {
                println!("no packages installed in {}", p.path.display());
            } else {
                for e in &elems {
                    let ver = e
                        .store_paths
                        .first()
                        .and_then(|s| store::StorePath::parse(s).ok())
                        .map(|s| s.name_version().1.to_string())
                        .unwrap_or_default();
                    println!(
                        "{:<28} {:<14} {}",
                        e.name,
                        ver,
                        e.original_url.clone().unwrap_or_default()
                    );
                }
            }
            ExitCode::SUCCESS
        }
        "generations" | "history" => print_generations(&profile::Profile::user_default(), json),
        "rollback" => {
            let p = profile::Profile::user_default();
            let r = match parse_to(&rest) {
                Some(n) => p.switch_to(n).map(|_| n),
                None => p.rollback(),
            };
            match r {
                Ok(n) => {
                    println!("switched {} to generation {n}", p.path.display());
                    ExitCode::SUCCESS
                }
                Err(e) => die(&e.to_string()),
            }
        }
        "system" => {
            let sub = rest.first().map(|s| s.as_str()).unwrap_or("generations");
            match sub {
                "switch" | "boot" | "test" | "build" | "dry-activate" => {
                    let mut a = vec![
                        sub.to_string(),
                        "--flake".to_string(),
                        format!("{SYSTEM_FLAKE}#raze"),
                    ];
                    a.extend(rest.iter().skip(1).cloned());
                    delegate("nixos-rebuild", &a)
                }
                "update" => delegate(
                    "nix",
                    &[
                        "flake".into(),
                        "update".into(),
                        "--flake".into(),
                        SYSTEM_FLAKE.into(),
                    ],
                ),
                "generations" => print_generations(&profile::Profile::system(), json),
                "rollback" => {
                    let p = profile::Profile::system();
                    let r = match parse_to(&rest) {
                        Some(n) => p.switch_to(n).map(|_| n),
                        None => p.rollback(),
                    };
                    let n = r.unwrap_or_else(|e| die(&e.to_string()));
                    let script = p.path.join("bin/switch-to-configuration");
                    // A generation with a different PID 1 (r6 <-> systemd) cannot be
                    // activated in place: make it the boot default and ask for a reboot.
                    let running_r6 = std::fs::read_to_string("/proc/1/comm")
                        .map(|c| c.trim() == "r6-init")
                        .unwrap_or(false);
                    let target_r6 = p.path.join("etc/r6/r6.toml").exists();
                    if running_r6 != target_r6 {
                        println!(
                            "system profile now at generation {n}; it uses {} as PID 1, so it \
                             becomes active after a reboot",
                            if target_r6 { "r6" } else { "systemd" }
                        );
                        delegate(&script.display().to_string(), &["boot".into()])
                    } else {
                        println!("system profile now at generation {n}; activating");
                        delegate(&script.display().to_string(), &["switch".into()])
                    }
                }
                other => die(&format!("unknown system command '{other}'")),
            }
        }
        "build" | "develop" | "shell" | "run" | "eval" | "repl" => {
            let mut a = vec![cmd.clone()];
            a.extend(rest);
            delegate("nix", &a)
        }
        "flake" => {
            let sub = rest.first().map(|s| s.as_str()).unwrap_or("info");
            if sub == "info" {
                let dir = rest
                    .get(1)
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("."));
                let text = std::fs::read_to_string(dir.join("flake.lock"))
                    .unwrap_or_else(|e| die(&format!("{}/flake.lock: {e}", dir.display())));
                let inputs = flakelock::parse(&text).unwrap_or_else(|e| die(&e));
                for i in inputs {
                    println!(
                        "{:<12} {:<28} {}  {}",
                        i.name,
                        i.source,
                        i.rev
                            .as_deref()
                            .map(|r| &r[..r.len().min(12)])
                            .unwrap_or("-"),
                        i.last_modified
                            .map(|t| fmt_time(Some(
                                std::time::UNIX_EPOCH + std::time::Duration::from_secs(t as u64)
                            )))
                            .unwrap_or_default()
                    );
                }
                return ExitCode::SUCCESS;
            }
            let mut a = vec!["flake".to_string()];
            a.extend(rest);
            delegate("nix", &a)
        }
        "gc" => {
            let mut a = Vec::new();
            if let Some(i) = rest.iter().position(|x| x == "--older-than") {
                a.push("--delete-older-than".to_string());
                a.push(
                    rest.get(i + 1)
                        .cloned()
                        .unwrap_or_else(|| die("--older-than needs a value like 30d")),
                );
            } else if rest.iter().any(|x| x == "--all") {
                a.push("-d".to_string());
            }
            delegate("nix-collect-garbage", &a)
        }
        "hash" => {
            let mode = rest
                .first()
                .map(|s| s.as_str())
                .unwrap_or_else(|| die("hash path|file PATH..."));
            if rest.len() < 2 {
                die("hash needs a path");
            }
            for p in &rest[1..] {
                let h = match mode {
                    "path" => nar::nar_hash(Path::new(p)).map(|(h, _)| h),
                    "file" => nar::file_hash(Path::new(p)),
                    _ => die("hash mode must be 'path' or 'file'"),
                };
                match h {
                    Ok(h) => println!("{}", if sri { nar::sri(&h) } else { nix32::encode(&h) }),
                    Err(e) => die(&format!("{p}: {e}")),
                }
            }
            ExitCode::SUCCESS
        }
        "verify" => {
            if rest.is_empty() {
                die("verify needs store paths");
            }
            cmd_verify(&rest)
        }
        "doctor" => cmd_doctor(),
        other => die(&format!("unknown command '{other}'\n\n{USAGE}")),
    }
}
