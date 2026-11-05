//! `rift-shell`: starts Rift Shell (a Quickshell configuration) and implements the
//! commands bound to keys in Rift: overlays (via Quickshell IPC), screenshots, screen
//! recording, volume/brightness with OSD and settings import/export.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

const USAGE: &str = "usage: rift-shell [command]
  (no command)              start the shell
  launcher | control-center | power | clipboard | lock
  dnd                       toggle Do Not Disturb
  theme                     toggle dark/light
  wallpaper <file>          set the wallpaper
  screenshot [full|region|window]
  record [toggle|start|stop] [region]
  volume up|down|mute       change the default sink volume
  brightness up|down        change the backlight
  config export <file> | config import <file>
  ipc <target> <function> [args...]";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match a.as_slice() {
        [] | ["run"] => run_shell(),
        ["launcher"] => ipc(&["launcher", "toggle"]),
        ["control-center"] => ipc(&["controlCenter", "toggle"]),
        ["power"] => ipc(&["power", "toggle"]),
        ["clipboard"] => ipc(&["clipboard", "toggle"]),
        ["lock"] => ipc(&["lock", "lock"]),
        ["dnd"] => ipc(&["notifications", "toggleDnd"]),
        ["theme"] => ipc(&["theme", "toggle"]),
        ["wallpaper", file] => absolute(file).and_then(|p| ipc(&["theme", "setWallpaper", &p])),
        ["screenshot"] => screenshot("full"),
        ["screenshot", mode] => screenshot(mode),
        ["record"] | ["record", "toggle"] => record_toggle(false),
        ["record", "toggle", "region"] => record_toggle(true),
        ["record", "start"] => record_start(false),
        ["record", "start", "region"] => record_start(true),
        ["record", "stop"] => record_stop(),
        ["volume", dir] => volume(dir),
        ["brightness", dir] => brightness(dir),
        ["config", "export", file] => config_export(file),
        ["config", "import", file] => config_import(file),
        ["ipc", rest @ ..] if rest.len() >= 2 => ipc(rest),
        ["-h"] | ["--help"] | ["help"] => {
            println!("{USAGE}");
            Ok(())
        }
        ["--version"] => {
            println!("rift-shell {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        _ => Err(format!("unknown command\n{USAGE}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("rift-shell: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Directory holding shell.qml: $RIFT_SHELL_DIR, else <prefix>/share/rift-shell/shell.
fn shell_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("RIFT_SHELL_DIR") {
        return PathBuf::from(d);
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.canonicalize().ok())
        .and_then(|exe| {
            exe.parent()
                .and_then(Path::parent)
                .map(|p| p.join("share/rift-shell/shell"))
        })
        .unwrap_or_else(|| PathBuf::from("/run/current-system/sw/share/rift-shell/shell"))
}

fn run_shell() -> Result<(), String> {
    use std::os::unix::process::CommandExt;
    let mut cmd = Command::new("quickshell");
    cmd.arg("-p").arg(shell_dir()).arg("--no-duplicate");
    // When Rift composites in software (no 3D-capable GPU), Qt's own software renderer
    // is far cheaper than OpenGL on llvmpipe.
    if std::env::var_os("QT_QUICK_BACKEND").is_none() {
        let software = rift(&["version"])
            .ok()
            .and_then(|v| {
                v["backend"]
                    .as_str()
                    .map(|b| b == "drm-pixman" || b == "headless")
            })
            .unwrap_or(false);
        if software {
            cmd.env("QT_QUICK_BACKEND", "software");
        }
    }
    let err = cmd.exec();
    Err(format!("cannot start quickshell: {err}"))
}

fn ipc(call: &[&str]) -> Result<(), String> {
    let status = Command::new("quickshell")
        .arg("ipc")
        .arg("-p")
        .arg(shell_dir())
        .arg("call")
        .args(call)
        .status()
        .map_err(|e| format!("cannot run quickshell: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("the shell is not running (start it with `rift-shell`)".into())
    }
}

fn output(cmd: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(cmd)
        .args(args)
        .stderr(Stdio::inherit())
        .output()
        .map_err(|e| format!("cannot run {cmd}: {e}"))?;
    if !out.status.success() {
        return Err(format!("{cmd} failed"));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn run(cmd: &str, args: &[&str]) -> Result<(), String> {
    let status = Command::new(cmd)
        .args(args)
        .status()
        .map_err(|e| format!("cannot run {cmd}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{cmd} failed"))
    }
}

fn notify(summary: &str, body: &str, icon: Option<&Path>) {
    let mut cmd = Command::new("notify-send");
    cmd.args(["-a", "Rift Shell", summary, body]);
    if let Some(i) = icon {
        cmd.arg("-i").arg(i);
    }
    let _ = cmd.stdout(Stdio::null()).stderr(Stdio::null()).status();
}

fn absolute(p: &str) -> Result<String, String> {
    std::fs::canonicalize(p)
        .map(|p| p.display().to_string())
        .map_err(|e| format!("{p}: {e}"))
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}

/// An XDG user directory (from user-dirs.dirs), e.g. "PICTURES".
fn xdg_user_dir(name: &str, fallback: &str) -> PathBuf {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".config"));
    if let Ok(text) = std::fs::read_to_string(config.join("user-dirs.dirs")) {
        let key = format!("XDG_{name}_DIR=");
        if let Some(line) = text.lines().find(|l| l.starts_with(&key)) {
            let v = line[key.len()..]
                .trim_matches('"')
                .replace("$HOME", &home().display().to_string());
            return PathBuf::from(v);
        }
    }
    home().join(fallback)
}

fn timestamp() -> String {
    output("date", &["+%Y-%m-%d_%H-%M-%S"]).unwrap_or_else(|_| "now".into())
}

/// Ask Rift for JSON (`rift msg <request>`).
fn rift(request: &[&str]) -> Result<serde_json::Value, String> {
    let text = output("rift", &[&["msg"], request].concat())?;
    serde_json::from_str(&text).map_err(|e| format!("bad reply from rift: {e}"))
}

fn focused_output() -> Option<String> {
    let state = rift(&["state"]).ok()?;
    state["workspaces"]
        .as_array()?
        .iter()
        .find(|w| w["focused"].as_bool() == Some(true))
        .and_then(|w| w["monitor"].as_str().map(String::from))
}

fn screenshot(mode: &str) -> Result<(), String> {
    let dir = xdg_user_dir("PICTURES", "Pictures").join("Screenshots");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let file = dir.join(format!("Screenshot_{}.png", timestamp()));
    let file_s = file.display().to_string();
    match mode {
        "region" | "area" => {
            let geometry = match output("slurp", &["-d"]) {
                Ok(g) if !g.is_empty() => g,
                _ => return Ok(()), // cancelled
            };
            run("grim", &["-g", &geometry, &file_s])?;
        }
        "window" => {
            let w = rift(&["focused-window"])?;
            let g = |k: &str| w[k].as_i64().ok_or_else(|| "no focused window".to_string());
            let geometry = format!("{},{} {}x{}", g("x")?, g("y")?, g("width")?, g("height")?);
            run("grim", &["-g", &geometry, &file_s])?;
        }
        "full" | "output" | "screen" => match focused_output() {
            Some(o) => run("grim", &["-o", &o, &file_s])?,
            None => run("grim", &[&file_s])?,
        },
        other => return Err(format!("unknown screenshot mode {other:?}")),
    }
    // Copy to the clipboard as an image.
    if let Ok(f) = std::fs::File::open(&file) {
        let _ = Command::new("wl-copy")
            .args(["--type", "image/png"])
            .stdin(f)
            .status();
    }
    notify("Screenshot saved", &file_s, Some(&file));
    println!("{file_s}");
    Ok(())
}

fn record_pidfile() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("rift-shell-record")
}

fn recording_pid() -> Option<i32> {
    let text = std::fs::read_to_string(record_pidfile()).ok()?;
    let pid: i32 = text.lines().next()?.trim().parse().ok()?;
    Path::new(&format!("/proc/{pid}")).exists().then_some(pid)
}

fn record_toggle(region: bool) -> Result<(), String> {
    if recording_pid().is_some() {
        record_stop()
    } else {
        record_start(region)
    }
}

fn record_start(region: bool) -> Result<(), String> {
    if recording_pid().is_some() {
        return Err("already recording".into());
    }
    let dir = xdg_user_dir("VIDEOS", "Videos").join("Recordings");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let file = dir.join(format!("Recording_{}.mp4", timestamp()));
    // wf-recorder captures through wlr-screencopy (frames only when the screen changes)
    // and encodes with libx264 on the CPU.
    let mut cmd = Command::new("wf-recorder");
    if region {
        let geometry = match output("slurp", &["-d"]) {
            Ok(g) if !g.is_empty() => g,
            _ => return Ok(()),
        };
        cmd.args(["-g", &geometry]);
    } else if let Some(o) = focused_output() {
        cmd.args(["-o", &o]);
    }
    cmd.arg("-f")
        .arg(&file)
        .stdin(Stdio::null())
        .stdout(Stdio::null());
    let log = std::fs::File::create(record_pidfile().with_extension("log")).ok();
    if let Some(l) = log {
        cmd.stderr(l);
    }
    let child = cmd
        .spawn()
        .map_err(|e| format!("cannot start wf-recorder: {e}"))?;
    std::fs::write(
        record_pidfile(),
        format!("{}\n{}\n", child.id(), file.display()),
    )
    .map_err(|e| format!("cannot write the recording state: {e}"))?;
    let _ = ipc(&["recording", "setActive", "true"]);
    notify(
        "Recording started",
        "Stop it from the bar or with rift-shell record stop",
        None,
    );
    Ok(())
}

fn record_stop() -> Result<(), String> {
    let Some(pid) = recording_pid() else {
        let _ = ipc(&["recording", "setActive", "false"]);
        return Err("not recording".into());
    };
    let file = std::fs::read_to_string(record_pidfile())
        .ok()
        .and_then(|t| t.lines().nth(1).map(String::from))
        .unwrap_or_default();
    // SAFETY: plain kill(2) on a pid we started.
    unsafe {
        libc_kill(pid, 2);
    }
    for _ in 0..100 {
        if !Path::new(&format!("/proc/{pid}")).exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let _ = std::fs::remove_file(record_pidfile());
    let _ = ipc(&["recording", "setActive", "false"]);
    notify("Recording saved", &file, None);
    println!("{file}");
    Ok(())
}

extern "C" {
    #[link_name = "kill"]
    fn libc_kill(pid: i32, sig: i32) -> i32;
}

fn volume(dir: &str) -> Result<(), String> {
    match dir {
        "up" => run(
            "wpctl",
            &["set-volume", "-l", "1.0", "@DEFAULT_AUDIO_SINK@", "5%+"],
        ),
        "down" => run("wpctl", &["set-volume", "@DEFAULT_AUDIO_SINK@", "5%-"]),
        "mute" => run("wpctl", &["set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"]),
        _ => Err("volume up|down|mute".into()),
    }
}

fn brightness(dir: &str) -> Result<(), String> {
    let step = match dir {
        "up" => "5%+",
        "down" => "5%-",
        _ => return Err("brightness up|down".into()),
    };
    run("brightnessctl", &["-q", "-c", "backlight", "set", step])?;
    let info = output("brightnessctl", &["-m", "-c", "backlight"])?;
    let pct = info
        .split(',')
        .nth(3)
        .unwrap_or("0%")
        .trim_end_matches('%')
        .to_string();
    let _ = ipc(&["osd", "brightness", &pct]);
    Ok(())
}

fn settings_path() -> PathBuf {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".config"));
    config.join("rift-shell/settings.json")
}

fn config_export(file: &str) -> Result<(), String> {
    std::fs::copy(settings_path(), file).map_err(|e| format!("cannot export: {e}"))?;
    println!("exported {} to {file}", settings_path().display());
    Ok(())
}

fn config_import(file: &str) -> Result<(), String> {
    let text = std::fs::read_to_string(file).map_err(|e| format!("{file}: {e}"))?;
    let v: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("{file}: not valid JSON: {e}"))?;
    if !v.is_object() {
        return Err(format!("{file}: expected a JSON object"));
    }
    let dest = settings_path();
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&dest, text).map_err(|e| format!("{}: {e}", dest.display()))?;
    println!("imported {file}; the shell reloads it automatically");
    Ok(())
}
