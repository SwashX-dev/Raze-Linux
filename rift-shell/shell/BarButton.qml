import QtQuick

// A rounded, hoverable area in the bar holding an icon and optional text.
Rectangle {
    id: root
    property string icon: ""
    property string label: ""
    property color iconColor: Theme.text
    property bool active: false
    signal clicked(var mouse)
    signal scrolled(int delta)

    implicitHeight: Theme.barHeight - 8
    implicitWidth: row.implicitWidth + 16
    radius: height / 2
    color: active ? Qt.alpha(Theme.accent, 0.25) : (mouse.containsMouse ? Theme.surfaceHigh : "transparent")
    Behavior on color { ColorAnimation { duration: 120 } }

    Row {
        id: row
        anchors.centerIn: parent
        spacing: 6
        Icon {
            visible: root.icon !== ""
            icon: root.icon
            color: root.iconColor
            anchors.verticalCenter: parent.verticalCenter
        }
        Txt {
            visible: root.label !== ""
            text: root.label
            anchors.verticalCenter: parent.verticalCenter
        }
    }

    MouseArea {
        id: mouse
        anchors.fill: parent
        hoverEnabled: true
        acceptedButtons: Qt.LeftButton | Qt.RightButton | Qt.MiddleButton
        cursorShape: Qt.PointingHandCursor
        onClicked: m => root.clicked(m)
        onWheel: w => root.scrolled(w.angleDelta.y)
    }
}
