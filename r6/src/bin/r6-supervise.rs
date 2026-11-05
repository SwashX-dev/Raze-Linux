//! r6-supervise: supervise all services defined in the service directory.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version") {
        println!("r6-supervise {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let paths = r6::paths::Paths::from_env();
    let mut sup = match r6::supervisor::Supervisor::new(paths) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("r6-supervise: {e}");
            std::process::exit(1);
        }
    };
    sup.reap_stale();
    if !args.iter().any(|a| a == "--no-boot") {
        sup.activate_boot_set();
    }
    std::process::exit(sup.run());
}
