import QtQuick
import Quickshell
import Quickshell.Wayland

// Full-screen layer-shell overlay on the active screen: dims the desktop, takes the
// keyboard, closes on Escape or a click outside the content.
PanelWindow {
    id: root
    property bool open: false
    property string ns: "rift-shell"
    property real dim: 0.35
    default property alias content: holder.data
    signal closeRequested()

    visible: open
    screen: Shell.activeScreen
    anchors { top: true; bottom: true; left: true; right: true }
    exclusionMode: ExclusionMode.Ignore
    color: "transparent"
    WlrLayershell.layer: WlrLayer.Overlay
    WlrLayershell.namespace: ns
    WlrLayershell.keyboardFocus: open ? WlrKeyboardFocus.Exclusive : WlrKeyboardFocus.None

    Rectangle {
        anchors.fill: parent
        color: Qt.rgba(0, 0, 0, root.dim)
        MouseArea {
            anchors.fill: parent
            onClicked: root.closeRequested()
        }
    }

    Item {
        id: holder
        anchors.fill: parent
        focus: true
        Keys.onEscapePressed: root.closeRequested()
    }
}
