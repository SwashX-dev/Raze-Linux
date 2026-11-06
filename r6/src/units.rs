//! Import systemd units as r6 services (`r6-import`).
//!
//! NixOS generates a complete systemd unit tree (`/etc/systemd/system`: unit files
//! from packages and from NixOS options, drop-ins, `*.wants/` links). Under r6 that
//! tree is not executed; this module translates the units it can into r6 service
//! definitions at system build time, so services enabled through ordinary NixOS
//! options run under r6 without hand-written definitions:
//!
//! - services wanted by the boot targets (`multi-user.target.wants/` and friends),
//!   and the services they require, including drop-ins and template instances;
//! - `.socket` units with Unix stream sockets become r6 socket activation (`listen`);
//! - `.timer` units become entries for `r6-timers` (period approximated from
//!   `OnCalendar=`/`OnUnitActiveSec=`).
//!
//! A unit that uses something r6 cannot provide (`Type=forking`, network sockets,
//! systemd's own binaries, ...) is not imported; the reason is reported.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Targets whose `.wants/` directories define what starts at boot.
pub const BOOT_TARGETS: &[&str] = &[
    "sysinit.target",
    "basic.target",
    "network.target",
    "network-online.target",
    "nss-lookup.target",
    "time-sync.target",
    "multi-user.target",
    "graphical.target",
    "default.target",
];

/// A parsed unit file with its drop-ins applied in order.
#[derive(Debug, Default, Clone)]
pub struct UnitFile {
    /// (section, key, value) in file order; drop-ins appended after the main file.
    pub entries: Vec<(String, String, String)>,
}

impl UnitFile {
    pub fn parse(text: &str) -> UnitFile {
        let mut u = UnitFile::default();
        u.append(text);
        u
    }

    fn append(&mut self, text: &str) {
        let mut section = String::new();
        let mut pending = String::new();
        for raw in text.lines() {
            let line = raw.trim_end();
            if pending.is_empty() {
                let t = line.trim_start();
                if t.is_empty() || t.starts_with('#') || t.starts_with(';') {
                    continue;
                }
                if t.starts_with('[') && t.ends_with(']') {
                    section = t[1..t.len() - 1].to_string();
                    continue;
                }
            }
            if let Some(stripped) = line.strip_suffix('\\') {
                pending.push_str(stripped);
                pending.push(' ');
                continue;
            }
            pending.push_str(line);
            let full = std::mem::take(&mut pending);
            if let Some((k, v)) = full.split_once('=') {
                self.entries
                    .push((section.clone(), k.trim().to_string(), v.trim().to_string()));
            }
        }
    }

    /// Every value of `key` in `section`, honouring empty-assignment resets.
    pub fn list(&self, section: &str, key: &str) -> Vec<String> {
        let mut out = Vec::new();
        for (s, k, v) in &self.entries {
            if s == section && k == key {
                if v.is_empty() {
                    out.clear();
                } else {
                    out.push(v.clone());
                }
            }
        }
        out
    }

    /// The last value of `key` in `section`.
    pub fn get(&self, section: &str, key: &str) -> Option<String> {
        self.list(section, key).pop()
    }

    /// Space-separated list values (Requires=, After=, ...), accumulated.
    pub fn words(&self, section: &str, key: &str) -> Vec<String> {
        self.list(section, key)
            .iter()
            .flat_map(|v| v.split_whitespace().map(str::to_string).collect::<Vec<_>>())
            .collect()
    }

    pub fn has_section(&self, section: &str) -> bool {
        self.entries.iter().any(|(s, _, _)| s == section)
    }

    pub fn keys(&self, section: &str) -> BTreeSet<String> {
        self.entries
            .iter()
            .filter(|(s, _, _)| s == section)
            .map(|(_, k, _)| k.clone())
            .collect()
    }
}

/// `foo@bar.service` -> ("foo", Some("bar"), "service").
pub fn split_name(name: &str) -> (&str, Option<&str>, &str) {
    let (stem, suffix) = name.rsplit_once('.').unwrap_or((name, ""));
    match stem.split_once('@') {
        Some((p, i)) => (p, Some(i), suffix),
        None => (stem, None, suffix),
    }
}

/// Load a unit (following symlinks) with its drop-ins; template instances fall back
/// to the template file.
pub fn load(dir: &Path, name: &str) -> Option<UnitFile> {
    let (prefix, instance, suffix) = split_name(name);
    let template = instance.map(|_| format!("{prefix}@.{suffix}"));
    let main = std::fs::read_to_string(dir.join(name)).ok().or_else(|| {
        template
            .as_ref()
            .and_then(|t| std::fs::read_to_string(dir.join(t)).ok())
    })?;
    let mut u = UnitFile::parse(&main);
    // Drop-ins from the template's and the unit's .d directories, sorted by file name.
    let mut dropins: BTreeMap<String, PathBuf> = BTreeMap::new();
    for d in template.iter().chain(std::iter::once(&name.to_string())) {
        if let Ok(rd) = std::fs::read_dir(dir.join(format!("{d}.d"))) {
            for e in rd.flatten() {
                let f = e.file_name().to_string_lossy().into_owned();
                if f.ends_with(".conf") {
                    dropins.insert(f, e.path());
                }
            }
        }
    }
    for p in dropins.values() {
        if let Ok(t) = std::fs::read_to_string(p) {
            u.append(&t);
        }
    }
    Some(u)
}

