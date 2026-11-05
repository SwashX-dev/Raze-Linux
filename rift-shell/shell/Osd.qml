import QtQuick
import Quickshell
import Quickshell.Wayland
import Quickshell.Services.Pipewire

// On-screen display for volume and brightness changes.
PanelWindow {
    id: root
    property string icon: "volume_up"
    property real value: 0
    property bool shown: false

    screen: Shell.activeScreen
    visible: shown && !Shell.locked
    anchors { bottom: true }
    margins { bottom: 80 }
    implicitWidth: 300
    implicitHeight: 56
    exclusionMode: ExclusionMode.Ignore
    color: "transparent"
    WlrLayershell.layer: WlrLayer.Overlay
    WlrLayershell.namespace: "rift-shell-osd"

    function show(icon, value) {
        root.icon = icon;
        root.value = value;
        root.shown = true;
        hide.restart();
    }

    Timer { id: hide; interval: 1500; onTriggered: root.shown = false }

    readonly property var sink: Pipewire.defaultAudioSink
    PwObjectTracker { objects: [root.sink] }
    property bool ready: false
    Timer { running: true; interval: 2000; onTriggered: root.ready = true }
    Connections {
        target: root.sink ? root.sink.audio : null
        function onVolumeChanged() { if (root.ready) root.show(root.sink.audio.muted ? "volume_off" : "volume_up", root.sink.audio.volume); }
        function onMutedChanged() { if (root.ready) root.show(root.sink.audio.muted ? "volume_off" : "volume_up", root.sink.audio.volume); }
    }

    Rectangle {
        anchors.fill: parent
        radius: height / 2
        color: Theme.surface
        border.color: Theme.border
        Icon {
            id: ic
            icon: root.icon
            size: 24
            anchors.left: parent.left
            anchors.leftMargin: 18
            anchors.verticalCenter: parent.verticalCenter
        }
        Rectangle {
            anchors.left: ic.right
            anchors.leftMargin: 14
            anchors.right: parent.right
            anchors.rightMargin: 22
            anchors.verticalCenter: parent.verticalCenter
            height: 8
            radius: 4
            color: Theme.surfaceHigh
            Rectangle {
                width: parent.width * Math.min(1, root.value)
                height: parent.height
                radius: 4
                color: Theme.accent
                Behavior on width { NumberAnimation { duration: 80 } }
            }
        }
    }
}
