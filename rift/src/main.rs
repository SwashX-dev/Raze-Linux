//! Rift: a scrollable-tiling Wayland compositor for Raze Linux.

mod backend;
mod config;
mod handlers;
mod input;
mod ipc;
mod layout;
mod render;
mod screencopy;
mod state;

use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::Display;

use crate::backend::Backend;
use crate::config::Config;
use crate::state::Rift;

const USAGE: &str = "usage:
  rift [--backend udev|winit|headless] [--size WxH] [--outputs N]
  rift msg [request...]        talk to the running compositor (see `rift msg help`)
  rift validate [config.toml]  check a configuration file
  rift --version";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("msg") => std::process::exit(ipc::client(&args[1..])),
        Some("validate") => std::process::exit(validate(args.get(1))),
        Some("--version") | Some("-V") => {
            println!("rift {}", env!("CARGO_PKG_VERSION"));
            return;
        }
        Some("--help") | Some("-h") => {
            println!("{USAGE}");
            return;
        }
        _ => {}
    }

    let mut backend_kind: Option<String> = None;
    let mut size = (1280, 800);
    let mut outputs = 1usize;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--backend" => backend_kind = it.next().cloned(),
            "--size" => {
                size = it
                    .next()
                    .and_then(|s| s.split_once('x'))
                    .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
                    .unwrap_or_else(|| die("--size expects WxH"));
            }
            "--outputs" => {
                outputs = it
                    .next()
                    .and_then(|n| n.parse().ok())
                    .unwrap_or_else(|| die("--outputs expects a number"))
            }
            other => die(&format!("unknown argument {other:?}\n{USAGE}")),
        }
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("RIFT_LOG")
                .unwrap_or_else(|_| "rift=info,smithay=warn".into()),
        )
        .with_writer(std::io::stderr)
        .init();

    let backend_kind = backend_kind.unwrap_or_else(|| {
        if std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("DISPLAY").is_some() {
            "winit".into()
        } else {
            "udev".into()
        }
    });

    let (config, config_error) = Config::load();
    if let Some(e) = &config_error {
        tracing::warn!("configuration error, using defaults: {e}");
    }

    let mut event_loop: EventLoop<'static, Rift> =
        EventLoop::try_new().unwrap_or_else(|e| die(&format!("event loop: {e}")));
    let display: Display<Rift> =
        Display::new().unwrap_or_else(|e| die(&format!("Wayland display: {e}")));

    let backend = match backend_kind.as_str() {
        "headless" => Backend::Headless(Box::new(
            backend::headless::Headless::new(size).unwrap_or_else(|e| die(&e)),
        )),
        "winit" => Backend::Winit(Box::new(
            backend::winit::Winit::new(&event_loop).unwrap_or_else(|e| die(&e)),
        )),
        "udev" | "drm" | "tty" => Backend::Udev(Box::new(
            backend::udev::Udev::new(&event_loop).unwrap_or_else(|e| die(&e)),
        )),
        other => die(&format!("unknown backend {other:?}")),
    };

    let mut state = Rift::new(
        display,
        event_loop.handle(),
        event_loop.get_signal(),
        backend,
        config,
        config_error,
    );
    match &state.backend {
        Backend::Headless(_) => backend::headless::init(&mut state, outputs),
        Backend::Winit(_) => backend::winit::init(&mut state),
        Backend::Udev(_) => {
            if let Err(e) = backend::udev::init(&mut state) {
                die(&e);
            }
        }
        Backend::Empty => {}
    }
    if state.outputs.is_empty() && !matches!(state.backend, Backend::Udev(_)) {
        die("no outputs");
    }

    tracing::info!(
        "Rift {} running on WAYLAND_DISPLAY={} (backend {}), IPC {}",
        env!("CARGO_PKG_VERSION"),
        state.socket_name.to_string_lossy(),
        state.backend.name(),
        state.ipc.path.display()
    );
    // Children inherit these through `child_env`; also export them for our own helpers.
    std::env::set_var("WAYLAND_DISPLAY", &state.socket_name);
    std::env::set_var("RIFT_SOCKET", &state.ipc.path);

    state.start_xwayland();
    for cmd in state.config.spawn_at_startup.clone() {
        state.spawn(&cmd);
    }

    let result = event_loop.run(None, &mut state, |state| {
        state.refresh();
        backend::render_pending(state);
        let _ = state.display_handle.flush_clients();
    });
    if let Err(e) = result {
        die(&format!("event loop: {e}"));
    }
    for child in state.children.iter_mut() {
        let _ = child.kill();
    }
}

fn validate(path: Option<&String>) -> i32 {
    let path = path.map(std::path::PathBuf::from).or_else(Config::path);
    let Some(path) = path else {
        eprintln!("no configuration file found");
        return 1;
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{}: {e}", path.display());
            return 1;
        }
    };
    match Config::parse(&text) {
        Ok(_) => {
            println!("{}: ok", path.display());
            0
        }
        Err(e) => {
            eprintln!("{}: {e}", path.display());
            1
        }
    }
}

fn die(msg: &str) -> ! {
    eprintln!("rift: {msg}");
    std::process::exit(1)
}