/// Replace systemd specifiers (%n, %i, %t, ...) in a value.
pub fn specifiers(value: &str, name: &str, home: &str) -> String {
    let (prefix, instance, _) = split_name(name);
    let stem = name.rsplit_once('.').map(|x| x.0).unwrap_or(name);
    let inst = instance.unwrap_or("");
    let mut out = String::new();
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push_str(name),
            Some('N') => out.push_str(stem),
            Some('p') | Some('P') => out.push_str(prefix),
            Some('i') => out.push_str(inst),
            Some('I') => out.push_str(&inst.replace('-', "/")),
            Some('t') => out.push_str("/run"),
            Some('S') => out.push_str("/var/lib"),
            Some('C') => out.push_str("/var/cache"),
            Some('L') => out.push_str("/var/log"),
            Some('E') => out.push_str("/etc"),
            Some('T') => out.push_str("/tmp"),
            Some('V') => out.push_str("/var/tmp"),
            Some('h') => out.push_str(home),
            Some('%') => out.push('%'),
            Some(o) => {
                out.push('%');
                out.push(o);
            }
            None => out.push('%'),
        }
    }
    out
}

/// Split a systemd command line into words (quotes and backslash escapes).
pub fn split_command(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (None, ' ' | '\t') => {
                if in_word {
                    words.push(std::mem::take(&mut cur));
                    in_word = false;
                }
            }
            (None, '"' | '\'') => {
                quote = Some(c);
                in_word = true;
            }
            (Some(q), c) if c == q => quote = None,
            (_, '\\') => {
                in_word = true;
                match chars.next() {
                    Some('n') => cur.push('\n'),
                    Some('t') => cur.push('\t'),
                    Some(o) => cur.push(o),
                    None => {}
                }
            }
            (_, c) => {
                cur.push(c);
                in_word = true;
            }
        }
    }
    if in_word {
        words.push(cur);
    }
    words
}

/// A command line from Exec*=: prefixes and words.
#[derive(Debug, Clone, PartialEq)]
pub struct ExecLine {
    pub ignore_failure: bool,
    pub privileged: bool,
    /// `@`: the second word is argv[0].
    pub argv0: Option<String>,
    pub argv: Vec<String>,
}

pub fn parse_exec(line: &str) -> Option<ExecLine> {
    let mut words = split_command(line);
    if words.is_empty() {
        return None;
    }
    let mut first = words.remove(0);
    let (mut ignore, mut privileged, mut at) = (false, false, false);
    loop {
        match first.chars().next() {
            Some('-') => ignore = true,
            Some('+') | Some('!') => privileged = true,
            Some('@') => at = true,
            Some(':') | Some('|') => {}
            _ => break,
        }
        first.remove(0);
    }
    let argv0 = if at && !words.is_empty() {
        Some(words.remove(0))
    } else {
        None
    };
    let mut argv = vec![first];
    argv.extend(words);
    Some(ExecLine {
        ignore_failure: ignore,
        privileged,
        argv0,
        argv,
    })
}

