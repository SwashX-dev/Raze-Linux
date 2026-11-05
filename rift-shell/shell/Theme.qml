pragma Singleton
import QtQuick
import Quickshell

// Colours and metrics shared by every component. Follows Settings (dark/light, accent).
Singleton {
    readonly property bool dark: Settings.data.theme !== "light"
    readonly property color accent: Settings.data.accent
    readonly property color bg: dark ? "#0f0f10" : "#f4f4f5"
    readonly property color surface: dark ? "#18181b" : "#ffffff"
    readonly property color surfaceHigh: dark ? "#27272a" : "#e4e4e7"
    readonly property color border: dark ? "#3f3f46" : "#d4d4d8"
    readonly property color text: dark ? "#fafafa" : "#18181b"
    readonly property color textDim: dark ? "#a1a1aa" : "#52525b"
    readonly property color danger: "#ef4444"
    readonly property color barBg: Qt.rgba(bg.r, bg.g, bg.b, 0.92)
    readonly property string font: "Inter"
    readonly property string mono: "JetBrains Mono"
    readonly property int radius: 10
    readonly property int barHeight: 34
    readonly property int spacing: 8
    readonly property int fontSize: 13
}
