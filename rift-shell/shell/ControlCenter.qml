import QtQuick
import QtQuick.Layouts
import QtQuick.Controls
import Quickshell
import Quickshell.Io
import Quickshell.Widgets
import Quickshell.Services.Pipewire
import Quickshell.Services.UPower
import Quickshell.Services.Mpris
import Quickshell.Networking
import Quickshell.Bluetooth

// Quick settings, media, Wi-Fi/Bluetooth lists and notification history.
OverlayPanel {
    id: root
    open: Shell.controlCenterOpen
    ns: "rift-shell-control-center"
    dim: 0
    onCloseRequested: Shell.controlCenterOpen = false
    onOpenChanged: if (open) { brightnessRead.running = true; uptime.reload(); page = "main"; }

    property string page: "main"   // main | wifi | bluetooth
    property real brightness: -1
    readonly property var sink: Pipewire.defaultAudioSink
    readonly property var source: Pipewire.defaultAudioSource
    readonly property var wifi: Networking.devices.values.find(d => d.type === DeviceType.Wifi) || null
    readonly property var activeWifi: wifi ? wifi.networks.values.find(n => n.connected) || null : null
    readonly property var wired: Networking.devices.values.find(d => d.type === DeviceType.Wired && d.connected) || null
    readonly property var adapter: Bluetooth.defaultAdapter
    readonly property var player: Mpris.players.values.find(p => p.isPlaying) || Mpris.players.values[0] || null

    PwObjectTracker { objects: [root.sink, root.source] }

    Process {
        id: brightnessRead
        command: ["brightnessctl", "-m", "-c", "backlight"]
        stdout: StdioCollector {
            onStreamFinished: {
                const f = text.trim().split(",");
                root.brightness = f.length >= 4 ? parseInt(f[3]) / 100 : -1;
            }
        }
    }

    FileView {
        id: uptime
        path: "/proc/uptime"
        readonly property int secs: parseInt(text().split(" ")[0]) || 0
    }

    function powerProfileName(p) {
        return p === PowerProfile.PowerSaver ? "Power saver" : p === PowerProfile.Performance ? "Performance" : "Balanced";
    }

    Rectangle {
        id: panel
        anchors.top: parent.top
        anchors.right: parent.right
        anchors.topMargin: Theme.barHeight + 6
        anchors.rightMargin: 8
        width: 400
        height: Math.min(parent.height - Theme.barHeight - 20, content.implicitHeight + 28)
        radius: Theme.radius + 4
        color: Theme.surface
        border.color: Theme.border
        MouseArea { anchors.fill: parent }

        Flickable {
            anchors.fill: parent
            anchors.margins: 14
            contentHeight: content.implicitHeight
            clip: true

            ColumnLayout {
                id: content
                width: parent.width
                spacing: 12

                // Header
                RowLayout {
                    Layout.fillWidth: true
                    Column {
                        Layout.fillWidth: true
                        Txt { text: Quickshell.env("USER"); font.pixelSize: 16; font.weight: Font.DemiBold }
                        Txt {
                            text: "up " + Math.floor(uptime.secs / 3600) + " h " + Math.floor(uptime.secs % 3600 / 60) + " min"
                            color: Theme.textDim
                            font.pixelSize: Theme.fontSize - 1
                        }
                    }
                    BarButton { icon: "screenshot_region"; onClicked: { Shell.controlCenterOpen = false; Shell.run("sleep 0.3; rift-shell screenshot region"); } }
                    BarButton { icon: "palette"; onClicked: Settings.data.theme = Theme.dark ? "light" : "dark" }
                    BarButton { icon: "lock"; onClicked: Shell.lock() }
                    BarButton { icon: "power_settings_new"; iconColor: Theme.accent; onClicked: Shell.toggle("powerMenuOpen") }
                }

                // Main page: toggles, sliders, media, notifications
                GridLayout {
                    visible: root.page === "main"
                    Layout.fillWidth: true
                    columns: 2
                    columnSpacing: 8
                    rowSpacing: 8

                    Toggle {
                        Layout.fillWidth: true
                        icon: root.wired ? "lan" : "wifi"
                        title: root.wired ? "Ethernet" : "Wi-Fi"
                        subtitle: root.wired ? "Connected" : (root.activeWifi ? root.activeWifi.name : (Networking.wifiEnabled ? "Not connected" : "Off"))
                        checked: root.wired !== null || Networking.wifiEnabled
                        onToggled: Networking.wifiEnabled = !Networking.wifiEnabled
                        onPressAndHold: root.page = "wifi"
                        Icon {
                            visible: root.wifi !== null
                            anchors.right: parent.right; anchors.rightMargin: 8; anchors.verticalCenter: parent.verticalCenter
                            icon: "chevron_right"
                            color: parent.checked ? "#ffffff" : Theme.textDim
                            MouseArea { anchors.fill: parent; anchors.margins: -6; onClicked: root.page = "wifi" }
                        }
                    }
                    Toggle {
                        Layout.fillWidth: true
                        visible: root.adapter !== null
                        icon: "bluetooth"
                        title: "Bluetooth"
                        subtitle: root.adapter && root.adapter.enabled ? (Bluetooth.devices.values.filter(d => d.connected).map(d => d.name).join(", ") || "On") : "Off"
                        checked: root.adapter !== null && root.adapter.enabled
                        onToggled: root.adapter.enabled = !root.adapter.enabled
                        onPressAndHold: root.page = "bluetooth"
                        Icon {
                            anchors.right: parent.right; anchors.rightMargin: 8; anchors.verticalCenter: parent.verticalCenter
                            icon: "chevron_right"
                            color: parent.checked ? "#ffffff" : Theme.textDim
                            MouseArea { anchors.fill: parent; anchors.margins: -6; onClicked: root.page = "bluetooth" }
                        }
                    }
                    Toggle {
                        Layout.fillWidth: true
                        icon: "do_not_disturb_on"
                        title: "Do not disturb"
                        subtitle: Notifs.dnd ? "Only urgent" : "Off"
                        checked: Notifs.dnd
                        onToggled: Notifs.setDnd(!Notifs.dnd)
                    }
                    Toggle {
                        Layout.fillWidth: true
                        icon: "nightlight"
                        title: "Night light"
                        subtitle: Settings.data.nightLight ? Settings.data.nightLightTemp + " K" : "Off"
                        checked: Settings.data.nightLight
                        onToggled: Settings.data.nightLight = !Settings.data.nightLight
                    }
                    Toggle {
                        Layout.fillWidth: true
                        icon: "speed"
                        title: "Power mode"
                        subtitle: root.powerProfileName(PowerProfiles.profile)
                        checked: PowerProfiles.profile === PowerProfile.Performance
                        onToggled: PowerProfiles.profile = PowerProfiles.profile === PowerProfile.Balanced
                            ? (PowerProfiles.hasPerformanceProfile ? PowerProfile.Performance : PowerProfile.PowerSaver)
                            : PowerProfiles.profile === PowerProfile.Performance ? PowerProfile.PowerSaver : PowerProfile.Balanced
                    }
                    Toggle {
                        Layout.fillWidth: true
                        icon: "coffee"
                        title: "Stay awake"
                        subtitle: Shell.caffeine ? "Idle lock paused" : "Off"
                        checked: Shell.caffeine
                        onToggled: Shell.caffeine = !Shell.caffeine
                    }
                    Toggle {
                        Layout.fillWidth: true
                        icon: "radio_button_checked"
                        title: "Record screen"
                        subtitle: Shell.recording ? "Recording…" : "Off"
                        checked: Shell.recording
                        onToggled: { Shell.controlCenterOpen = false; Shell.run("rift-shell record toggle"); }
                    }
                    Toggle {
                        Layout.fillWidth: true
                        icon: "content_paste"
                        title: "Clipboard"
                        subtitle: "History"
                        onToggled: Shell.toggle("clipboardOpen")
                    }
                }

                SliderRow {
                    visible: root.page === "main" && root.sink !== null
                    Layout.fillWidth: true
                    icon: root.sink && root.sink.audio && root.sink.audio.muted ? "volume_off" : "volume_up"
                    value: root.sink && root.sink.audio ? root.sink.audio.volume : 0
                    muted: root.sink && root.sink.audio ? root.sink.audio.muted : false
                    onMoved: v => root.sink.audio.volume = v
                    onIconClicked: root.sink.audio.muted = !root.sink.audio.muted
                }
                SliderRow {
                    visible: root.page === "main" && root.source !== null
                    Layout.fillWidth: true
                    icon: root.source && root.source.audio && root.source.audio.muted ? "mic_off" : "mic"
                    value: root.source && root.source.audio ? root.source.audio.volume : 0
                    muted: root.source && root.source.audio ? root.source.audio.muted : false
                    onMoved: v => root.source.audio.volume = v
                    onIconClicked: root.source.audio.muted = !root.source.audio.muted
                }
                SliderRow {
                    visible: root.page === "main" && root.brightness >= 0
                    Layout.fillWidth: true
                    icon: "brightness_medium"
                    value: root.brightness
                    onMoved: v => {
                        root.brightness = Math.max(0.01, v);
                        Quickshell.execDetached(["brightnessctl", "-q", "-c", "backlight", "set", Math.round(root.brightness * 100) + "%"]);
                    }
                }

                // Media
                Rectangle {
                    visible: root.page === "main" && root.player !== null
                    Layout.fillWidth: true
                    implicitHeight: 84
                    radius: Theme.radius
                    color: Theme.bg
                    border.color: Theme.border
                    Image {
                        id: art
                        anchors.left: parent.left
                        anchors.leftMargin: 10
                        anchors.verticalCenter: parent.verticalCenter
                        width: 64
                        height: 64
                        fillMode: Image.PreserveAspectCrop
                        source: root.player ? root.player.trackArtUrl : ""
                        visible: status === Image.Ready
                    }
                    Column {
                        anchors.left: art.visible ? art.right : parent.left
                        anchors.leftMargin: 12
                        anchors.right: controls.left
                        anchors.verticalCenter: parent.verticalCenter
                        Txt { width: parent.width; text: root.player ? root.player.trackTitle || root.player.identity : ""; font.weight: Font.DemiBold }
                        Txt { width: parent.width; text: root.player ? root.player.trackArtist : ""; color: Theme.textDim }
                    }
                    Row {
                        id: controls
                        anchors.right: parent.right
                        anchors.rightMargin: 6
                        anchors.verticalCenter: parent.verticalCenter
                        BarButton { icon: "skip_previous"; onClicked: root.player.previous() }
                        BarButton { icon: root.player && root.player.isPlaying ? "pause" : "play_arrow"; onClicked: root.player.togglePlaying() }
                        BarButton { icon: "skip_next"; onClicked: root.player.next() }
                    }
                }

                // Notifications
                RowLayout {
                    visible: root.page === "main"
                    Layout.fillWidth: true
                    Txt { Layout.fillWidth: true; text: "Notifications"; font.weight: Font.DemiBold }
                    BarButton { visible: Notifs.history.length > 0; label: "Clear"; icon: "clear_all"; onClicked: Notifs.clearHistory() }
                }
                Txt {
                    visible: root.page === "main" && Notifs.history.length === 0
                    text: "No notifications"
                    color: Theme.textDim
                }
                Repeater {
                    model: root.page === "main" ? Notifs.history.slice(0, 30) : []
                    delegate: Rectangle {
                        required property var modelData
                        Layout.fillWidth: true
                        implicitHeight: col.implicitHeight + 16
                        radius: Theme.radius
                        color: Theme.bg
                        border.color: modelData.urgency === 2 ? Theme.danger : Theme.border
                        Column {
                            id: col
                            anchors.left: parent.left
                            anchors.right: close.left
                            anchors.margins: 10
                            anchors.verticalCenter: parent.verticalCenter
                            spacing: 2
                            Txt {
                                width: parent.width
                                text: (modelData.appName || "Notification") + " · " + Qt.formatDateTime(new Date(modelData.time), "HH:mm")
                                color: Theme.textDim
                                font.pixelSize: Theme.fontSize - 2
                            }
                            Txt { width: parent.width; text: modelData.summary; font.weight: Font.DemiBold }
                            Txt {
                                width: parent.width
                                visible: text !== ""
                                text: modelData.body
                                wrapMode: Text.Wrap
                                maximumLineCount: 3
                                textFormat: Text.StyledText
                                color: Theme.textDim
                            }
                        }
                        BarButton {
                            id: close
                            anchors.right: parent.right
                            anchors.top: parent.top
                            anchors.margins: 4
                            icon: "close"
                            onClicked: Notifs.remove(modelData.id)
                        }
                    }
                }

                // Wi-Fi page
                RowLayout {
                    visible: root.page !== "main"
                    Layout.fillWidth: true
                    BarButton { icon: "arrow_back"; onClicked: root.page = "main" }
                    Txt { Layout.fillWidth: true; text: root.page === "wifi" ? "Wi-Fi networks" : "Bluetooth devices"; font.weight: Font.DemiBold }
                    BarButton {
                        visible: root.page === "bluetooth" && root.adapter !== null
                        icon: root.adapter && root.adapter.discovering ? "progress_activity" : "refresh"
                        onClicked: root.adapter.discovering = !root.adapter.discovering
                    }
                }
                Repeater {
                    model: root.page === "wifi" && root.wifi ? root.wifi.networks.values.slice().sort((a, b) => b.connected - a.connected || b.signalStrength - a.signalStrength) : []
                    delegate: Rectangle {
                        id: net
                        required property var modelData
                        property bool askPsk: false
                        Layout.fillWidth: true
                        implicitHeight: askPsk ? 92 : 44
                        radius: Theme.radius
                        color: modelData.connected ? Qt.alpha(Theme.accent, 0.2) : (netArea.containsMouse ? Theme.surfaceHigh : Theme.bg)
                        RowLayout {
                            x: 10
                            width: parent.width - 20
                            height: 44
                            Icon { icon: net.modelData.signalStrength > 0.66 ? "wifi" : net.modelData.signalStrength > 0.33 ? "wifi_2_bar" : "wifi_1_bar" }
                            Txt { Layout.fillWidth: true; text: net.modelData.name }
                            Icon { visible: net.modelData.security !== WifiSecurityType.Open; icon: "lock"; size: 14; color: Theme.textDim }
                            Txt { visible: net.modelData.connected; text: "Connected"; color: Theme.textDim }
                        }
                        MouseArea {
                            id: netArea
                            width: parent.width
                            height: 44
                            hoverEnabled: true
                            onClicked: {
                                if (net.modelData.connected) net.modelData.disconnect();
                                else if (net.modelData.known || net.modelData.security === WifiSecurityType.Open) net.modelData.connect();
                                else { net.askPsk = true; psk.forceActiveFocus(); }
                            }
                        }
                        TextField {
                            id: psk
                            visible: net.askPsk
                            x: 10
                            y: 48
                            width: parent.width - 20
                            height: 36
                            echoMode: TextInput.Password
                            placeholderText: "Password"
                            color: Theme.text
                            background: Rectangle { radius: 8; color: Theme.surface; border.color: Theme.accent }
                            onAccepted: { net.modelData.connectWithPsk(text); net.askPsk = false; text = ""; }
                        }
                    }
                }
                Repeater {
                    model: root.page === "bluetooth" ? Bluetooth.devices.values.slice().sort((a, b) => b.connected - a.connected || b.paired - a.paired) : []
                    delegate: Rectangle {
                        required property var modelData
                        Layout.fillWidth: true
                        implicitHeight: 44
                        radius: Theme.radius
                        color: modelData.connected ? Qt.alpha(Theme.accent, 0.2) : (btArea.containsMouse ? Theme.surfaceHigh : Theme.bg)
                        RowLayout {
                            anchors.fill: parent
                            anchors.leftMargin: 10
                            anchors.rightMargin: 10
                            Icon { icon: modelData.connected ? "bluetooth_connected" : "bluetooth" }
                            Txt { Layout.fillWidth: true; text: modelData.name || modelData.address }
                            Txt {
                                text: modelData.connected ? (modelData.batteryAvailable ? Math.round(modelData.battery * 100) + "%" : "Connected") : (modelData.paired ? "Paired" : "")
                                color: Theme.textDim
                            }
                        }
                        MouseArea {
                            id: btArea
                            anchors.fill: parent
                            hoverEnabled: true
                            onClicked: {
                                if (modelData.connected) modelData.disconnect();
                                else if (modelData.paired) modelData.connect();
                                else { modelData.trusted = true; modelData.pair(); }
                            }
                        }
                    }
                }
            }
        }
    }
}
