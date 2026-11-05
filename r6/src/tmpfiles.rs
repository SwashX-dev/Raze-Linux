//! r6-tmpfiles: create, adjust and remove volatile files from `tmpfiles.d(5)` rules.
//!
//! NixOS (and the packages it ships) declare boot-time directories, symlinks, device
//! nodes and sysfs writes as tmpfiles.d rules. Without systemd something has to
//! apply them; this is that implementation.
//!
//! Supported line types: `f F w d D e v q Q p L c b C x X r R z Z`, with the `+`
//! (replace), `!` (boot only) and `-` (ignore errors) modifiers, octal modes with the
//! `~` (mask) and `:` (creation only) prefixes, numeric or named owners, and the
//! common `%` specifiers. Not supported (reported, then skipped): extended attributes
//! (`t T`), file attributes (`h H`), ACLs (`a A`), age-based cleaning (`--clean`).

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::{symlink, DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub kind: char,
    pub plus: bool,
    pub boot_only: bool,
    pub ignore_errors: bool,
    /// `?` on L/C: only act if the link target / copy source exists.
    pub only_if_source: bool,
    pub path: PathBuf,
    pub mode: Option<Mode>,
    pub user: Option<String>,
    pub group: Option<String>,
    pub argument: Option<String>,
    pub origin: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mode {
    pub bits: u32,
    /// `~`: mask the bits with the existing/derived mode.
    pub masked: bool,
    /// `:`: apply only when creating.
    pub create_only: bool,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Options {
    pub create: bool,
    pub remove: bool,
    pub boot: bool,
}

#[derive(Debug, Default)]
pub struct Report {
    pub applied: usize,
    pub skipped: usize,
    pub errors: Vec<String>,
}

pub const DEFAULT_DIRS: [&str; 3] = ["/etc/tmpfiles.d", "/run/tmpfiles.d", "/usr/lib/tmpfiles.d"];

/// Split a line into fields, honouring single and double quotes (NixOS quotes every
/// field with single quotes) and backslash escapes. The sixth separator ends field
/// splitting: the argument is the rest of the line, with surrounding quotes removed.
fn fields(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut have = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if quote != Some('\'') => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                    have = true;
                }
            }
            '"' | '\'' if quote.is_none() => {
                quote = Some(c);
                have = true;
            }
            c if Some(c) == quote => quote = None,
            c if c.is_whitespace() && quote.is_none() => {
                if have {
                    out.push(std::mem::take(&mut cur));
                    have = false;
                }
                if out.len() == 6 {
                    let rest: String = chars.collect();
                    let rest = rest.trim();
                    let unquoted = ['\'', '"']
                        .iter()
                        .find_map(|q| rest.strip_prefix(*q).and_then(|r| r.strip_suffix(*q)))
                        .unwrap_or(rest);
                    if !rest.is_empty() {
                        out.push(unquoted.to_string());
                    }
                    return out;
                }
            }
            c => {
                cur.push(c);
                have = true;
            }
        }
    }
    if have {
        out.push(cur);
    }
    out
}

fn dash(s: Option<&String>) -> Option<String> {
    s.filter(|v| v.as_str() != "-").cloned()
}

pub fn parse_mode(s: &str) -> Option<Mode> {
    let mut m = Mode {
        bits: 0,
        masked: false,
        create_only: false,
    };
    let mut s = s;
    loop {
        if let Some(r) = s.strip_prefix('~') {
            m.masked = true;
            s = r;
        } else if let Some(r) = s.strip_prefix(':') {
            m.create_only = true;
            s = r;
        } else {
            break;
        }
    }
    m.bits = u32::from_str_radix(s, 8).ok().filter(|b| *b <= 0o7777)?;
    Some(m)
}

/// Expand `%` specifiers.
pub fn expand(s: &str, spec: &BTreeMap<char, String>) -> Result<String, String> {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('%') => out.push('%'),
            Some(k) => match spec.get(&k) {
                Some(v) => out.push_str(v),
                None => return Err(format!("unsupported specifier %{k}")),
            },
            None => return Err("trailing %".into()),
        }
    }
    Ok(out)
}

