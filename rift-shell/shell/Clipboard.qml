import QtQuick
import QtQuick.Controls
import Quickshell
import Quickshell.Io

// Clipboard history (cliphist; `wl-paste --watch cliphist store` runs as a user service).
OverlayPanel {
    id: root
    open: Shell.clipboardOpen
    ns: "rift-shell-clipboard"
    onCloseRequested: Shell.clipboardOpen = false
    onOpenChanged: if (open) { filter.text = ""; reader.running = true; filter.forceActiveFocus(); }

    property var entries: []
    readonly property var shown: entries.filter(e => e.text.toLowerCase().includes(filter.text.toLowerCase())).slice(0, 200)

    Process {
        id: reader
        command: ["cliphist", "list"]
        stdout: StdioCollector {
            onStreamFinished: {
                root.entries = text.split("\n").filter(l => l !== "").map(l => {
                    const tab = l.indexOf("\t");
                    return { id: l.substring(0, tab), text: l.substring(tab + 1), line: l };
                });
                list.currentIndex = 0;
            }
        }
    }

    function pick(e) {
        if (!e) return;
        Quickshell.execDetached(["sh", "-c", 'printf "%s" "$1" | cliphist decode | wl-copy', "sh", e.line]);
        Shell.clipboardOpen = false;
    }

    function wipe() {
        Quickshell.execDetached(["cliphist", "wipe"]);
        entries = [];
    }

    Rectangle {
        anchors.horizontalCenter: parent.horizontalCenter
        y: parent.height * 0.15
        width: Math.min(640, parent.width - 40)
        height: Math.min(parent.height * 0.7, 120 + root.shown.length * 44)
        radius: Theme.radius + 4
        color: Theme.surface
        border.color: Theme.border
        MouseArea { anchors.fill: parent }

        Row {
            id: top
            anchors { top: parent.top; left: parent.left; right: parent.right; margins: 12 }
            spacing: 8
            TextField {
                id: filter
                width: parent.width - clear.width - 8
                height: 44
                placeholderText: "Search clipboard history"
                color: Theme.text
                placeholderTextColor: Theme.textDim
                font.pixelSize: 15
                background: Rectangle { radius: Theme.radius; color: Theme.bg; border.color: filter.activeFocus ? Theme.accent : Theme.border }
                Keys.onDownPressed: list.incrementCurrentIndex()
                Keys.onUpPressed: list.decrementCurrentIndex()
                Keys.onReturnPressed: root.pick(root.shown[list.currentIndex])
                Keys.onEscapePressed: Shell.clipboardOpen = false
                Keys.onDeletePressed: {
                    const e = root.shown[list.currentIndex];
                    if (e) {
                        Quickshell.execDetached(["sh", "-c", 'printf "%s" "$1" | cliphist delete', "sh", e.line]);
                        root.entries = root.entries.filter(x => x.id !== e.id);
                    }
                }
            }
            BarButton { id: clear; icon: "delete_sweep"; anchors.verticalCenter: parent.verticalCenter; onClicked: root.wipe() }
        }

        Txt {
            visible: root.shown.length === 0
            anchors.centerIn: parent
            text: "Clipboard history is empty"
            color: Theme.textDim
        }

        ListView {
            id: list
            anchors { top: top.bottom; left: parent.left; right: parent.right; bottom: parent.bottom; margins: 8 }
            clip: true
            model: root.shown
            delegate: Rectangle {
                required property var modelData
                required property int index
                width: list.width
                height: 44
                radius: Theme.radius
                color: ListView.isCurrentItem ? Qt.alpha(Theme.accent, 0.22) : (area.containsMouse ? Theme.surfaceHigh : "transparent")
                Txt {
                    anchors.fill: parent
                    anchors.leftMargin: 12
                    anchors.rightMargin: 12
                    text: modelData.text.replace(/\s+/g, " ")
                    font.family: modelData.text.startsWith("[[ binary") ? Theme.font : Theme.mono
                    color: modelData.text.startsWith("[[ binary") ? Theme.textDim : Theme.text
                }
                MouseArea { id: area; anchors.fill: parent; hoverEnabled: true; onClicked: root.pick(modelData) }
            }
        }
    }
}
