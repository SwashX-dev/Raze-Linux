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
    // Services get a clean environment; pass on the login session's identity and
    // locations (an explicit list, not everything the session happens to have).
    const SESSION_VARS: &[&str] = &[
        "HOME",
        "USER",
        "LOGNAME",
        "SHELL",
        "LANG",
        "LANGUAGE",
        "XDG_RUNTIME_DIR",
        "XDG_SESSION_ID",
        "XDG_SESSION_TYPE",
        "XDG_SESSION_CLASS",
        "XDG_SEAT",
        "XDG_VTNR",
        "XDG_DATA_DIRS",
        "XDG_CONFIG_DIRS",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_STATE_HOME",
        "XDG_CACHE_HOME",
        "DBUS_SESSION_BUS_ADDRESS",
        "LOCALE_ARCHIVE",
        "TZDIR",
    ];
    sup.set_env(
        std::env::vars()
            .filter(|(k, _)| SESSION_VARS.contains(&k.as_str()) || k.starts_with("LC_")),
    );
    sup.reap_stale();
    if !args.iter().any(|a| a == "--no-boot") {
        sup.activate_boot_set();
    }
    std::process::exit(sup.run());
}
