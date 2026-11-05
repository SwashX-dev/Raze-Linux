import QtQuick
import Quickshell
import Quickshell.Io

// Commands from `rift-shell <command>` (via `quickshell ipc call`).
Scope {
    IpcHandler {
        target: "launcher"
        function toggle(): void { Shell.toggle("launcherOpen"); }
        function open(): void { Shell.closeAll(); Shell.launcherOpen = true; }
        function close(): void { Shell.launcherOpen = false; }
    }
    IpcHandler {
        target: "controlCenter"
        function toggle(): void { Shell.toggle("controlCenterOpen"); }
    }
    IpcHandler {
        target: "power"
        function toggle(): void { Shell.toggle("powerMenuOpen"); }
    }
    IpcHandler {
        target: "clipboard"
        function toggle(): void { Shell.toggle("clipboardOpen"); }
    }
    IpcHandler {
        target: "lock"
        function lock(): void { Shell.lock(); }
        function isLocked(): bool { return Shell.locked; }
    }
    IpcHandler {
        target: "notifications"
        function toggleDnd(): void { Notifs.setDnd(!Notifs.dnd); }
        function clear(): void { Notifs.clearHistory(); }
        function count(): int { return Notifs.history.length; }
    }
    IpcHandler {
        target: "theme"
        function toggle(): void { Settings.data.theme = Theme.dark ? "light" : "dark"; }
        function setAccent(color: string): void { Settings.data.accent = color; }
        function setWallpaper(path: string): void { Settings.data.wallpaper = path; }
    }
    IpcHandler {
        target: "osd"
        function brightness(percent: int): void { osd.show("brightness_medium", percent / 100); }
        function volume(percent: int): void { osd.show("volume_up", percent / 100); }
    }
    IpcHandler {
        target: "recording"
        function setActive(active: bool): void { Shell.recording = active; }
    }

    property var osd
}
