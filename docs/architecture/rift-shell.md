# Rift Shell

The desktop shell of the default Raze desktop: panel, launcher, control centre,
notifications, lock screen, power menu, clipboard history, on-screen display,
polkit agent, idle management and night light. Inspired by Noctalia, written for
Rift.

It is a [Quickshell](https://quickshell.org) configuration (QML, `rift-shell/shell`)
plus a small Rust program, `rift-shell` (`rift-shell/src`), which starts it and
implements the commands bound to keys in Rift.

## Components

| Component | File | Notes |
|---|---|---|
| Panel (one per output) | `Bar.qml` | workspaces of that output (from Rift IPC), focused window, clock, media, tray, plugins, network, Bluetooth, volume (scroll to change), battery, recording indicator, notifications, power |
| Launcher | `Launcher.qml` | desktop entries with fuzzy matching; `=` calculator (Enter copies), `>` runs a command |
| Control centre | `ControlCenter.qml` | Wi-Fi (networks, connect with password), Bluetooth (devices, pair), Do Not Disturb, night light, power profile, stay awake, screen recording, clipboard; volume, microphone and backlight sliders; media player; notification history |
| Notifications | `Notifs.qml`, `NotificationPopups.qml` | the `org.freedesktop.Notifications` server; actions, images, urgency; history (100 entries, kept across sessions) and Do Not Disturb (critical notifications still show) |
| Lock screen | `Lock.qml` | ext-session-lock (Rift keeps the session locked even if the shell crashes), PAM service `rift-shell` |
| Power menu | `PowerMenu.qml` | lock, suspend, log out, restart, shut down (elogind) |
| Clipboard history | `Clipboard.qml` | cliphist (`wl-paste --watch cliphist store` runs as a user service); search, Enter pastes, Delete removes |
| OSD | `Osd.qml` | volume (any change, from any program) and backlight |
| Polkit agent | `PolkitDialog.qml` | authentication dialogs for privileged actions |
| Idle | `Idle.qml` | lock after `idleLockMinutes`, screens off after `idleScreenOffMinutes`; video players' idle inhibitors and "Stay awake" pause it |
| Night light | `NightLight.qml` | `rift msg night-light <K>` (DRM gamma ramps) |
| Wallpaper | `Wallpaper.qml` | background layer, per output |

Multi-monitor: the panel and wallpaper exist once per output; overlays open on the
output that has keyboard focus in Rift.

Everything is event driven (Quickshell bindings on D-Bus/PipeWire/Rift IPC events);
there are no polling loops. When Rift composites in software, `rift-shell` starts
Quickshell with Qt's software renderer, which uses less memory and CPU than OpenGL
on llvmpipe.

## Commands

```bash
rift-shell                       # start the shell (r6 user service `rift-shell`)
rift-shell launcher | control-center | power | clipboard | lock | dnd | theme
rift-shell screenshot [full|region|window]   # ~/Pictures/Screenshots, also copied to the clipboard
rift-shell record [toggle|start|stop] [region] # ~/Videos/Recordings (wf-recorder)
rift-shell volume up|down|mute
rift-shell brightness up|down
rift-shell wallpaper FILE
rift-shell config export FILE | config import FILE
rift-shell ipc TARGET FUNCTION [ARGS]   # any IpcHandler in Ipc.qml
```

## Settings and themes

`~/.config/rift-shell/settings.json`, written by the shell when you change something
and re-read when the file changes:

```json
{
  "theme": "dark",            // or "light"
  "accent": "#dc2626",
  "wallpaper": "",            // empty: the Raze wallpaper
  "dnd": false,
  "use24h": true,
  "showSeconds": false,
  "nightLight": false,
  "nightLightTemp": 4000,
  "idleLockMinutes": 10,      // 0 disables
  "idleScreenOffMinutes": 15, // 0 disables
  "pinned": ["foot.desktop"],
  "disabledPlugins": []
}
```

`rift-shell config export/import` copies this file, e.g. to another machine.

## Plugins

Every `*.qml` file in `~/.config/rift-shell/plugins/` is loaded into the panel (right
side). A plugin is a QML item; if it declares `property var shell`, it receives
`{ theme, rift, settings, notifs, run }`:

```qml
// ~/.config/rift-shell/plugins/Uptime.qml
import QtQuick
import Quickshell.Io

Text {
    property var shell
    color: shell ? shell.theme.text : "white"
    font.pixelSize: 13
    Process {
        running: true
        command: ["cat", "/proc/uptime"]
        stdout: StdioCollector { onStreamFinished: parent.text = "up " + Math.round(text.split(" ")[0] / 3600) + "h" }
    }
}
```

Add a file name to `disabledPlugins` to disable a plugin without deleting it.

## Resource use

Measured numbers are in [performance/benchmarks.md](../performance/benchmarks.md).