pub fn specifiers() -> BTreeMap<char, String> {
    let read = |p: &str| {
        fs::read_to_string(p)
            .map(|s| s.trim().to_string())
            .unwrap_or_default()
    };
    let host = read("/proc/sys/kernel/hostname");
    let mut m = BTreeMap::new();
    m.insert(
        'b',
        read("/proc/sys/kernel/random/boot_id").replace('-', ""),
    );
    m.insert('m', read("/etc/machine-id"));
    m.insert('H', host.clone());
    m.insert('l', host.split('.').next().unwrap_or("").to_string());
    m.insert('v', read("/proc/sys/kernel/osrelease"));
    m.insert('a', std::env::consts::ARCH.replace('_', "-"));
    m.insert('t', "/run".into());
    m.insert('S', "/var/lib".into());
    m.insert('C', "/var/cache".into());
    m.insert('L', "/var/log".into());
    m.insert('T', "/tmp".into());
    m.insert('V', "/var/tmp".into());
    m.insert('h', "/root".into());
    m.insert('u', "root".into());
    m.insert('U', "0".into());
    m.insert('g', "root".into());
    m.insert('G', "0".into());
    m
}

pub fn parse_line(
    line: &str,
    origin: &str,
    spec: &BTreeMap<char, String>,
) -> Result<Option<Rule>, String> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return Ok(None);
    }
    let f = fields(line);
    if f.len() < 2 {
        return Err(format!("{origin}: too few fields"));
    }
    let mut chars = f[0].chars();
    let kind = chars
        .next()
        .ok_or_else(|| format!("{origin}: empty type"))?;
    let mut rule = Rule {
        kind,
        plus: false,
        boot_only: false,
        ignore_errors: false,
        only_if_source: false,
        path: PathBuf::new(),
        mode: None,
        user: None,
        group: None,
        argument: None,
        origin: origin.into(),
    };
    for m in chars {
        match m {
            '+' => rule.plus = true,
            '!' => rule.boot_only = true,
            '-' => rule.ignore_errors = true,
            '?' => rule.only_if_source = true,
            '=' | '^' | '~' | '$' => {}
            other => return Err(format!("{origin}: unknown modifier '{other}'")),
        }
    }
    let path = expand(&f[1], spec).map_err(|e| format!("{origin}: {e}"))?;
    if !path.starts_with('/') {
        return Err(format!("{origin}: path must be absolute"));
    }
    rule.path = PathBuf::from(path);
    rule.mode = match dash(f.get(2)) {
        None => None,
        Some(m) => Some(parse_mode(&m).ok_or_else(|| format!("{origin}: invalid mode '{m}'"))?),
    };
    rule.user = dash(f.get(3));
    rule.group = dash(f.get(4));
    // f[5] is the age (cleaning is not implemented).
    rule.argument = match dash(f.get(6)) {
        None => None,
        Some(a) => Some(expand(&a, spec).map_err(|e| format!("{origin}: {e}"))?),
    };
    Ok(Some(rule))
}

/// Collect rules from the configuration directories. A file in an earlier directory
/// overrides one of the same name in a later directory; files are applied in name order.
pub fn load(dirs: &[PathBuf], spec: &BTreeMap<char, String>, report: &mut Report) -> Vec<Rule> {
    let mut files: BTreeMap<String, PathBuf> = BTreeMap::new();
    for d in dirs {
        let Ok(rd) = fs::read_dir(d) else { continue };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.ends_with(".conf") {
                files.entry(name).or_insert_with(|| e.path());
            }
        }
    }
    let mut rules = Vec::new();
    for (_, path) in files {
        // /dev/null symlinks mask a file, as with systemd.
        if fs::read_link(&path)
            .map(|t| t == Path::new("/dev/null"))
            .unwrap_or(false)
        {
            continue;
        }
        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                report.errors.push(format!("{}: {e}", path.display()));
                continue;
            }
        };
        for (n, line) in text.lines().enumerate() {
            let origin = format!("{}:{}", path.display(), n + 1);
            match parse_line(line, &origin, spec) {
                Ok(Some(r)) => rules.push(r),
                Ok(None) => {}
                Err(e) => report.errors.push(e),
            }
        }
    }
    rules
}

fn resolve_ids(rule: &Rule) -> (Option<u32>, Option<u32>) {
    let uid = rule
        .user
        .as_deref()
        .and_then(|u| crate::sys::lookup_user(u).map(|(uid, _)| uid));
    let gid = rule.group.as_deref().and_then(crate::sys::lookup_group);
    (uid, gid)
}

