import QtQuick
import Quickshell
import Quickshell.Wayland
import Quickshell.Widgets
import Quickshell.Services.Notifications

// Notification popups, top-right of the active screen.
PanelWindow {
    id: root
    screen: Shell.activeScreen
    visible: Notifs.popups.length > 0 && !Shell.locked
    anchors { top: true; right: true }
    margins { top: Theme.barHeight + 6; right: 8 }
    implicitWidth: 380
    implicitHeight: col.implicitHeight
    exclusionMode: ExclusionMode.Ignore
    color: "transparent"
    WlrLayershell.layer: WlrLayer.Overlay
    WlrLayershell.namespace: "rift-shell-notifications"

    Column {
        id: col
        width: parent.width
        spacing: 8

        Repeater {
            model: Notifs.popups.slice(-5)
            delegate: Rectangle {
                id: card
                required property var modelData
                width: col.width
                implicitHeight: body.implicitHeight + 24
                radius: Theme.radius + 2
                color: Theme.surface
                border.color: modelData.urgency === NotificationUrgency.Critical ? Theme.danger : Theme.border
                border.width: modelData.urgency === NotificationUrgency.Critical ? 2 : 1

                Timer {
                    running: card.modelData.urgency !== NotificationUrgency.Critical && !hover.containsMouse
                    interval: card.modelData.expireTimeout > 0 ? card.modelData.expireTimeout * 1000 : 6000
                    onTriggered: Notifs.dismissPopup(card.modelData)
                }

                MouseArea {
                    id: hover
                    anchors.fill: parent
                    hoverEnabled: true
                    onClicked: {
                        const def = card.modelData.actions.find(a => a.identifier === "default");
                        if (def) def.invoke();
                        Notifs.dismissPopup(card.modelData);
                    }
                }

                Row {
                    id: body
                    x: 12
                    y: 12
                    width: parent.width - 24
                    spacing: 12

                    IconImage {
                        visible: source.toString() !== ""
                        implicitSize: 40
                        source: card.modelData.image !== "" ? card.modelData.image : (card.modelData.appIcon === "" ? "" : card.modelData.appIcon.startsWith("/") || card.modelData.appIcon.startsWith("file:") ? card.modelData.appIcon : Quickshell.iconPath(card.modelData.appIcon, true))
                    }

                    Column {
                        width: parent.width - (parent.children[0].visible ? 52 : 0)
                        spacing: 4
                        Txt {
                            width: parent.width
                            text: card.modelData.appName
                            color: Theme.textDim
                            font.pixelSize: Theme.fontSize - 2
                        }
                        Txt { width: parent.width; text: card.modelData.summary; font.weight: Font.DemiBold }
                        Txt {
                            width: parent.width
                            visible: text !== ""
                            text: card.modelData.body
                            wrapMode: Text.Wrap
                            maximumLineCount: 4
                            textFormat: Text.StyledText
                            color: Theme.textDim
                        }
                        Row {
                            spacing: 6
                            visible: card.modelData.actions.length > 0
                            Repeater {
                                model: card.modelData.actions.filter(a => a.identifier !== "default")
                                delegate: BarButton {
                                    required property var modelData
                                    label: modelData.text
                                    color: Theme.surfaceHigh
                                    onClicked: { modelData.invoke(); Notifs.dismissPopup(card.modelData); }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
