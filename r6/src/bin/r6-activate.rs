//! r6-activate: validate a service directory and print the parallel activation plan.
//! Used at system build time so that broken service graphs never reach a boot.
//!
//! Usage: r6-activate [--dir DIR] [--all] [--json]

use r6::activation::Graph;
use r6::config::ServiceSet;
use std::collections::BTreeSet;

fn main() {
    let mut args = std::env::args().skip(1);
    let mut dir = r6::paths::Paths::from_env().service_dir;
    let (mut all, mut json) = (false, false);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--dir" => dir = args.next().map(Into::into).unwrap_or(dir),
            "--all" => all = true,
            "--json" => json = true,
            _ => {
                eprintln!("usage: r6-activate [--dir DIR] [--all] [--json]");
                std::process::exit(2);
            }
        }
    }
    let set = match ServiceSet::load_dir(&dir) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("r6-activate: invalid configuration: {e}");
            std::process::exit(1);
        }
    };
    let g = Graph::new(&set);
    let roots: Vec<usize> = (0..g.names.len())
        .filter(|&i| all || set.services[g.names[i]].enable)
        .collect();
    let members: BTreeSet<usize> = g.closure(roots);
    let waves = g.waves(&members).expect("validated");
    if json {
        println!(
            "{}",
            serde_json::json!({ "services": set.services.len(), "activated": members.len(), "waves": waves })
        );
        return;
    }
    println!(
        "{} service(s) defined, {} activated at boot, {} wave(s)",
        set.services.len(),
        members.len(),
        waves.len()
    );
    for (i, w) in waves.iter().enumerate() {
        println!("  wave {i}: {}", w.join(" "));
    }
}