fn chown(path: &Path, uid: Option<u32>, gid: Option<u32>, follow: bool) -> io::Result<()> {
    if uid.is_none() && gid.is_none() {
        return Ok(());
    }
    let meta = if follow {
        fs::metadata(path)?
    } else {
        fs::symlink_metadata(path)?
    };
    let (u, g) = (uid.unwrap_or(meta.uid()), gid.unwrap_or(meta.gid()));
    if u == meta.uid() && g == meta.gid() {
        return Ok(());
    }
    if follow {
        std::os::unix::fs::chown(path, Some(u), Some(g))
    } else {
        std::os::unix::fs::lchown(path, Some(u), Some(g))
    }
}

fn chmod(path: &Path, mode: Option<Mode>, created: bool, default: u32) -> io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() {
        return Ok(());
    }
    let want = match mode {
        None if !created => return Ok(()),
        None => default,
        Some(m) if m.create_only && !created => return Ok(()),
        Some(m) if m.masked => {
            let cur = meta.mode() & 0o7777;
            cur & m.bits
        }
        Some(m) => m.bits,
    };
    if meta.mode() & 0o7777 != want {
        fs::set_permissions(path, fs::Permissions::from_mode(want))?;
    }
    Ok(())
}

fn adjust(rule: &Rule, path: &Path, created: bool, default: u32) -> io::Result<()> {
    let (uid, gid) = resolve_ids(rule);
    if created || rule.user.is_some() || rule.group.is_some() {
        chown(path, uid, gid, false)?;
    }
    chmod(path, rule.mode, created, default)
}

fn adjust_recursive(rule: &Rule, path: &Path) -> io::Result<()> {
    adjust(rule, path, false, 0)?;
    let meta = fs::symlink_metadata(path)?;
    if meta.is_dir() {
        for e in fs::read_dir(path)? {
            adjust_recursive(rule, &e?.path())?;
        }
    }
    Ok(())
}

fn mkdir_parents(path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o755)
            .create(parent)?;
    }
    Ok(())
}

