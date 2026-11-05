pragma Singleton
import QtQuick
import Quickshell
import Quickshell.Io

// Live compositor state from `rift msg subscribe` (one JSON object per change).
Singleton {
    id: root
    property var workspaces: []
    property var windows: []
    property var focused: null
    property var outputs: []
    property bool connected: false
    readonly property bool available: Quickshell.env("RIFT_SOCKET") !== ""
    readonly property string focusedOutput: {
        const ws = workspaces.find(w => w.focused);
        return ws ? ws.monitor : (outputs.length ? outputs[0].name : "");
    }

    function workspacesOn(output) {
        return workspaces.filter(w => w.monitor === output);
    }

    function action(text) {
        Quickshell.execDetached(["rift", "msg", "action"].concat(text.split(" ")));
    }

    Process {
        id: proc
        running: root.available
        command: ["rift", "msg", "subscribe"]
        stdout: SplitParser {
            onRead: line => {
                try {
                    const s = JSON.parse(line);
                    root.workspaces = s.workspaces || [];
                    root.windows = s.windows || [];
                    root.focused = s.focused;
                    root.outputs = s.outputs || [];
                    root.connected = true;
                } catch (e) {
                    console.warn("rift-shell: bad state from rift:", e);
                }
            }
        }
        onExited: {
            root.connected = false;
            restart.start();
        }
    }

    Timer {
        id: restart
        interval: 1000
        onTriggered: if (root.available) proc.running = true
    }
}
