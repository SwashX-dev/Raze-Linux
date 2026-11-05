//@ pragma UseQApplication
//@ pragma Env QT_QUICK_CONTROLS_STYLE=Basic
import QtQuick
import Quickshell

// Rift Shell: panel, launcher, control centre, notifications, lock screen, power menu,
// clipboard history, OSD, polkit agent and idle management for the Rift compositor.
ShellRoot {
    Variants {
        model: Quickshell.screens
        delegate: Component {
            Scope {
                id: perScreen
                required property var modelData
                Wallpaper { modelData: perScreen.modelData }
                Bar { modelData: perScreen.modelData }
            }
        }
    }

    Launcher {}
    ControlCenter {}
    PowerMenu {}
    Clipboard {}
    NotificationPopups {}
    Osd { id: osd }
    PolkitDialog {}
    Lock {}
    Idle {}
    NightLight {}
    Ipc { osd: osd }
}
