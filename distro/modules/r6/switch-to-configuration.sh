#!@shell@
# switch-to-configuration for r6 systems (replaces NixOS's systemd-specific one).
#   switch        install the boot loader, activate now, restart changed services
#   boot          install the boot loader only (applies at next boot)
#   test          activate now without touching the boot loader
#   dry-activate  show which r6 services would be started, restarted or stopped
set -eu
PATH=@path@:$PATH
toplevel=@toplevel@
action=${1:-}
# Restarting these would end the running session; their changes apply at next boot.
keep="dbus elogind greetd udevd udev-settle filesystems wrappers"

case "$action" in
    switch | boot | test | dry-activate) ;;
    *)
        echo "usage: switch-to-configuration switch|boot|test|dry-activate" >&2
        exit 1
        ;;
esac

old=$(readlink -f /run/current-system 2>/dev/null || true)
new_dir=$toplevel/etc/r6/services
old_dir=$old/etc/r6/services

# Print "start|restart|stop|keep NAME" for every service whose definition changed.
plan() {
    for f in "$new_dir"/*.toml; do
        [ -e "$f" ] || continue
        n=$(basename "$f" .toml)
        o=$old_dir/$n.toml
        if [ ! -e "$o" ]; then
            grep -q '^enable = false' "$f" || echo "start $n"
        elif ! cmp -s "$f" "$o"; then
            case " $keep " in
                *" $n "*) echo "keep $n" ;;
                *) echo "restart $n" ;;
            esac
        fi
    done
    for o in "$old_dir"/*.toml; do
        [ -e "$o" ] || continue
        n=$(basename "$o" .toml)
        [ -e "$new_dir/$n.toml" ] || echo "stop $n"
    done
}

if [ "$action" = dry-activate ]; then
    echo "would activate $toplevel"
    plan | sed 's/^/would /'
    exit 0
fi

if [ "$(id -u)" != 0 ]; then
    echo "switch-to-configuration: must be run as root" >&2
    exit 1
fi

if [ "$action" = switch ] || [ "$action" = boot ]; then
    @preSwitchCheck@ "$toplevel" "$action"
    @installBootLoader@ "$toplevel"
fi
[ "$action" = boot ] && exit 0

if [ "$(cat /proc/1/comm 2>/dev/null)" != r6-init ]; then
    echo "switch-to-configuration: r6 is not PID 1; the new configuration applies at next boot" >&2
    exit 0
fi

steps=$(plan)
"$toplevel/activate"
r6 reload
status=0
while read -r what name; do
    [ -n "$what" ] || continue
    state=$(r6 list | awk -v n="$name" '$1 == n { print $2 }')
    case "$what" in
        start)
            echo "starting $name"
            r6 start --no-block "$name" || status=1
            ;;
        restart)
            if [ "$state" = up ] || [ "$state" = starting ]; then
                echo "restarting $name"
                r6 restart --no-block "$name" || status=1
            fi
            ;;
        stop)
            # `r6 reload` retired it; the supervisor stops retired services itself.
            echo "stopping $name (removed)"
            ;;
        keep)
            echo "$name changed; the new definition applies at next boot"
            ;;
    esac
done <<STEPS
$steps
STEPS
echo "Switched to $toplevel. User services change at next login."
exit $status
