//! r6-user: the per-user supervisor of a login session.
//!
//! Runs the same supervisor as the system (`r6-supervise`) with per-user locations:
//! services from `/etc/r6/user-services` overlaid by `~/.config/r6/services`, state in
//! `$XDG_RUNTIME_DIR/r6` and `~/.local/state/r6`. Graphical sessions start it before
//! the compositor, import WAYLAND_DISPLAY with `r6 --user import-env`, and stop it with
//! `r6 --user shutdown` on logout. Control it with `r6 --user …`.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version") {
        println!("r6-user {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let paths = match r6::paths::user_paths() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("r6-user: {e}");
            std::process::exit(1);
        }
    };
    let mut sup = match r6::supervisor::Supervisor::new(paths) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("r6-user: {e}");
            std::process::exit(1);
        }
    };
    sup.reap_stale();
    if !args.iter().any(|a| a == "--no-boot") {
        sup.activate_boot_set();
    }
    std::process::exit(sup.run());
}
