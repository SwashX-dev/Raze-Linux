pragma Singleton
import QtQuick
import Quickshell

// UI state shared between panels (which overlay is open, where).
Singleton {
    id: root
    property bool launcherOpen: false
    property bool controlCenterOpen: false
    property bool powerMenuOpen: false
    property bool clipboardOpen: false
    property bool locked: false
    property bool recording: false
    property bool caffeine: false

    // The screen overlays open on: the compositor's focused output.
    readonly property var activeScreen: {
        const name = Rift.focusedOutput;
        return Quickshell.screens.find(s => s.name === name) || Quickshell.screens[0] || null;
    }

    function closeAll() {
        launcherOpen = false;
        controlCenterOpen = false;
        powerMenuOpen = false;
        clipboardOpen = false;
    }

    function toggle(which) {
        const was = root[which];
        closeAll();
        root[which] = !was;
    }

    function lock() {
        closeAll();
        locked = true;
    }

    function run(cmd) {
        Quickshell.execDetached(["sh", "-c", cmd]);
    }
}
