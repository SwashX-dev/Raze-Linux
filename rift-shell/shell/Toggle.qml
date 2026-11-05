import QtQuick

// A control-centre tile: icon, title, subtitle; toggles on click.
Rectangle {
    id: root
    property string icon: ""
    property string title: ""
    property string subtitle: ""
    property bool checked: false
    signal toggled()
    signal pressAndHold()

    implicitHeight: 56
    radius: Theme.radius
    color: checked ? Theme.accent : (mouse.containsMouse ? Theme.surfaceHigh : Theme.surface)
    border.color: checked ? "transparent" : Theme.border
    border.width: 1
    Behavior on color { ColorAnimation { duration: 120 } }

    Row {
        anchors.fill: parent
        anchors.margins: 10
        spacing: 10
        Icon {
            icon: root.icon
            size: 22
            filled: root.checked
            color: root.checked ? "#ffffff" : Theme.text
            anchors.verticalCenter: parent.verticalCenter
        }
        Column {
            anchors.verticalCenter: parent.verticalCenter
            width: parent.width - 42
            Txt {
                text: root.title
                width: parent.width
                font.weight: Font.DemiBold
                color: root.checked ? "#ffffff" : Theme.text
            }
            Txt {
                visible: text !== ""
                text: root.subtitle
                width: parent.width
                font.pixelSize: Theme.fontSize - 2
                color: root.checked ? "#fde8e8" : Theme.textDim
            }
        }
    }

    MouseArea {
        id: mouse
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: Qt.PointingHandCursor
        onClicked: root.toggled()
        onPressAndHold: root.pressAndHold()
    }
}