fn remove_contents(dir: &Path) -> io::Result<()> {
    match fs::read_dir(dir) {
        Ok(rd) => {
            for e in rd {
                let p = e?.path();
                if fs::symlink_metadata(&p)?.is_dir() {
                    fs::remove_dir_all(&p)?;
                } else {
                    fs::remove_file(&p)?;
                }
            }
            Ok(())
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

fn exists(p: &Path) -> bool {
    fs::symlink_metadata(p).is_ok()
}

fn copy_recursive(src: &Path, dst: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(src)?;
    if meta.file_type().is_symlink() {
        symlink(fs::read_link(src)?, dst)
    } else if meta.is_dir() {
        fs::DirBuilder::new()
            .mode(meta.mode() & 0o7777)
            .create(dst)?;
        for e in fs::read_dir(src)? {
            let e = e?;
            copy_recursive(&e.path(), &dst.join(e.file_name()))?;
        }
        Ok(())
    } else {
        fs::copy(src, dst)?;
        fs::set_permissions(dst, fs::Permissions::from_mode(meta.mode() & 0o7777))
    }
}

fn mknod(path: &Path, kind: char, arg: Option<&str>, mode: u32) -> io::Result<()> {
    let bad = || {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "device argument must be MAJOR:MINOR",
        )
    };
    let (maj, min) = arg.and_then(|a| a.split_once(':')).ok_or_else(bad)?;
    let (maj, min): (u32, u32) = (
        maj.parse().map_err(|_| bad())?,
        min.parse().map_err(|_| bad())?,
    );
    let c = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).map_err(|_| bad())?;
    let t = if kind == 'c' {
        libc::S_IFCHR
    } else {
        libc::S_IFBLK
    };
    let r = unsafe {
        libc::mknod(
            c.as_ptr(),
            t | mode as libc::mode_t,
            libc::makedev(maj, min),
        )
    };
    if r < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn apply_create(rule: &Rule) -> io::Result<bool> {
    let p = &rule.path;
    let arg = rule.argument.as_deref();
    match rule.kind {
        'f' | 'F' => {
            let replace = rule.plus || rule.kind == 'F';
            let created = !exists(p);
            if created || replace {
                mkdir_parents(p)?;
                let mut f = fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .mode(0o644)
                    .open(p)?;
                if let Some(a) = arg {
                    f.write_all(a.as_bytes())?;
                }
            }
            adjust(rule, p, created, 0o644)?;
            Ok(true)
        }
        'w' => {
            if !exists(p) {
                return Ok(false);
            }
            let mut f = fs::OpenOptions::new()
                .write(true)
                .append(rule.plus)
                .truncate(!rule.plus)
                .open(p)?;
            f.write_all(arg.unwrap_or("").as_bytes())?;
            Ok(true)
        }
        'd' | 'D' | 'v' | 'q' | 'Q' => {
            let created = !exists(p);
            if created {
                fs::DirBuilder::new()
                    .recursive(true)
                    .mode(0o755)
                    .create(p)?;
            }
            adjust(rule, p, created, 0o755)?;
            Ok(true)
        }
        'e' => {
            if exists(p) {
                adjust(rule, p, false, 0o755)?;
                return Ok(true);
            }
            Ok(false)
        }
        'p' => {
            if exists(p) && !rule.plus {
                return Ok(false);
            }
            let _ = fs::remove_file(p);
            mkdir_parents(p)?;
            let c = std::ffi::CString::new(p.as_os_str().as_encoded_bytes())
                .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
            if unsafe { libc::mkfifo(c.as_ptr(), 0o644) } < 0 {
                return Err(io::Error::last_os_error());
            }
            adjust(rule, p, true, 0o644)?;
            Ok(true)
        }
        'L' => {
            let target = match arg {
                Some(t) => PathBuf::from(t),
                // Without an argument, link to the same path below /usr/share/factory.
                None => Path::new("/usr/share/factory").join(p.strip_prefix("/").unwrap_or(p)),
            };
            if rule.only_if_source && !exists(&target) {
                return Ok(false);
            }
            if exists(p) {
                if fs::read_link(p).map(|t| t == target).unwrap_or(false) || !rule.plus {
                    return Ok(false);
                }
                if fs::symlink_metadata(p)?.is_dir() {
                    fs::remove_dir_all(p)?;
                } else {
                    fs::remove_file(p)?;
                }
            }
            mkdir_parents(p)?;
            symlink(&target, p)?;
            let (uid, gid) = resolve_ids(rule);
            chown(p, uid, gid, false)?;
            Ok(true)
        }
        'c' | 'b' => {
            if exists(p) {
                if !rule.plus {
                    return Ok(false);
                }
                fs::remove_file(p)?;
            }
            mkdir_parents(p)?;
            mknod(
                p,
                rule.kind,
                arg,
                rule.mode.map(|m| m.bits).unwrap_or(0o644),
            )?;
            adjust(rule, p, true, 0o644)?;
            Ok(true)
        }
        'C' => {
            let src = match arg {
                Some(s) => PathBuf::from(s),
                None => Path::new("/usr/share/factory").join(p.strip_prefix("/").unwrap_or(p)),
            };
            if exists(p) {
                if !rule.plus {
                    return Ok(false);
                }
                if fs::symlink_metadata(p)?.is_dir() {
                    fs::remove_dir_all(p)?;
                } else {
                    fs::remove_file(p)?;
                }
            }
            if !exists(&src) {
                return Ok(false);
            }
            mkdir_parents(p)?;
            copy_recursive(&src, p)?;
            Ok(true)
        }
        'z' => {
            if exists(p) {
                adjust(rule, p, false, 0)?;
                return Ok(true);
            }
            Ok(false)
        }
        'Z' => {
            if exists(p) {
                adjust_recursive(rule, p)?;
                return Ok(true);
            }
            Ok(false)
        }
        _ => Ok(false),
    }
}

fn apply_remove(rule: &Rule) -> io::Result<bool> {
    let p = &rule.path;
    match rule.kind {
        'r' => match fs::symlink_metadata(p) {
            Ok(m) if m.is_dir() => fs::remove_dir(p).map(|_| true),
            Ok(_) => fs::remove_file(p).map(|_| true),
            Err(_) => Ok(false),
        },
        'R' => match fs::symlink_metadata(p) {
            Ok(m) if m.is_dir() => fs::remove_dir_all(p).map(|_| true),
            Ok(_) => fs::remove_file(p).map(|_| true),
            Err(_) => Ok(false),
        },
        'D' => remove_contents(p).map(|_| true),
        _ => Ok(false),
    }
}

/// Apply rules. Removal runs first so that `D`/`R` + recreate sequences work.
pub fn apply(rules: &[Rule], opts: Options, report: &mut Report) {
    let active = |r: &&Rule| opts.boot || !r.boot_only;
    if opts.remove {
        for r in rules.iter().filter(active) {
            match apply_remove(r) {
                Ok(true) => report.applied += 1,
                Ok(false) => {}
                Err(e) if r.ignore_errors => {
                    report.skipped += usize::from(e.kind() != io::ErrorKind::NotFound)
                }
                Err(e) => report
                    .errors
                    .push(format!("{}: {}: {e}", r.origin, r.path.display())),
            }
        }
    }
    if opts.create {
        for r in rules.iter().filter(active) {
            if matches!(r.kind, 't' | 'T' | 'h' | 'H' | 'a' | 'A') {
                report.skipped += 1;
                continue;
            }
            match apply_create(r) {
                Ok(true) => report.applied += 1,
                Ok(false) => {}
                Err(_) if r.ignore_errors => report.skipped += 1,
                Err(e) => report
                    .errors
                    .push(format!("{}: {}: {e}", r.origin, r.path.display())),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::FileTypeExt;

    fn spec() -> BTreeMap<char, String> {
        let mut s = specifiers();
        s.insert('m', "abc".into());
        s
    }

    #[test]
    fn parses_fields_modes_and_specifiers() {
        let r = parse_line("d! /run/%m/x 0750 - - 10d", "t", &spec())
            .unwrap()
            .unwrap();
        assert_eq!(r.kind, 'd');
        assert!(r.boot_only);
        assert_eq!(r.path, Path::new("/run/abc/x"));
        assert_eq!(
            r.mode,
            Some(Mode {
                bits: 0o750,
                masked: false,
                create_only: false
            })
        );
        let r = parse_line(
            r#"f+ "/tmp/a b" ~0644 root root - hello world"#,
            "t",
            &spec(),
        )
        .unwrap()
        .unwrap();
        assert!(r.plus);
        assert_eq!(r.path, Path::new("/tmp/a b"));
        assert!(r.mode.unwrap().masked);
        assert_eq!(r.argument.as_deref(), Some("hello world"));
        assert!(parse_line("# comment", "t", &spec()).unwrap().is_none());
        assert!(parse_line("d relative", "t", &spec()).is_err());
        assert!(parse_line("d /x 0999", "t", &spec()).is_err());
        assert!(parse_line("d /%Q", "t", &spec()).is_err());
    }

    #[test]
    fn parses_nixos_single_quoted_rules() {
        // Verbatim lines from NixOS-generated tmpfiles.d files.
        let r = parse_line(
            "d '/var/cache/tuigreet' - greeter greeter - -",
            "t",
            &spec(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(r.path, Path::new("/var/cache/tuigreet"));
        assert_eq!(r.user.as_deref(), Some("greeter"));
        assert_eq!(r.argument, None);
        let r = parse_line(
            "'L+' '/run/opengl-driver' '-' '-' '-' '-' /nix/store/x-graphics-drivers",
            "t",
            &spec(),
        )
        .unwrap()
        .unwrap();
        assert_eq!((r.kind, r.plus), ('L', true));
        assert_eq!(r.path, Path::new("/run/opengl-driver"));
        assert_eq!(r.mode, None);
        assert_eq!(r.argument.as_deref(), Some("/nix/store/x-graphics-drivers"));
        let r = parse_line("'r' '/run/opengl-driver-32' '-' '-' '-' '-' ", "t", &spec())
            .unwrap()
            .unwrap();
        assert_eq!(r.kind, 'r');
        assert_eq!(r.argument, None);
        let r = parse_line(
            "'d' '/etc/NetworkManager/system-connections' '0700' 'root' 'root' '-' ",
            "t",
            &spec(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(r.mode.unwrap().bits, 0o700);
        let r = parse_line("L? /run/systemd/x   - - - - /run/host/x", "t", &spec())
            .unwrap()
            .unwrap();
        assert!(r.only_if_source);
        assert_eq!(r.argument.as_deref(), Some("/run/host/x"));
        let r = parse_line("f /etc/a 0644 - - - 'quoted content'", "t", &spec())
            .unwrap()
            .unwrap();
        assert_eq!(r.argument.as_deref(), Some("quoted content"));
    }

    #[test]
    fn creates_adjusts_and_removes() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().display().to_string();
        let conf = format!(
            "d {root}/dir 0700 - - -\n\
             f {root}/dir/file 0600 - - - content\n\
             L {root}/link - - - - {root}/dir/file\n\
             D {root}/scratch 0755 - - -\n\
             w {root}/dir/file - - - - replaced\n\
             C {root}/copy - - - - {root}/dir\n\
             r {root}/gone\n\
             p {root}/fifo 0600\n"
        );
        fs::create_dir(d.path().join("conf")).unwrap();
        fs::write(d.path().join("conf/test.conf"), conf).unwrap();
        fs::create_dir(d.path().join("scratch")).unwrap();
        fs::write(d.path().join("scratch/old"), "x").unwrap();
        fs::write(d.path().join("gone"), "x").unwrap();
        let mut rep = Report::default();
        let rules = load(&[d.path().join("conf")], &spec(), &mut rep);
        assert_eq!(rules.len(), 8);
        apply(
            &rules,
            Options {
                create: true,
                remove: true,
                boot: true,
            },
            &mut rep,
        );
        assert!(rep.errors.is_empty(), "{:?}", rep.errors);
        let mode = |p: &str| fs::metadata(d.path().join(p)).unwrap().mode() & 0o777;
        assert_eq!(mode("dir"), 0o700);
        assert_eq!(mode("dir/file"), 0o600);
        assert_eq!(
            fs::read_to_string(d.path().join("dir/file")).unwrap(),
            "replaced"
        );
        assert_eq!(
            fs::read_link(d.path().join("link")).unwrap(),
            d.path().join("dir/file")
        );
        assert!(!d.path().join("scratch/old").exists());
        assert!(d.path().join("scratch").is_dir());
        assert!(d.path().join("copy/file").exists());
        assert!(!d.path().join("gone").exists());
        assert!(fs::symlink_metadata(d.path().join("fifo"))
            .unwrap()
            .file_type()
            .is_fifo());

        // Second run is idempotent and leaves existing files alone.
        fs::write(d.path().join("dir/file"), "user edit").unwrap();
        let mut rep2 = Report::default();
        let rules = load(&[d.path().join("conf")], &spec(), &mut rep2);
        apply(
            &rules,
            Options {
                create: true,
                remove: false,
                boot: true,
            },
            &mut rep2,
        );
        assert!(rep2.errors.is_empty(), "{:?}", rep2.errors);
        // `w` still writes (it always does), but `f` without `+` does not truncate.
        assert_eq!(
            fs::read_to_string(d.path().join("dir/file")).unwrap(),
            "replaced"
        );
    }

    #[test]
    fn earlier_directories_override_and_dev_null_masks() {
        let d = tempfile::tempdir().unwrap();
        let (a, b) = (d.path().join("a"), d.path().join("b"));
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        fs::write(b.join("x.conf"), format!("d {}/from-b", d.path().display())).unwrap();
        fs::write(a.join("x.conf"), format!("d {}/from-a", d.path().display())).unwrap();
        fs::write(b.join("y.conf"), "d /should-not-load").unwrap();
        symlink("/dev/null", a.join("y.conf")).unwrap();
        let mut rep = Report::default();
        let rules = load(&[a, b], &spec(), &mut rep);
        assert_eq!(rules.len(), 1);
        assert!(rules[0].path.ends_with("from-a"));
    }

    #[test]
    fn boot_only_rules_need_boot_flag() {
        let d = tempfile::tempdir().unwrap();
        let rule = parse_line(&format!("d! {}/b", d.path().display()), "t", &spec())
            .unwrap()
            .unwrap();
        let mut rep = Report::default();
        apply(
            std::slice::from_ref(&rule),
            Options {
                create: true,
                remove: false,
                boot: false,
            },
            &mut rep,
        );
        assert!(!d.path().join("b").exists());
        apply(
            &[rule],
            Options {
                create: true,
                remove: false,
                boot: true,
            },
            &mut rep,
        );
        assert!(d.path().join("b").exists());
    }
}