fn shell_quote(s: &str) -> String {
    if !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/_-.,:=+@%".contains(&b))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

/// Shell text for one command; `$VAR`/`${VAR}` stay unquoted so they expand as
/// systemd would expand them.
fn shell_command(e: &ExecLine) -> String {
    let words: Vec<String> = e
        .argv
        .iter()
        .map(|w| {
            if w.contains('$') {
                // Double quotes keep `$VAR` expansion (as one word, like `${VAR}`).
                let esc = w
                    .replace('\\', "\\\\")
                    .replace('"', "\\\"")
                    .replace('`', "\\`");
                format!("\"{esc}\"")
            } else {
                shell_quote(w)
            }
        })
        .collect();
    match &e.argv0 {
        Some(a0) => format!("exec -a {} {}", shell_quote(a0), words.join(" ")),
        None => words.join(" "),
    }
}

/// Seconds from a systemd time span ("5min 30s", "1h", "90", "infinity" -> None).
pub fn timespan(s: &str) -> Option<u64> {
    let s = s.trim();
    if s == "infinity" {
        return None;
    }
    let mut total = 0f64;
    let mut num = String::new();
    let mut unit = String::new();
    let flush = |num: &mut String, unit: &mut String, total: &mut f64| -> Option<()> {
        if num.is_empty() {
            return Some(());
        }
        let n: f64 = num.parse().ok()?;
        let mult = match unit.as_str() {
            "" | "s" | "sec" | "second" | "seconds" => 1.0,
            "ms" | "msec" => 0.001,
            "us" | "usec" => 0.000001,
            "m" | "min" | "minute" | "minutes" => 60.0,
            "h" | "hr" | "hour" | "hours" => 3600.0,
            "d" | "day" | "days" => 86400.0,
            "w" | "week" | "weeks" => 604800.0,
            "M" | "month" | "months" => 2629800.0,
            "y" | "year" | "years" => 31557600.0,
            _ => return None,
        };
        *total += n * mult;
        num.clear();
        unit.clear();
        Some(())
    };
    for c in s.chars() {
        if c.is_ascii_digit() || c == '.' {
            if !unit.is_empty() {
                flush(&mut num, &mut unit, &mut total)?;
            }
            num.push(c);
        } else if c.is_whitespace() {
            continue;
        } else {
            unit.push(c);
        }
    }
    flush(&mut num, &mut unit, &mut total)?;
    Some(total.round() as u64)
}

/// The period of an OnCalendar= expression, in seconds (calendar events are
/// approximated by how often they recur).
pub fn calendar_period(spec: &str) -> u64 {
    let s = spec.trim().to_ascii_lowercase();
    match s.as_str() {
        "minutely" => return 60,
        "hourly" => return 3600,
        "daily" => return 86400,
        "weekly" => return 604800,
        "monthly" => return 2629800,
        "quarterly" => return 7889400,
        "semiannually" | "semi-annually" => return 15778800,
        "yearly" | "annually" => return 31557600,
        _ => {}
    }
    let parts: Vec<&str> = s.split_whitespace().collect();
    let weekday = parts.iter().any(|p| {
        ["mon", "tue", "wed", "thu", "fri", "sat", "sun"]
            .iter()
            .any(|d| p.starts_with(d))
    });
    let date = parts.iter().find(|p| p.contains('-') && !p.contains(':'));
    let time = parts.iter().find(|p| p.contains(':'));
    if let Some(t) = time {
        let fields: Vec<&str> = t.split(':').collect();
        let step = |f: &str| f.split_once('/').and_then(|(_, n)| n.parse::<u64>().ok());
        let (h, m) = (fields[0], fields.get(1).copied().unwrap_or("0"));
        if h == "*" || h.contains('/') || h.contains(',') {
            if m == "*" {
                return 60;
            }
            if let Some(n) = step(m) {
                return n * 60;
            }
            if let Some(n) = step(h) {
                return n * 3600;
            }
            if h == "*" {
                return 3600;
            }
            let n = h.split(',').count() as u64;
            return 86400 / n.max(1);
        }
    }
    if weekday {
        return 604800;
    }
    if let Some(d) = date {
        let f: Vec<&str> = d.split('-').collect();
        if f.len() == 3 && f[1] != "*" {
            return 31557600;
        }
        if f.len() >= 2 && f[f.len() - 1] != "*" {
            return 2629800;
        }
    }
    86400
}

/// What the importer knows about the r6 services defined by hand.
#[derive(Debug, Default, Clone)]
pub struct Context {
    /// systemd unit name (e.g. "NetworkManager.service", "network.target") -> r6 service.
    pub aliases: BTreeMap<String, String>,
    /// Units never imported (hand-written equivalents, not applicable under r6).
    pub skip: BTreeSet<String>,
    /// Names of existing r6 services (not to be shadowed).
    pub existing: BTreeSet<String>,
    /// Existing r6 services that start at boot.
    pub existing_enabled: BTreeSet<String>,
    /// Users that exist statically (DynamicUser= services run as one of these).
    pub static_users: BTreeSet<String>,
    /// Soft dependencies of every imported system service (e.g. filesystems).
    pub base_wants: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Imported {
    pub name: String,
    /// TOML text of the r6 service.
    pub toml: String,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Timer {
    pub name: String,
    pub service: String,
    pub interval_s: u64,
    pub boot_delay_s: u64,
    pub persistent: bool,
}

#[derive(Debug, Default)]
pub struct Report {
    pub imported: Vec<Imported>,
    pub timers: Vec<Timer>,
    /// (unit, reason)
    pub unsupported: Vec<(String, String)>,
}

fn toml_str(s: &str) -> String {
    serde_json::to_string(s).unwrap()
}

fn toml_list(v: &[String]) -> String {
    format!(
        "[{}]",
        v.iter().map(|s| toml_str(s)).collect::<Vec<_>>().join(", ")
    )
}

/// The r6 name for a systemd service unit.
pub fn r6_name(unit: &str) -> String {
    let n = unit.strip_suffix(".service").unwrap_or(unit);
    // An Accept=yes socket's template (foo@.service) becomes the service "foo".
    n.strip_suffix('@').unwrap_or(n).to_string()
}

struct Translation {
    kind: &'static str,
    command: Vec<String>,
    lines: Vec<String>,
    deps_hard: Vec<String>,
    deps_soft: Vec<String>,
    environment: BTreeMap<String, String>,
    notes: Vec<String>,
}

const SUPPORTED_SERVICE_KEYS: &[&str] = &[
    "Type",
    "ExecStart",
    "ExecStartPre",
    "ExecStartPost",
    "ExecStop",
    "ExecStopPost",
    "ExecReload",
    "User",
    "Group",
    "SupplementaryGroups",
    "DynamicUser",
    "Environment",
    "EnvironmentFile",
    "WorkingDirectory",
    "UMask",
    "LimitNOFILE",
    "OOMScoreAdjust",
    "AmbientCapabilities",
    "CapabilityBoundingSet",
    "RuntimeDirectory",
    "StateDirectory",
    "Restart",
    "RestartSec",
    "TimeoutStopSec",
    "TimeoutStartSec",
    "TimeoutSec",
    "KillSignal",
    "KillMode",
    "RemainAfterExit",
    "BusName",
    "NotifyAccess",
    "PIDFile",
    "StandardOutput",
    "StandardError",
    "StandardInput",
    "SyslogIdentifier",
];

fn translate(unit: &str, u: &UnitFile, ctx: &Context) -> Result<Translation, String> {
    let get = |k: &str| u.get("Service", k).map(|v| specifiers(&v, unit, "/root"));
    let ty = get("Type").unwrap_or_else(|| "simple".into());
    let kind = match ty.as_str() {
        "simple" | "exec" | "idle" | "dbus" | "notify" | "notify-reload" => "longrun",
        "oneshot" => "oneshot",
        "forking" => return Err("Type=forking is not supported".into()),
        other => return Err(format!("Type={other} is not supported")),
    };
    let execs: Vec<ExecLine> = u
        .list("Service", "ExecStart")
        .iter()
        .filter_map(|l| parse_exec(&specifiers(l, unit, "/root")))
        .collect();
    if execs.is_empty() {
        return Err("no ExecStart=".into());
    }
    if kind == "longrun" && execs.len() > 1 {
        return Err("several ExecStart= in a long-running service".into());
    }
    let systemd_bin = |e: &ExecLine| {
        e.argv[0].contains("-systemd-") && e.argv[0].contains("/lib/systemd/")
            || e.argv[0].ends_with("/systemctl")
            || e.argv[0].ends_with("/systemd-tmpfiles")
    };
    if execs.iter().any(systemd_bin) {
        return Err("runs a systemd component".into());
    }
    if execs.iter().any(|e| !e.argv[0].starts_with('/')) {
        return Err("ExecStart= without an absolute path".into());
    }
    let mut notes = Vec::new();
    for k in u.keys("Service") {
        if !SUPPORTED_SERVICE_KEYS.contains(&k.as_str())
            && !k.starts_with("Protect")
            && !k.starts_with("Private")
            && !k.starts_with("Restrict")
            && !k.starts_with("System")
            && !k.starts_with("Lock")
            && !k.starts_with("Memory")
            && !k.starts_with("Read")
            && !k.starts_with("Inaccessible")
            && !k.starts_with("Device")
            && !k.starts_with("NoNew")
            && !k.starts_with("Ignore")
        {
            notes.push(format!("ignored {k}="));
        }
    }
    let user = get("User").filter(|s| !s.is_empty());
    let group = get("Group").filter(|s| !s.is_empty());
    let dynamic = get("DynamicUser").map(|v| is_true(&v)).unwrap_or(false);
    let (user, group) = if dynamic {
        let wanted = user.clone().unwrap_or_else(|| r6_name(unit));
        if ctx.static_users.contains(&wanted) {
            (Some(wanted), group)
        } else {
            notes.push(format!(
                "DynamicUser: runs as nobody (no static user '{wanted}')"
            ));
            (Some("nobody".to_string()), Some("nogroup".to_string()))
        }
    } else {
        (user, group)
    };
    let pre: Vec<ExecLine> = u
        .list("Service", "ExecStartPre")
        .iter()
        .filter_map(|l| parse_exec(&specifiers(l, unit, "/root")))
        .collect();
    let post: Vec<ExecLine> = u
        .list("Service", "ExecStartPost")
        .iter()
        .filter_map(|l| parse_exec(&specifiers(l, unit, "/root")))
        .collect();
    if user.is_some() && pre.iter().chain(post.iter()).any(|e| e.privileged) {
        return Err("privileged (+/!) ExecStartPre/Post with User=".into());
    }
    if pre.iter().chain(post.iter()).any(systemd_bin) {
        return Err("ExecStartPre/Post runs a systemd component".into());
    }
    // Main command privileged with User= -> run as root (systemd semantics).
    let user = if execs.iter().all(|e| e.privileged) {
        None
    } else {
        user
    };
    let env_files = u.list("Service", "EnvironmentFile");
    let mut environment: BTreeMap<String, String> = BTreeMap::new();
    for line in u.list("Service", "Environment") {
        for w in split_command(&specifiers(&line, unit, "/root")) {
            if let Some((k, v)) = w.split_once('=') {
                environment.insert(k.to_string(), v.to_string());
            }
        }
    }
    let conditions: Vec<(String, String)> = u
        .entries
        .iter()
        .filter(|(s, k, v)| {
            s == "Unit" && (k.starts_with("Condition") || k.starts_with("Assert")) && !v.is_empty()
        })
        .map(|(_, k, v)| (k.clone(), specifiers(v, unit, "/root")))
        .collect();
    let mut checks = Vec::new();
    for (k, v) in &conditions {
        let (neg, path) = match v.strip_prefix('!') {
            Some(p) => (true, p.trim_start_matches('|')),
            None => (false, v.trim_start_matches('|')),
        };
        let test = match k
            .trim_start_matches("Condition")
            .trim_start_matches("Assert")
        {
            "PathExists" => format!("[ -e {} ]", shell_quote(path)),
            "PathIsDirectory" => format!("[ -d {} ]", shell_quote(path)),
            "FileNotEmpty" => format!("[ -s {} ]", shell_quote(path)),
            "PathIsSymbolicLink" => format!("[ -L {} ]", shell_quote(path)),
            "FileIsExecutable" => format!("[ -x {} ]", shell_quote(path)),
            "DirectoryNotEmpty" => format!("[ -n \"$(ls -A {} 2>/dev/null)\" ]", shell_quote(path)),
            _ => {
                notes.push(format!("ignored {k}={v}"));
                continue;
            }
        };
        checks.push(if neg { format!("! {test}") } else { test });
    }
    let simple = pre.is_empty()
        && post.is_empty()
        && env_files.is_empty()
        && checks.is_empty()
        && execs.len() == 1
        && execs[0].argv0.is_none()
        && !execs[0].ignore_failure
        && !execs[0].argv.iter().any(|w| w.contains('$'));
    let command = if simple {
        execs[0].argv.clone()
    } else {
        let mut script = Vec::new();
        for c in &checks {
            script.push(format!("{c} || exit 0"));
        }
        for f in &env_files {
            let (opt, f) = match f.strip_prefix('-') {
                Some(p) => (true, p),
                None => (false, f.as_str()),
            };
            let f = specifiers(f, unit, "/root");
            if opt {
                script.push(format!(
                    "[ -r {0} ] && {{ set -a; . {0}; set +a; }}",
                    shell_quote(&f)
                ));
            } else {
                script.push(format!("set -a; . {} || exit 1; set +a", shell_quote(&f)));
            }
        }
        for p in &pre {
            let c = shell_command(p);
            script.push(if p.ignore_failure {
                format!("{c} || true")
            } else {
                format!("{c} || exit $?")
            });
        }
        if kind == "oneshot" {
            for e in execs.iter().chain(post.iter()) {
                let c = shell_command(e);
                script.push(if e.ignore_failure {
                    format!("{c} || true")
                } else {
                    format!("{c} || exit $?")
                });
            }
        } else {
            if !post.is_empty() {
                let body: Vec<String> = post.iter().map(shell_command).collect();
                script.push(format!("( sleep 1; {} ) &", body.join("; ")));
            }
            let main = &execs[0];
            let c = shell_command(main);
            script.push(if c.starts_with("exec ") {
                c
            } else {
                format!("exec {c}")
            });
        }
        vec!["/bin/sh".into(), "-c".into(), script.join("\n")]
    };

    let mut lines = Vec::new();
    match ty.as_str() {
        "notify" | "notify-reload" => lines.push("ready = \"sd-notify\"".to_string()),
        _ => {}
    }
    if let Some(user) = &user {
        lines.push(format!("user = {}", toml_str(user)));
    }
    if let (Some(g), true) = (&group, user.is_some()) {
        lines.push(format!("group = {}", toml_str(g)));
    }
    if let Some(wd) = get("WorkingDirectory") {
        let wd = wd.trim_start_matches('-');
        if wd.starts_with('/') {
            lines.push(format!("working_directory = {}", toml_str(wd)));
        } else {
            notes.push(format!("ignored WorkingDirectory={wd}"));
        }
    }
    if let Some(m) = get("UMask") {
        lines.push(format!("umask = {}", toml_str(&m)));
    }
    if let Some(n) = get("LimitNOFILE") {
        let hard = n.rsplit(':').next().unwrap_or(&n);
        let v = if hard == "infinity" {
            Some(1048576)
        } else {
            hard.parse::<u64>().ok()
        };
        if let Some(v) = v {
            lines.push(format!("rlimit_nofile = {v}"));
        }
    }
    if let Some(o) = get("OOMScoreAdjust").and_then(|o| o.parse::<i32>().ok()) {
        lines.push(format!("oom_score_adjust = {o}"));
    }
    let caps: Vec<String> = u
        .words("Service", "AmbientCapabilities")
        .into_iter()
        .filter(|c| !c.starts_with('~'))
        .collect();
    if !caps.is_empty() && user.is_some() {
        lines.push(format!("capabilities = {}", toml_list(&caps)));
    }
    let first_dir = |k: &str| u.words("Service", k).into_iter().next();
    if let Some(d) = first_dir("RuntimeDirectory") {
        lines.push(format!(
            "runtime_directory = {}",
            toml_str(d.split(':').next().unwrap())
        ));
    }
    if let Some(d) = first_dir("StateDirectory") {
        lines.push(format!(
            "state_directory = {}",
            toml_str(d.split(':').next().unwrap())
        ));
    }
    let restart = match get("Restart").as_deref() {
        Some("always") if kind == "longrun" => Some("always"),
        Some("on-failure" | "on-abnormal" | "on-abort" | "on-watchdog" | "always") => {
            Some("on-failure")
        }
        Some("no") | None => Some("never"),
        Some("on-success") => Some("never"),
        Some(_) => None,
    };
    if let Some(r) = restart {
        lines.push(format!("restart = \"{r}\""));
    }
    if let Some(ms) = get("RestartSec").and_then(|s| timespan(&s)) {
        lines.push(format!("restart_delay_ms = {}", (ms * 1000).max(100)));
    }
    if let Some(s) = get("TimeoutStopSec")
        .or_else(|| get("TimeoutSec"))
        .and_then(|s| timespan(&s))
    {
        lines.push(format!("stop_timeout_ms = {}", s.clamp(1, 600) * 1000));
    }
    if let Some(sig) = get("KillSignal") {
        if crate::config::parse_signal(&sig).is_some() {
            lines.push(format!("stop_signal = {}", toml_str(&sig)));
        }
    }
    if ty == "notify" || ty == "notify-reload" {
        let t = get("TimeoutStartSec")
            .or_else(|| get("TimeoutSec"))
            .and_then(|s| timespan(&s))
            .unwrap_or(90);
        lines.push(format!("ready_timeout_ms = {}", t.clamp(1, 3600) * 1000));
    }
    // Dependencies (unit names; resolved by the caller).
    let mut deps_hard = Vec::new();
    for k in ["Requires", "BindsTo", "Requisite"] {
        deps_hard.extend(u.words("Unit", k));
    }
    let mut deps_soft = u.words("Unit", "Wants");
    deps_soft.extend(u.words("Unit", "After"));
    Ok(Translation {
        kind,
        command,
        lines,
        deps_hard,
        deps_soft,
        environment,
        notes,
    })
}

fn is_true(v: &str) -> bool {
    matches!(v.to_ascii_lowercase().as_str(), "1" | "yes" | "true" | "on")
}

/// Unit names linked into `<target>.wants/` for the given targets.
pub fn wanted_by(dir: &Path, targets: &[&str]) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for t in targets {
        if let Ok(rd) = std::fs::read_dir(dir.join(format!("{t}.wants"))) {
            for e in rd.flatten() {
                out.insert(e.file_name().to_string_lossy().into_owned());
            }
        }
    }
    out
}

/// Import the boot-time units of a systemd unit directory.
pub fn import(dir: &Path, ctx: &Context) -> Report {
    let mut report = Report::default();
    let roots = wanted_by(dir, BOOT_TARGETS);
    let sockets = wanted_by(dir, &["sockets.target"]);
    let timers = wanted_by(dir, &["timers.target"]);

    // Socket activation: service unit -> (paths, mode, accept).
    let mut listen: BTreeMap<String, (Vec<String>, String, bool)> = BTreeMap::new();
    for s in &sockets {
        if ctx.skip.contains(s) || s.starts_with("systemd-") {
            continue;
        }
        let Some(u) = load(dir, s) else { continue };
        let accept = u
            .get("Socket", "Accept")
            .map(|v| is_true(&v))
            .unwrap_or(false);
        let stem = s.trim_end_matches(".socket");
        let svc = u.get("Socket", "Service").unwrap_or_else(|| {
            if accept {
                format!("{stem}@.service")
            } else {
                format!("{stem}.service")
            }
        });
        if ctx.skip.contains(&svc) || ctx.aliases.contains_key(&svc) {
            continue;
        }
        let streams = u.list("Socket", "ListenStream");
        let others: Vec<String> = [
            "ListenDatagram",
            "ListenSequentialPacket",
            "ListenFIFO",
            "ListenNetlink",
        ]
        .iter()
        .flat_map(|k| u.list("Socket", k))
        .collect();
        if streams.is_empty() && others.is_empty() {
            // An empty (disabled) socket unit.
            continue;
        }
        let paths: Vec<String> = streams
            .iter()
            .map(|p| specifiers(p, s, "/root"))
            .filter(|p| p.starts_with('/'))
            .collect();
        if !others.is_empty() || paths.len() != streams.len() || paths.is_empty() {
            report
                .unsupported
                .push((s.clone(), "network or non-stream sockets".into()));
            continue;
        }
        let mode = u
            .get("Socket", "SocketMode")
            .unwrap_or_else(|| "0666".into());
        listen.insert(svc, (paths, mode, accept));
    }

    // Timers: service -> timer.
    let mut timed: BTreeMap<String, Timer> = BTreeMap::new();
    for t in &timers {
        if ctx.skip.contains(t) || t.starts_with("systemd-") {
            continue;
        }
        let Some(u) = load(dir, t) else { continue };
        let svc = u
            .get("Timer", "Unit")
            .unwrap_or_else(|| format!("{}.service", t.trim_end_matches(".timer")));
        let interval = u
            .get("Timer", "OnUnitActiveSec")
            .or_else(|| u.get("Timer", "OnUnitInactiveSec"))
            .and_then(|s| timespan(&s))
            .or_else(|| u.get("Timer", "OnCalendar").map(|c| calendar_period(&c)));
        let boot = u
            .get("Timer", "OnBootSec")
            .or_else(|| u.get("Timer", "OnStartupSec"))
            .and_then(|s| timespan(&s));
        let Some(interval) = interval.or(boot.map(|_| 0)) else {
            report.unsupported.push((
                t.clone(),
                "timer without OnCalendar=/OnUnitActiveSec=/OnBootSec=".into(),
            ));
            continue;
        };
        timed.insert(
            svc.clone(),
            Timer {
                name: t.trim_end_matches(".timer").to_string(),
                service: r6_name(&svc),
                interval_s: interval,
                boot_delay_s: boot.unwrap_or(300),
                persistent: u
                    .get("Timer", "Persistent")
                    .map(|v| is_true(&v))
                    .unwrap_or(false),
            },
        );
    }

    // Services to import: boot roots, socket-activated, timer-driven, then whatever
    // they require.
    let mut queue: Vec<(String, bool)> = Vec::new();
    for r in &roots {
        if r.ends_with(".service") {
            queue.push((r.clone(), true));
        }
    }
    for s in listen.keys() {
        queue.push((s.clone(), true));
    }
    for s in timed.keys() {
        queue.push((s.clone(), false));
    }
    let mut done: BTreeMap<String, (Translation, bool)> = BTreeMap::new();
    let mut failed: BTreeSet<String> = BTreeSet::new();
    while let Some((unit, enable)) = queue.pop() {
        // systemd's own units: r6 has its own equivalents (or they do not apply).
        if ctx.skip.contains(&unit)
            || ctx.aliases.contains_key(&unit)
            || unit.starts_with("systemd-")
        {
            continue;
        }
        if let Some((_, en)) = done.get_mut(&unit) {
            *en |= enable;
            continue;
        }
        if failed.contains(&unit) || ctx.existing.contains(&r6_name(&unit)) {
            continue;
        }
        let Some(u) = load(dir, &unit) else {
            // Units required but not present are ignored, like systemd does for Wants=.
            continue;
        };
        match translate(&unit, &u, ctx) {
            Ok(t) => {
                for d in &t.deps_hard {
                    if d.ends_with(".service") {
                        queue.push((d.clone(), enable));
                    }
                }
                for d in u.words("Unit", "Wants") {
                    if d.ends_with(".service") {
                        queue.push((d, enable));
                    }
                }
                done.insert(unit, (t, enable));
            }
            Err(reason) => {
                failed.insert(unit.clone());
                report.unsupported.push((unit, reason));
            }
        }
    }

    // Timer-driven services become oneshots started by r6-timers.
    for (svc, t) in &timed {
        if done.contains_key(svc) || ctx.existing.contains(&t.service) {
            report.timers.push(t.clone());
        } else if !failed.contains(svc) {
            report
                .unsupported
                .push((format!("{}.timer", t.name), format!("{svc} not found")));
        }
    }

    // Resolve dependencies to r6 names.
    let imported: BTreeSet<String> = done.keys().map(|u| r6_name(u)).collect();
    let resolve = |d: &str, hard: bool, enabled: &BTreeMap<String, bool>| -> Option<String> {
        if let Some(a) = ctx.aliases.get(d) {
            return Some(a.clone());
        }
        let n = r6_name(d);
        if !d.ends_with(".service") {
            return None;
        }
        if imported.contains(&n) {
            // Ordering after an imported unit that is not started at boot would start
            // it (r6 wants pull services in); only hard dependencies may do that.
            if hard || enabled.get(d).copied().unwrap_or(false) {
                return Some(n);
            }
            return None;
        }
        if ctx.existing.contains(&n) && (hard || ctx.existing_enabled.contains(&n)) {
            return Some(n);
        }
        None
    };
    let enabled: BTreeMap<String, bool> = done.iter().map(|(k, (_, e))| (k.clone(), *e)).collect();
    for (unit, (t, enable)) in &done {
        let name = r6_name(unit);
        let mut depends: Vec<String> = t
            .deps_hard
            .iter()
            .filter_map(|d| resolve(d, true, &enabled))
            .filter(|d| d != &name)
            .collect();
        depends.sort();
        depends.dedup();
        let mut wants: Vec<String> = ctx.base_wants.clone();
        wants.extend(
            t.deps_soft
                .iter()
                .filter_map(|d| resolve(d, false, &enabled)),
        );
        wants.retain(|w| w != &name && !depends.contains(w));
        wants.sort();
        wants.dedup();
        let mut text = String::new();
        let desc = load(dir, unit)
            .and_then(|u| u.get("Unit", "Description"))
            .unwrap_or_default();
        let desc = if desc.is_empty() {
            format!("{unit} (imported)")
        } else {
            format!("{desc} (imported from {unit})")
        };
        text.push_str(&format!("description = {}\n", toml_str(&desc)));
        text.push_str(&format!("command = {}\n", toml_list(&t.command)));
        let accept = listen.get(unit).map(|l| l.2).unwrap_or(false);
        // inetd-style instances are spawned per connection by a listening service.
        let kind = if accept { "longrun" } else { t.kind };
        text.push_str(&format!("type = \"{kind}\"\n"));
        if !depends.is_empty() {
            text.push_str(&format!("depends = {}\n", toml_list(&depends)));
        }
        if !wants.is_empty() {
            text.push_str(&format!("wants = {}\n", toml_list(&wants)));
        }
        let mut lines = t.lines.clone();
        if let Some((paths, mode, accept)) = listen.get(unit) {
            if *accept {
                lines.push("accept = true".into());
            }
            if kind == "longrun" {
                lines.push(format!("listen = {}", toml_list(paths)));
                lines.push(format!(
                    "listen_mode = {}",
                    toml_str(mode.trim_start_matches('0'))
                ));
            }
        }
        for l in &lines {
            text.push_str(l);
            text.push('\n');
        }
        text.push_str(&format!(
            "enable = {}\n",
            *enable && !timed.contains_key(unit)
        ));
        if !t.environment.is_empty() {
            text.push_str("\n[environment]\n");
            for (k, v) in &t.environment {
                text.push_str(&format!("{} = {}\n", toml_str(k), toml_str(v)));
            }
        }
        report.imported.push(Imported {
            name,
            toml: text,
            notes: t.notes.clone(),
        });
    }
    report.unsupported.sort();
    report.unsupported.dedup();
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(files: &[(&str, &str)]) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        for (p, text) in files {
            let path = d.path().join(p);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            if let Some(target) = text.strip_prefix("->") {
                std::os::unix::fs::symlink(target, &path).unwrap();
            } else {
                std::fs::write(&path, text).unwrap();
            }
        }
        d
    }

    fn ctx() -> Context {
        let mut c = Context::default();
        c.aliases
            .insert("network.target".into(), "networkmanager".into());
        c.aliases.insert("dbus.service".into(), "dbus".into());
        c.existing
            .extend(["networkmanager".to_string(), "dbus".to_string()]);
        c.existing_enabled
            .extend(["networkmanager".to_string(), "dbus".to_string()]);
        c.base_wants = vec!["filesystems".into()];
        c.static_users.insert("cups".into());
        c
    }

    #[test]
    fn parses_units_with_resets_and_continuations() {
        let u = UnitFile::parse(
            "[Unit]\nAfter=a.service b.service\n# c\n[Service]\nExecStart=/bin/x \\\n  --flag\nExecStart=\nExecStart=/bin/y\nEnvironment=A=1\n; x\n",
        );
        assert_eq!(u.words("Unit", "After"), ["a.service", "b.service"]);
        assert_eq!(u.list("Service", "ExecStart"), ["/bin/y"]);
        assert_eq!(u.get("Service", "Environment").as_deref(), Some("A=1"));
    }

    #[test]
    fn command_lines_and_specifiers() {
        assert_eq!(
            split_command(r#"/bin/a "b c" 'd e' f\ g"#),
            ["/bin/a", "b c", "d e", "f g"]
        );
        let e = parse_exec("-@/bin/real argv0 x").unwrap();
        assert!(e.ignore_failure);
        assert_eq!(e.argv0.as_deref(), Some("argv0"));
        assert_eq!(e.argv, ["/bin/real", "x"]);
        assert_eq!(
            specifiers("%t/%i/%N/%%", "getty@tty1.service", "/root"),
            "/run/tty1/getty@tty1/%"
        );
    }

    #[test]
    fn time_spans_and_calendar_periods() {
        assert_eq!(timespan("5min 30s"), Some(330));
        assert_eq!(timespan("1h"), Some(3600));
        assert_eq!(timespan("90"), Some(90));
        assert_eq!(timespan("infinity"), None);
        assert_eq!(calendar_period("weekly"), 604800);
        assert_eq!(calendar_period("03:15"), 86400);
        assert_eq!(calendar_period("*-*-* 04:00:00"), 86400);
        assert_eq!(calendar_period("*:0/15"), 900);
        assert_eq!(calendar_period("*-*-* *:00:00"), 3600);
        assert_eq!(calendar_period("Mon *-*-* 00:00:00"), 604800);
        assert_eq!(calendar_period("*-*-01 00:00:00"), 2629800);
    }

    #[test]
    fn imports_services_sockets_and_timers() {
        let d = tree(&[
            // A package unit with a NixOS drop-in, enabled at boot.
            ("pkg/tailscaled.service", "[Unit]\nDescription=Tailscale\nAfter=network.target dbus.service\nWants=network.target\n[Service]\nType=notify\nExecStart=/nix/store/x/bin/tailscaled --port=${PORT}\nRestart=on-failure\nRuntimeDirectory=tailscale\nStateDirectory=tailscale\n"),
            ("tailscaled.service", "->pkg/tailscaled.service"),
            ("tailscaled.service.d/overrides.conf", "[Service]\nEnvironment=\"PORT=41641\"\nEnvironmentFile=-/etc/default/tailscaled\n"),
            ("multi-user.target.wants/tailscaled.service", "->../tailscaled.service"),
            // Socket-activated.
            ("cups.socket", "[Socket]\nListenStream=/run/cups/cups.sock\nSocketMode=0666\n"),
            ("cups.service", "[Unit]\nRequires=cups-setup.service\n[Service]\nExecStart=/bin/cupsd -l\nUser=cups\nDynamicUser=yes\n"),
            ("cups-setup.service", "[Service]\nType=oneshot\nExecStart=/bin/setup\n"),
            ("sockets.target.wants/cups.socket", "->../cups.socket"),
            // inetd-style socket.
            ("helper.socket", "[Socket]\nAccept=yes\nListenStream=/run/helper.sock\n"),
            ("helper@.service", "[Service]\nType=oneshot\nExecStart=/bin/helper --socket-activated\nStandardInput=socket\n"),
            ("sockets.target.wants/helper.socket", "->../helper.socket"),
            ("empty.socket", ""),
            ("sockets.target.wants/empty.socket", "->../empty.socket"),
            // Timer.
            ("gc.timer", "[Timer]\nOnCalendar=03:15\nPersistent=true\n"),
            ("gc.service", "[Service]\nType=oneshot\nExecStart=/bin/gc\n"),
            ("timers.target.wants/gc.timer", "->../gc.timer"),
            // Unsupported.
            ("old.service", "[Service]\nType=forking\nExecStart=/bin/old -d\n"),
            ("multi-user.target.wants/old.service", "->../old.service"),
            ("net.socket", "[Socket]\nListenStream=0.0.0.0:80\n"),
            ("sockets.target.wants/net.socket", "->../net.socket"),
            // Template instance.
            ("worker@.service", "[Service]\nExecStart=/bin/worker %i\n"),
            ("multi-user.target.wants/worker@a.service", "->../worker@.service"),
        ]);
        let rep = import(d.path(), &ctx());
        let get = |n: &str| {
            rep.imported
                .iter()
                .find(|i| i.name == n)
                .map(|i| i.toml.clone())
        };
        let ts = get("tailscaled").expect("tailscaled imported");
        assert!(ts.contains("ready = \"sd-notify\""), "{ts}");
        assert!(ts.contains("runtime_directory = \"tailscale\""), "{ts}");
        assert!(
            ts.contains("wants = [\"dbus\", \"filesystems\", \"networkmanager\"]"),
            "{ts}"
        );
        assert!(ts.contains("/etc/default/tailscaled"), "{ts}");
        assert!(ts.contains("\\\"--port=${PORT}\\\""), "{ts}");
        assert!(ts.contains("[environment]\n\"PORT\" = \"41641\""), "{ts}");
        assert!(ts.contains("enable = true"), "{ts}");
        let cups = get("cups").expect("cups imported");
        assert!(
            cups.contains("listen = [\"/run/cups/cups.sock\"]"),
            "{cups}"
        );
        assert!(cups.contains("listen_mode = \"666\""), "{cups}");
        assert!(cups.contains("depends = [\"cups-setup\"]"), "{cups}");
        assert!(cups.contains("user = \"cups\""), "{cups}");
        assert!(get("cups-setup").unwrap().contains("type = \"oneshot\""));
        let helper = get("helper").expect("helper imported");
        assert!(helper.contains("accept = true"), "{helper}");
        assert!(helper.contains("type = \"longrun\""), "{helper}");
        assert!(
            helper.contains("listen = [\"/run/helper.sock\"]"),
            "{helper}"
        );
        let gc = get("gc").expect("gc imported");
        assert!(gc.contains("enable = false"), "{gc}");
        assert_eq!(rep.timers.len(), 1);
        assert_eq!(rep.timers[0].service, "gc");
        assert_eq!(rep.timers[0].interval_s, 86400);
        assert!(rep.timers[0].persistent);
        assert!(get("worker@a")
            .unwrap()
            .contains("[\"/bin/worker\", \"a\"]"));
        let unsupported: Vec<&str> = rep.unsupported.iter().map(|(u, _)| u.as_str()).collect();
        assert_eq!(unsupported, ["net.socket", "old.service"]);
        // Every imported definition is valid r6 configuration.
        for i in &rep.imported {
            crate::config::Service::parse(&i.name, &i.toml, Path::new("x")).unwrap();
        }
    }
}
