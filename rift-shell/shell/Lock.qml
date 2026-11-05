import QtQuick
import QtQuick.Controls
import Quickshell
import Quickshell.Wayland
import Quickshell.Services.Pam

// Lock screen (ext-session-lock-v1), authenticated through PAM service "rift-shell".
Scope {
    id: root

    property string status: ""
    property bool busy: false
    property string password: ""

    PamContext {
        id: pam
        config: "rift-shell"
        onPamMessage: {
            if (responseRequired) respond(root.password);
            else if (messageIsError) root.status = message;
        }
        onCompleted: result => {
            root.busy = false;
            if (result === PamResult.Success) {
                root.status = "";
                Shell.locked = false;
            } else {
                root.status = "Wrong password";
            }
            root.password = "";
        }
        onError: e => {
            root.busy = false;
            root.status = "Authentication error";
        }
    }

    function tryUnlock(text) {
        if (busy || text === "") return;
        password = text;
        busy = true;
        status = "";
        pam.start();
    }

    WlSessionLock {
        id: lock
        locked: Shell.locked

        WlSessionLockSurface {
            color: "#0f0f10"

            SystemClock { id: clock; precision: SystemClock.Seconds }

            Image {
                anchors.fill: parent
                source: Settings.data.wallpaper !== "" ? "file://" + Settings.data.wallpaper : ""
                fillMode: Image.PreserveAspectCrop
                opacity: 0.35
            }

            Column {
                anchors.centerIn: parent
                spacing: 18
                width: 360

                Txt {
                    anchors.horizontalCenter: parent.horizontalCenter
                    text: Qt.formatDateTime(clock.date, Settings.data.use24h ? "HH:mm" : "h:mm")
                    font.pixelSize: 96
                    font.weight: Font.Light
                    color: "#fafafa"
                }
                Txt {
                    anchors.horizontalCenter: parent.horizontalCenter
                    text: Qt.formatDateTime(clock.date, "dddd, d MMMM")
                    font.pixelSize: 18
                    color: "#a1a1aa"
                }
                Item { width: 1; height: 24 }
                TextField {
                    id: field
                    width: parent.width
                    height: 46
                    focus: true
                    echoMode: TextInput.Password
                    enabled: !root.busy
                    placeholderText: "Password for " + Quickshell.env("USER")
                    horizontalAlignment: Text.AlignHCenter
                    color: "#fafafa"
                    placeholderTextColor: "#71717a"
                    font.pixelSize: 16
                    background: Rectangle {
                        radius: 23
                        color: "#18181b"
                        border.color: root.status !== "" ? "#ef4444" : (field.activeFocus ? Theme.accent : "#3f3f46")
                        border.width: 2
                    }
                    onAccepted: { root.tryUnlock(text); text = ""; }
                    Component.onCompleted: forceActiveFocus()
                }
                Txt {
                    anchors.horizontalCenter: parent.horizontalCenter
                    text: root.busy ? "Checking…" : root.status
                    color: root.busy ? "#a1a1aa" : "#ef4444"
                }
            }
        }
    }
}
