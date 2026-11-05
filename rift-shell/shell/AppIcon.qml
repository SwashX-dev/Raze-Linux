import QtQuick
import Quickshell
import Quickshell.Widgets

// An application icon from the icon theme, or a lettered tile when the theme has none.
Item {
    id: root
    property string icon: ""
    property string name: ""
    property int size: 32
    readonly property string path: icon === "" ? "" : (icon.startsWith("/") ? icon : Quickshell.iconPath(icon, true))
    implicitWidth: size
    implicitHeight: size

    IconImage {
        anchors.fill: parent
        visible: root.path !== ""
        source: root.path
        implicitSize: root.size
    }
    Rectangle {
        anchors.fill: parent
        visible: root.path === ""
        radius: root.size / 4
        color: Qt.hsla((root.name.charCodeAt(0) || 0) * 37 % 360 / 360, 0.45, 0.35, 1)
        Text {
            anchors.centerIn: parent
            text: root.name.charAt(0).toUpperCase()
            color: "#ffffff"
            font.family: Theme.font
            font.pixelSize: root.size * 0.5
            font.weight: Font.DemiBold
        }
    }
}
