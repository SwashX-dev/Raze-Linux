pragma Singleton
import QtQuick
import Quickshell
import Quickshell.Io

// Persistent user settings: ~/.config/rift-shell/settings.json (written on change).
Singleton {
    id: root
    readonly property string dir: (Quickshell.env("XDG_CONFIG_HOME") || (Quickshell.env("HOME") + "/.config")) + "/rift-shell"
    readonly property string path: dir + "/settings.json"
    property alias data: adapter

    FileView {
        id: file
        path: root.path
        watchChanges: true
        blockLoading: true
        onFileChanged: reload()
        onAdapterUpdated: writeAdapter()
        onLoadFailed: error => {
            if (error === FileViewError.FileNotFound) {
                Quickshell.execDetached(["mkdir", "-p", root.dir]);
                writeTimer.start();
            }
        }

        JsonAdapter {
            id: adapter
            property string theme: "dark"
            property string accent: "#dc2626"
            property string wallpaper: ""
            property bool dnd: false
            property bool use24h: true
            property bool nightLight: false
            property int nightLightTemp: 4000
            property int idleLockMinutes: 10
            property int idleScreenOffMinutes: 15
            property bool showSeconds: false
            property list<string> pinned: ["foot.desktop", "firefox.desktop", "pcmanfm.desktop"]
            property list<string> disabledPlugins: []
        }
    }

    Timer {
        id: writeTimer
        interval: 200
        onTriggered: file.writeAdapter()
    }
}
