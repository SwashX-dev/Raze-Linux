import QtQuick
import QtQuick.Layouts
import Qt.labs.folderlistmodel
import Quickshell
import Quickshell.Wayland
import Quickshell.Widgets
import Quickshell.Services.Pipewire
import Quickshell.Services.UPower
import Quickshell.Services.Mpris
import Quickshell.Services.SystemTray
import Quickshell.Networking
import Quickshell.Bluetooth

// The top panel, one per screen.
PanelWindow {
    id: bar
    required property var modelData
    screen: modelData
    anchors { top: true; left: true; right: true }
    implicitHeight: Theme.barHeight
    color: "transparent"
    WlrLayershell.namespace: "rift-shell-bar"
    WlrLayershell.layer: WlrLayer.Top

    readonly property var sink: Pipewire.defaultAudioSink
    PwObjectTracker { objects: [bar.sink] }

    readonly property var wifi: Networking.devices.values.find(d => d.type === DeviceType.Wifi) || null
    readonly property var wired: Networking.devices.values.find(d => d.type === DeviceType.Wired && d.connected) || null
    readonly property var activeWifi: wifi ? wifi.networks.values.find(n => n.connected) || null : null
    readonly property var player: Mpris.players.values.find(p => p.isPlaying) || Mpris.players.values[0] || null
    readonly property var battery: UPower.displayDevice

    Rectangle {
        anchors.fill: parent
        color: Theme.barBg
        Rectangle {
            anchors.bottom: parent.bottom
            width: parent.width
            height: 1
            color: Theme.border
        }
    }

    // Left: launcher button, workspaces, focused window.
    RowLayout {
        anchors.left: parent.left
        anchors.leftMargin: 6
        anchors.verticalCenter: parent.verticalCenter
        spacing: 4

        BarButton {
            icon: "apps"
            iconColor: Theme.accent
            onClicked: Shell.toggle("launcherOpen")
        }

        Row {
            spacing: 4
            Repeater {
                model: Rift.workspacesOn(bar.screen ? bar.screen.name : "")
                delegate: Rectangle {
                    required property var modelData
                    readonly property bool isActive: modelData.active
                    width: isActive ? 28 : (modelData.windows > 0 ? 14 : 10)
                    height: 10
                    radius: 5
                    anchors.verticalCenter: parent.verticalCenter
                    color: isActive ? Theme.accent : (modelData.windows > 0 ? Theme.textDim : Theme.border)
                    Behavior on width { NumberAnimation { duration: 160; easing.type: Easing.OutCubic } }
                    MouseArea {
                        anchors.fill: parent
                        anchors.margins: -4
                        cursorShape: Qt.PointingHandCursor
                        onClicked: Rift.action("workspace " + (modelData.index + 1))
                    }
                }
            }
        }

        Txt {
            Layout.leftMargin: 8
            Layout.maximumWidth: bar.width * 0.28
            text: Rift.focused ? (Rift.focused.title || Rift.focused.app_id) : ""
            color: Theme.textDim
        }
    }

    // Centre: clock (click opens the control centre with the calendar).
    BarButton {
        anchors.centerIn: parent
        SystemClock {
            id: clock
            precision: Settings.data.showSeconds ? SystemClock.Seconds : SystemClock.Minutes
        }
        label: Qt.formatDateTime(clock.date, (Settings.data.use24h ? "HH:mm" : "h:mm AP") + (Settings.data.showSeconds ? ":ss" : "") + "  ddd d MMM")
        onClicked: Shell.toggle("controlCenterOpen")
    }

    // Right: media, tray, plugins, status, notifications, power.
    RowLayout {
        anchors.right: parent.right
        anchors.rightMargin: 6
        anchors.verticalCenter: parent.verticalCenter
        spacing: 2

        BarButton {
            visible: bar.player !== null
            icon: bar.player && bar.player.isPlaying ? "pause" : "play_arrow"
            label: bar.player ? (bar.player.trackTitle || bar.player.identity).substring(0, 32) : ""
            onClicked: m => {
                if (m.button === Qt.RightButton) bar.player.next();
                else if (m.button === Qt.MiddleButton) bar.player.previous();
                else bar.player.togglePlaying();
            }
        }

        Repeater {
            model: SystemTray.items
            delegate: Item {
                id: trayItem
                required property var modelData
                implicitWidth: 26
                implicitHeight: 26
                IconImage {
                    anchors.centerIn: parent
                    implicitSize: 18
                    source: trayItem.modelData.icon
                }
                MouseArea {
                    anchors.fill: parent
                    acceptedButtons: Qt.LeftButton | Qt.RightButton
                    onClicked: m => {
                        if (m.button === Qt.RightButton || trayItem.modelData.onlyMenu) {
                            if (trayItem.modelData.hasMenu) {
                                const p = trayItem.mapToItem(null, 0, trayItem.height);
                                trayItem.modelData.display(bar, p.x, p.y);
                            }
                        } else {
                            trayItem.modelData.activate();
                        }
                    }
                }
            }
        }

        // Plugins: ~/.config/rift-shell/plugins/*.qml (see docs/rift-shell.md).
        Repeater {
            model: FolderListModel {
                folder: "file://" + Settings.dir + "/plugins"
                nameFilters: ["*.qml"]
                showDirs: false
            }
            delegate: Loader {
                required property string fileName
                required property url fileUrl
                active: !Settings.data.disabledPlugins.includes(fileName)
                source: fileUrl
                onLoaded: {
                    if (item.hasOwnProperty("shell")) item.shell = { theme: Theme, rift: Rift, settings: Settings, notifs: Notifs, run: Shell.run };
                }
            }
        }

        BarButton {
            icon: bar.wired ? "lan" : (bar.activeWifi ? (bar.activeWifi.signalStrength > 0.66 ? "wifi" : bar.activeWifi.signalStrength > 0.33 ? "wifi_2_bar" : "wifi_1_bar") : (Networking.wifiEnabled ? "wifi_off" : "signal_wifi_off"))
            iconColor: (bar.wired || bar.activeWifi) ? Theme.text : Theme.textDim
            onClicked: Shell.toggle("controlCenterOpen")
        }

        BarButton {
            visible: Bluetooth.defaultAdapter !== null
            icon: Bluetooth.defaultAdapter && Bluetooth.defaultAdapter.enabled ? (Bluetooth.devices.values.some(d => d.connected) ? "bluetooth_connected" : "bluetooth") : "bluetooth_disabled"
            iconColor: Bluetooth.defaultAdapter && Bluetooth.defaultAdapter.enabled ? Theme.text : Theme.textDim
            onClicked: Shell.toggle("controlCenterOpen")
        }

        BarButton {
            readonly property real vol: bar.sink && bar.sink.audio ? bar.sink.audio.volume : 0
            readonly property bool muted: bar.sink && bar.sink.audio ? bar.sink.audio.muted : true
            icon: muted ? "volume_off" : vol > 0.6 ? "volume_up" : vol > 0.2 ? "volume_down" : "volume_mute"
            label: bar.sink ? Math.round(vol * 100) + "%" : ""
            onClicked: m => {
                if (m.button === Qt.RightButton && bar.sink) bar.sink.audio.muted = !bar.sink.audio.muted;
                else Shell.toggle("controlCenterOpen");
            }
            onScrolled: d => {
                if (bar.sink && bar.sink.audio) bar.sink.audio.volume = Math.max(0, Math.min(1, bar.sink.audio.volume + (d > 0 ? 0.05 : -0.05)));
            }
        }

        BarButton {
            visible: bar.battery && bar.battery.isLaptopBattery
            readonly property real pct: bar.battery ? (bar.battery.percentage > 1 ? bar.battery.percentage / 100 : bar.battery.percentage) : 0
            icon: !UPower.onBattery ? "battery_charging_full" : pct > 0.9 ? "battery_full" : pct > 0.6 ? "battery_5_bar" : pct > 0.3 ? "battery_3_bar" : pct > 0.1 ? "battery_1_bar" : "battery_alert"
            iconColor: UPower.onBattery && pct <= 0.1 ? Theme.danger : Theme.text
            label: Math.round(pct * 100) + "%"
            onClicked: Shell.toggle("controlCenterOpen")
        }

        BarButton {
            visible: Shell.recording
            icon: "radio_button_checked"
            iconColor: Theme.danger
            onClicked: Shell.run("rift-shell record stop")
        }

        BarButton {
            icon: Notifs.dnd ? "notifications_off" : (Notifs.history.length > 0 ? "notifications_unread" : "notifications")
            onClicked: m => {
                if (m.button === Qt.RightButton) Notifs.setDnd(!Notifs.dnd);
                else Shell.toggle("controlCenterOpen");
            }
        }

        BarButton {
            icon: "power_settings_new"
            iconColor: Theme.accent
            onClicked: Shell.toggle("powerMenuOpen")
        }
    }
}
