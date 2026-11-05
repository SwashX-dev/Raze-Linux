import QtQuick
import Quickshell

// Night light: asks Rift to apply a colour-temperature gamma ramp on every output.
Scope {
    function apply() {
        Quickshell.execDetached(["rift", "msg", "night-light", Settings.data.nightLight ? String(Settings.data.nightLightTemp) : "off"]);
    }
    Connections {
        target: Settings.data
        function onNightLightChanged() { apply(); }
        function onNightLightTempChanged() { apply(); }
    }
    Component.onCompleted: if (Settings.data.nightLight) apply()
}
