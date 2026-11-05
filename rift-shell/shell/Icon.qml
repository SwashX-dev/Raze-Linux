import QtQuick

// A Material Symbols glyph (ligature name, e.g. "wifi").
Text {
    property string icon: ""
    property real size: 18
    property bool filled: false
    text: icon
    color: Theme.text
    font.family: "Material Symbols Rounded"
    font.pixelSize: size
    font.variableAxes: ({ "FILL": filled ? 1 : 0, "wght": 400, "opsz": 24 })
    verticalAlignment: Text.AlignVCenter
    horizontalAlignment: Text.AlignHCenter
}
