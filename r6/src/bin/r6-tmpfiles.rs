//! r6-tmpfiles: apply tmpfiles.d(5) rules (see r6::tmpfiles for the supported subset).
//!
//! Usage: r6-tmpfiles [--create] [--remove] [--boot] [--dir DIR]... [CONFIG-FILE]...

use std::path::PathBuf;

fn main() {
    let mut opts = r6::tmpfiles::Options::default();
    let mut dirs: Vec<PathBuf> = Vec::new();
    let mut files: Vec<PathBuf> = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--create" => opts.create = true,
            "--remove" => opts.remove = true,
            "--boot" => opts.boot = true,
            "--dir" => dirs.extend(args.next().map(PathBuf::from)),
            "-h" | "--help" => {
                println!(
                    "usage: r6-tmpfiles [--create] [--remove] [--boot] [--dir DIR]... [FILE]..."
                );
                return;
            }
            f => files.push(f.into()),
        }
    }
    if !opts.create && !opts.remove {
        eprintln!("r6-tmpfiles: nothing to do (use --create and/or --remove)");
        std::process::exit(2);
    }
    if dirs.is_empty() {
        dirs = r6::tmpfiles::DEFAULT_DIRS
            .iter()
            .map(PathBuf::from)
            .collect();
    }
    let spec = r6::tmpfiles::specifiers();
    let mut report = r6::tmpfiles::Report::default();
    let rules = if files.is_empty() {
        r6::tmpfiles::load(&dirs, &spec, &mut report)
    } else {
        let mut v = Vec::new();
        for f in &files {
            match std::fs::read_to_string(f) {
                Ok(t) => {
                    for (n, line) in t.lines().enumerate() {
                        match r6::tmpfiles::parse_line(
                            line,
                            &format!("{}:{}", f.display(), n + 1),
                            &spec,
                        ) {
                            Ok(Some(r)) => v.push(r),
                            Ok(None) => {}
                            Err(e) => report.errors.push(e),
                        }
                    }
                }
                Err(e) => report.errors.push(format!("{}: {e}", f.display())),
            }
        }
        v
    };
    r6::tmpfiles::apply(&rules, opts, &mut report);
    for e in &report.errors {
        eprintln!("r6-tmpfiles: {e}");
    }
    eprintln!(
        "r6-tmpfiles: {} rule(s), {} applied, {} skipped, {} error(s)",
        rules.len(),
        report.applied,
        report.skipped,
        report.errors.len()
    );
    std::process::exit(if report.errors.is_empty() { 0 } else { 1 });
}
