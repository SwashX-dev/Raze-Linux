import QtQuick

// Icon + horizontal slider (0..1). Emits moved(value) while dragging.
Item {
    id: root
    property string icon: ""
    property real value: 0
    property bool muted: false
    signal moved(real value)
    signal iconClicked()

    implicitHeight: 36

    Icon {
        id: ic
        icon: root.icon
        size: 22
        color: root.muted ? Theme.textDim : Theme.text
        anchors.left: parent.left
        anchors.verticalCenter: parent.verticalCenter
        MouseArea {
            anchors.fill: parent
            cursorShape: Qt.PointingHandCursor
            onClicked: root.iconClicked()
        }
    }

    Rectangle {
        id: track
        anchors.left: ic.right
        anchors.leftMargin: 12
        anchors.right: pct.left
        anchors.rightMargin: 10
        anchors.verticalCenter: parent.verticalCenter
        height: 8
        radius: 4
        color: Theme.surfaceHigh
        Rectangle {
            width: Math.max(height, parent.width * Math.min(1, root.value))
            height: parent.height
            radius: 4
            color: root.muted ? Theme.textDim : Theme.accent
        }
        Rectangle {
            x: Math.max(0, Math.min(parent.width - width, parent.width * Math.min(1, root.value) - width / 2))
            anchors.verticalCenter: parent.verticalCenter
            width: 18
            height: 18
            radius: 9
            color: "#ffffff"
            border.color: Theme.border
        }
        MouseArea {
            anchors.fill: parent
            anchors.margins: -8
            cursorShape: Qt.PointingHandCursor
            function update(m) {
                root.moved(Math.max(0, Math.min(1, (m.x - 8) / track.width)));
            }
            onPressed: m => update(m)
            onPositionChanged: m => { if (pressed) update(m); }
        }
    }

    Txt {
        id: pct
        anchors.right: parent.right
        anchors.verticalCenter: parent.verticalCenter
        width: 40
        horizontalAlignment: Text.AlignRight
        text: Math.round(root.value * 100) + "%"
        color: Theme.textDim
        font.family: Theme.mono
    }
}
