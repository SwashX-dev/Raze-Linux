import QtQuick
import Quickshell
import Quickshell.Wayland

// Desktop background (layer-shell background layer), one per screen.
PanelWindow {
    required property var modelData
    screen: modelData
    anchors { top: true; bottom: true; left: true; right: true }
    exclusionMode: ExclusionMode.Ignore
    color: Theme.bg
    WlrLayershell.layer: WlrLayer.Background
    WlrLayershell.namespace: "rift-shell-wallpaper"

    Image {
        anchors.fill: parent
        source: Settings.data.wallpaper !== "" ? "file://" + Settings.data.wallpaper : Quickshell.shellPath("assets/wallpaper.svg")
        sourceSize: Qt.size(width, height)
        fillMode: Image.PreserveAspectCrop
        asynchronous: true
        cache: false
    }
}
