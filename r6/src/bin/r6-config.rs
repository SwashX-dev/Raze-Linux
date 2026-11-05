//! r6-config: inspect and validate r6 service configuration offline.
//!
//!   r6-config check [--user]          validate all service directories
//!   r6-config show SERVICE [--user]   print the effective definition of a service
//!   r6-config graph [--user]          dependency graph in Graphviz dot format
//!   r6-config dirs [--user]           list the service directories in overlay order

use r6::activation::Graph;
use r6::config::ServiceSet;

fn die(msg: &str) -> ! {
    eprintln!("r6-config: {msg}");
    std::process::exit(1);
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let user = args.iter().any(|a| a == "--user");
    args.retain(|a| a != "--user");
    let paths = if user {
        r6::paths::user_paths().unwrap_or_else(|e| die(&e))
    } else {
        r6::paths::Paths::from_env()
    };
    let cmd = args.first().map(String::as_str).unwrap_or("check");
    if cmd == "dirs" {
        for d in &paths.service_dirs {
            println!(
                "{}{}",
                d.display(),
                if d.is_dir() { "" } else { " (missing)" }
            );
        }
        return;
    }
    let set = ServiceSet::load_dirs(&paths.service_dirs)
        .unwrap_or_else(|e| die(&format!("invalid configuration: {e}")));
    match cmd {
        "check" => println!("ok: {} service(s)", set.services.len()),
        "show" => {
            let name = args
                .get(1)
                .unwrap_or_else(|| die("show needs a service name"));
            let s = set
                .services
                .get(name)
                .unwrap_or_else(|| die(&format!("unknown service '{name}'")));
            println!("name            = {}", s.name);
            println!("description     = {:?}", s.description);
            println!("command         = {:?}", s.command);
            println!("type            = {:?}", s.kind);
            println!("depends         = {:?}", s.depends);
            println!("wants           = {:?}", s.wants);
            println!(
                "ready           = {:?} (timeout {} ms)",
                s.ready, s.ready_timeout_ms
            );
            println!(
                "restart         = {:?} (delay {} ms, max {} in {} s)",
                s.restart, s.restart_delay_ms, s.restart_max, s.restart_window_s
            );
            println!(
                "stop            = signal {} (timeout {} ms)",
                s.stop_signal, s.stop_timeout_ms
            );
            println!("user/group      = {:?} / {:?}", s.user, s.group);
            println!(
                "umask           = {:?}",
                s.umask.map(|m| format!("{m:03o}"))
            );
            println!("rlimit_nofile   = {:?}", s.rlimit_nofile);
            println!("oom_score_adjust= {:?}", s.oom_score_adjust);
            println!("log             = {:?}", s.log);
            println!("enable          = {}", s.enable);
            for (k, v) in &s.environment {
                println!("environment.{k} = {v:?}");
            }
        }
        "graph" => {
            let g = Graph::new(&set);
            println!("digraph r6 {{\n  rankdir=LR;");
            for (i, n) in g.names.iter().enumerate() {
                let style = if set.services[*n].enable {
                    "bold"
                } else {
                    "dashed"
                };
                println!("  \"{n}\" [style={style}];");
                for &d in &g.deps[i] {
                    let soft = !set.services[*n].depends.iter().any(|x| x == g.names[d]);
                    println!(
                        "  \"{}\" -> \"{n}\"{};",
                        g.names[d],
                        if soft { " [style=dotted]" } else { "" }
                    );
                }
            }
            println!("}}");
        }
        other => die(&format!("unknown command '{other}'")),
    }
}
