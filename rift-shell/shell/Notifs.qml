pragma Singleton
import QtQuick
import Quickshell
import Quickshell.Io
import Quickshell.Services.Notifications

// Notification daemon (org.freedesktop.Notifications) with history and Do Not Disturb.
Singleton {
    id: root
    property bool dnd: Settings.data.dnd
    // Notifications currently shown as popups (Notification objects).
    property list<QtObject> popups: []
    // History as plain objects, newest first; persisted across sessions.
    property var history: []
    readonly property int maxHistory: 100
    readonly property string historyPath: Quickshell.statePath("notifications.json")

    function setDnd(on) {
        Settings.data.dnd = on;
    }

    function dismissPopup(n) {
        root.popups = root.popups.filter(p => p !== n);
    }

    function clearHistory() {
        root.history = [];
        save();
    }

    function remove(id) {
        root.history = root.history.filter(h => h.id !== id);
        save();
    }

    function save() {
        historyFile.setText(JSON.stringify(root.history));
    }

    NotificationServer {
        id: server
        keepOnReload: false
        actionsSupported: true
        bodyMarkupSupported: true
        bodyHyperlinksSupported: true
        imageSupported: true
        persistenceSupported: true

        onNotification: n => {
            n.tracked = true;
            const entry = {
                id: Date.now() + "-" + n.id,
                appName: n.appName,
                appIcon: n.appIcon,
                summary: n.summary,
                body: n.body,
                urgency: n.urgency,
                time: Date.now()
            };
            root.history = [entry].concat(root.history).slice(0, root.maxHistory);
            root.save();
            if (!root.dnd || n.urgency === NotificationUrgency.Critical) {
                root.popups = root.popups.concat([n]);
            }
            n.closed.connect(() => root.dismissPopup(n));
        }
    }

    FileView {
        id: historyFile
        path: root.historyPath
        blockLoading: true
        onLoaded: {
            try {
                root.history = JSON.parse(text()) || [];
            } catch (e) {
                root.history = [];
            }
        }
    }
}
