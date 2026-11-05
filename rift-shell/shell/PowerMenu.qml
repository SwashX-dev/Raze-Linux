import QtQuick
import Quickshell

OverlayPanel {
    id: root
    open: Shell.powerMenuOpen
    ns: "rift-shell-power"
    dim: 0.55
    onCloseRequested: Shell.powerMenuOpen = false
    onOpenChanged: if (open) { current = 0; row.forceActiveFocus(); }

    readonly property var actions: [
        { icon: "lock", label: "Lock", cmd: "rift-shell lock" },
        { icon: "bedtime", label: "Suspend", cmd: "rift-shell lock; sleep 0.5; loginctl suspend" },
        { icon: "logout", label: "Log out", cmd: "rift msg action quit" },
        { icon: "restart_alt", label: "Restart", cmd: "loginctl reboot" },
        { icon: "power_settings_new", label: "Shut down", cmd: "loginctl poweroff" }
    ]
    property int current: 0

    Row {
        id: row
        anchors.centerIn: parent
        spacing: 18
        focus: true
        Keys.onLeftPressed: root.current = (root.current + root.actions.length - 1) % root.actions.length
        Keys.onRightPressed: root.current = (root.current + 1) % root.actions.length
        Keys.onReturnPressed: root.run(root.current)
        Keys.onEscapePressed: Shell.powerMenuOpen = false

        Repeater {
            model: root.actions
            delegate: Rectangle {
                required property var modelData
                required property int index
                width: 120
                height: 120
                radius: Theme.radius + 6
                color: (root.current === index || area.containsMouse) ? Theme.accent : Theme.surface
                border.color: Theme.border
                Column {
                    anchors.centerIn: parent
                    spacing: 8
                    Icon {
                        anchors.horizontalCenter: parent.horizontalCenter
                        icon: modelData.icon
                        size: 40
                        color: (root.current === index || area.containsMouse) ? "#ffffff" : Theme.text
                    }
                    Txt {
                        anchors.horizontalCenter: parent.horizontalCenter
                        text: modelData.label
                        color: (root.current === index || area.containsMouse) ? "#ffffff" : Theme.text
                    }
                }
                MouseArea {
                    id: area
                    anchors.fill: parent
                    hoverEnabled: true
                    onEntered: root.current = index
                    onClicked: root.run(index)
                }
            }
        }
    }

    function run(i) {
        Shell.powerMenuOpen = false;
        Shell.run(actions[i].cmd);
    }
}
