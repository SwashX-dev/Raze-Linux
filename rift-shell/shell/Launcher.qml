import QtQuick
import QtQuick.Controls
import Quickshell
import Quickshell.Widgets

// Application launcher. Prefixes: "=" calculator, ">" run a shell command.
OverlayPanel {
    id: root
    open: Shell.launcherOpen
    ns: "rift-shell-launcher"
    onCloseRequested: Shell.launcherOpen = false
    // Created on demand (shell.qml), so also initialise when created already open.
    function shown() { search.text = ""; list.currentIndex = 0; search.forceActiveFocus(); }
    onOpenChanged: if (open) shown()
    Component.onCompleted: if (open) shown()

    readonly property string query: search.text.trim()
    readonly property bool calc: query.startsWith("=")
    readonly property bool command: query.startsWith(">")
    readonly property string calcResult: {
        if (!calc) return "";
        const expr = query.substring(1);
        if (!/^[0-9+\-*/().,%^ e]*$/.test(expr) || expr.trim() === "") return "";
        try {
            const v = Function('"use strict"; return (' + expr.replace(/\^/g, "**").replace(/,/g, ".") + ")")();
            return (typeof v === "number" && isFinite(v)) ? String(Math.round(v * 1e10) / 1e10) : "";
        } catch (e) {
            return "";
        }
    }

    function score(entry, q) {
        const name = entry.name.toLowerCase();
        if (q === "") return Settings.data.pinned.includes(entry.id + ".desktop") ? 2 : 1;
        if (name.startsWith(q)) return 100 - name.length / 100;
        if (name.includes(q)) return 60;
        const hay = (entry.genericName + " " + entry.keywords + " " + entry.comment + " " + entry.id).toLowerCase();
        if (hay.includes(q)) return 30;
        // Subsequence match: "ff" finds "firefox".
        let i = 0;
        for (const c of name) if (c === q[i]) i++;
        return i === q.length ? 10 : 0;
    }

    readonly property var results: {
        if (calc || command) return [];
        const q = query.toLowerCase();
        return DesktopEntries.applications.values
            .filter(e => !e.noDisplay)
            .map(e => ({ e: e, s: score(e, q) }))
            .filter(x => x.s > 0)
            .sort((a, b) => b.s - a.s || a.e.name.localeCompare(b.e.name))
            .slice(0, 50)
            .map(x => x.e);
    }

    function activate(i) {
        if (calc) {
            if (calcResult !== "") Quickshell.execDetached(["wl-copy", calcResult]);
        } else if (command) {
            Shell.run(query.substring(1));
        } else if (results[i]) {
            results[i].execute();
        }
        Shell.launcherOpen = false;
    }

    Rectangle {
        anchors.horizontalCenter: parent.horizontalCenter
        y: parent.height * 0.18
        width: Math.min(640, parent.width - 40)
        height: 64 + Math.min(8, Math.max(1, list.count)) * 52 + 16
        radius: Theme.radius + 4
        color: Theme.surface
        border.color: Theme.border

        MouseArea { anchors.fill: parent }

        TextField {
            id: search
            anchors { top: parent.top; left: parent.left; right: parent.right; margins: 12 }
            height: 44
            placeholderText: "Search applications   ( =  calculate,  >  run )"
            color: Theme.text
            placeholderTextColor: Theme.textDim
            font.family: Theme.font
            font.pixelSize: 16
            leftPadding: 40
            background: Rectangle {
                radius: Theme.radius
                color: Theme.bg
                border.color: search.activeFocus ? Theme.accent : Theme.border
                Icon {
                    icon: "search"
                    color: Theme.textDim
                    anchors.left: parent.left
                    anchors.leftMargin: 10
                    anchors.verticalCenter: parent.verticalCenter
                }
            }
            Keys.onDownPressed: list.incrementCurrentIndex()
            Keys.onUpPressed: list.decrementCurrentIndex()
            Keys.onReturnPressed: root.activate(list.currentIndex)
            Keys.onEnterPressed: root.activate(list.currentIndex)
            Keys.onEscapePressed: Shell.launcherOpen = false
            onTextChanged: list.currentIndex = 0
        }

        Txt {
            visible: root.calc || root.command
            anchors { top: search.bottom; left: parent.left; right: parent.right; margins: 20 }
            height: 52
            font.pixelSize: 18
            text: root.calc ? (root.calcResult !== "" ? "= " + root.calcResult + "     (Enter copies)" : "…") : "Run: " + root.query.substring(1)
        }

        ListView {
            id: list
            visible: !root.calc && !root.command
            anchors { top: search.bottom; left: parent.left; right: parent.right; bottom: parent.bottom; margins: 8; topMargin: 8 }
            clip: true
            model: root.results
            highlightMoveDuration: 80
            delegate: Rectangle {
                id: row
                required property var modelData
                required property int index
                width: list.width
                height: 52
                radius: Theme.radius
                color: ListView.isCurrentItem ? Qt.alpha(Theme.accent, 0.22) : (hover.containsMouse ? Theme.surfaceHigh : "transparent")
                AppIcon {
                    id: appIcon
                    anchors.left: parent.left
                    anchors.leftMargin: 10
                    anchors.verticalCenter: parent.verticalCenter
                    size: 32
                    icon: row.modelData.icon
                    name: row.modelData.name
                }
                Column {
                    anchors.left: appIcon.right
                    anchors.leftMargin: 12
                    anchors.right: parent.right
                    anchors.rightMargin: 10
                    anchors.verticalCenter: parent.verticalCenter
                    Txt { text: row.modelData.name; width: parent.width; font.weight: Font.Medium }
                    Txt {
                        text: row.modelData.comment || row.modelData.genericName
                        visible: text !== ""
                        width: parent.width
                        color: Theme.textDim
                        font.pixelSize: Theme.fontSize - 2
                    }
                }
                MouseArea {
                    id: hover
                    anchors.fill: parent
                    hoverEnabled: true
                    onClicked: root.activate(row.index)
                }
            }
        }
    }
}
