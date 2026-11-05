import QtQuick
import Quickshell
import Quickshell.Wayland

// Idle management (ext-idle-notify): lock, then turn the screens off.
// Applications holding an idle inhibitor (video players) and "Stay awake" pause it.
Scope {
    IdleMonitor {
        enabled: !Shell.caffeine && Settings.data.idleLockMinutes > 0
        respectInhibitors: true
        timeout: Settings.data.idleLockMinutes * 60
        onIsIdleChanged: if (isIdle) Shell.lock()
    }
    IdleMonitor {
        enabled: !Shell.caffeine && Settings.data.idleScreenOffMinutes > 0
        respectInhibitors: true
        timeout: Settings.data.idleScreenOffMinutes * 60
        onIsIdleChanged: if (isIdle) Rift.action("power-off-monitors")
    }
}
