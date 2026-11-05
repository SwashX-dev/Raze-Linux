import QtQuick
import QtQuick.Controls
import Quickshell
import Quickshell.Services.Polkit

// Polkit authentication agent.
OverlayPanel {
    id: root
    readonly property var flow: agent.flow
    open: flow !== null
    ns: "rift-shell-polkit"
    dim: 0.5
    onCloseRequested: if (flow) flow.cancelAuthenticationRequest()
    onOpenChanged: if (open) { pass.text = ""; pass.forceActiveFocus(); }

    PolkitAgent { id: agent }

    Rectangle {
        anchors.centerIn: parent
        width: 420
        height: col.implicitHeight + 40
        radius: Theme.radius + 4
        color: Theme.surface
        border.color: Theme.border
        MouseArea { anchors.fill: parent }

        Column {
            id: col
            anchors.centerIn: parent
            width: parent.width - 40
            spacing: 14
            Row {
                spacing: 10
                Icon { icon: "admin_panel_settings"; size: 28; color: Theme.accent }
                Txt { text: "Authentication required"; font.pixelSize: 17; font.weight: Font.DemiBold; anchors.verticalCenter: parent.verticalCenter }
            }
            Txt {
                width: parent.width
                text: root.flow ? root.flow.message : ""
                wrapMode: Text.Wrap
                color: Theme.textDim
            }
            TextField {
                id: pass
                width: parent.width
                height: 42
                echoMode: root.flow && root.flow.responseVisible ? TextInput.Normal : TextInput.Password
                placeholderText: root.flow ? root.flow.inputPrompt : ""
                color: Theme.text
                background: Rectangle { radius: Theme.radius; color: Theme.bg; border.color: pass.activeFocus ? Theme.accent : Theme.border }
                onAccepted: if (root.flow) root.flow.submit(text)
            }
            Txt {
                visible: root.flow && root.flow.supplementaryMessage !== ""
                width: parent.width
                text: root.flow ? root.flow.supplementaryMessage : ""
                color: root.flow && root.flow.supplementaryIsError ? Theme.danger : Theme.textDim
                wrapMode: Text.Wrap
            }
            Row {
                anchors.right: parent.right
                spacing: 8
                BarButton { label: "Cancel"; onClicked: root.flow.cancelAuthenticationRequest() }
                BarButton { label: "Authenticate"; active: true; onClicked: root.flow.submit(pass.text) }
            }
        }
    }
}
