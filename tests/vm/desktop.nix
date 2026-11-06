# Boots the default Raze Desktop (r6 as PID 1, Rift + Rift Shell) in a VM without 3D
# acceleration and checks the integrated stack. The NixOS test driver's backdoor runs
# as an r6 service here (distro/modules/r6.nix), so systemd-specific driver helpers
# such as wait_for_unit are not used.
{ pkgs, self, lib, init ? "r6" }:
let
  version = self.packages.x86_64-linux.r6.version;
  asUser = cmd: "su - raze -c 'export XDG_RUNTIME_DIR=/run/user/1000 WAYLAND_DISPLAY=wayland-1 DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus RIFT_SOCKET=$(ls /run/user/1000/rift.*.sock); ${cmd}'";
  # Ask polkit for an admin action, allowing the session's agent to authenticate.
  polkitCheck = pkgs.writeShellScript "polkit-check" ''
    ${pkgs.polkit.bin}/bin/pkcheck --action-id org.freedesktop.policykit.exec --process $$ --allow-user-interaction \
      && touch /tmp/pk-ok
  '';
  # Screen cast client: portal session, then the PipeWire connection the portal grants,
  # as browsers do; the command given gets that fd and the node id.
  portalClient = pkgs.writeShellScript "portal-screencast" ''
    export GI_TYPELIB_PATH=${pkgs.glib.out}/lib/girepository-1.0:${pkgs.gobject-introspection}/lib/girepository-1.0
    exec ${pkgs.python3.withPackages (p: [ p.pygobject3 ])}/bin/python3 ${./scripts/portal-screencast.py} "$@"
  '';
  # Read three frames from the stream (declared as a video capture, so WirePlumber
  # links it to the screen cast source).
  gstFrames = pkgs.writeShellScript "gst-frames" ''
    export GST_PLUGIN_SYSTEM_PATH_1_0=${pkgs.pipewire}/lib/gstreamer-1.0:${pkgs.gst_all_1.gstreamer.out}/lib/gstreamer-1.0:${pkgs.gst_all_1.gst-plugins-base}/lib/gstreamer-1.0
    exec ${pkgs.gst_all_1.gstreamer.bin}/bin/gst-launch-1.0 pipewiresrc fd="$1" path="$2" num-buffers=3 \
      'stream-properties=props,media.type=Video,media.category=Capture,media.role=Screen' ! fakesink 2>&1
  '';
in
pkgs.testers.runNixOSTest {
  name = "raze-desktop-${init}";
  node.specialArgs = { razeVersion = version; razeSelf = self; };
  nodes.machine = { ... }: {
    imports = [ self.nixosModules.raze ];
    raze.profile = "desktop";
    raze.desktop = "rift";
    raze.init = init;
    # The installer's default kernel profile (sched_ext scheduler, full preemption).
    raze.kernel.profile = if init == "r6" then "performance" else "generic";
    raze.autologinUser = "raze";
    users.users.raze = {
      isNormalUser = true;
      extraGroups = [ "wheel" "networkmanager" "video" ];
      password = "raze";
    };
    virtualisation.memorySize = 3072;
    virtualisation.cores = 2;
    virtualisation.qemu.options = [ "-vga none" "-device virtio-vga" ];
  };
  testScript = ''
    r6 = "${init}" == "r6"
    machine.start()

    with subtest("PID 1 and services"):
        if r6:
            machine.succeed("test \"$(cat /proc/1/comm)\" = r6-init")
            machine.fail("pgrep -x systemd")
            machine.fail("pgrep -f systemd-journald")
            machine.succeed("test ! -e /run/systemd/system")
            machine.wait_until_succeeds("r6 status greetd | grep -q up", timeout=120)
            # Every enabled system service is up (longrun) or done (oneshot).
            machine.wait_until_succeeds("! r6 list | awk '$3 == \"enabled\" && $2 != \"up\" && $2 != \"done\" && $2 != \"listening\"' | grep .", timeout=120)
            machine.succeed("r6-activate --dir /etc/r6/services")
            # Regular users can read status but not control services.
            machine.succeed("su - raze -c 'r6 status greetd | grep -q up'")
            machine.fail("su - raze -c 'r6 stop greetd'")
            # NixOS firewall rules and the performance profile's sched_ext scheduler,
            # both NixOS systemd units ported to r6 services.
            machine.succeed("iptables -S | grep -q nixos-fw")
            machine.wait_until_succeeds("grep -qx enabled /sys/kernel/sched_ext/state", timeout=60)
            machine.succeed("grep -q preempt=full /proc/cmdline")
            # D-Bus activation (dbus-daemon + setuid launch helper) starts services
            # on first use, as systemd would.
            machine.succeed("dbus-send --system --print-reply --dest=org.freedesktop.UPower /org/freedesktop/UPower org.freedesktop.DBus.Properties.Get string:org.freedesktop.UPower string:DaemonVersion")
            machine.succeed("pgrep -x upowerd")
            machine.succeed("dbus-send --system --print-reply --dest=org.freedesktop.UDisks2 /org/freedesktop/UDisks2/Manager org.freedesktop.DBus.Properties.Get string:org.freedesktop.UDisks2.Manager string:Version")
            # Socket activation: nix-daemon waits for its first client (as on systemd).
            machine.succeed("r6 status nix-daemon | grep -q listening")
            machine.succeed("su - raze -c 'nix store info'")
            machine.succeed("r6 status nix-daemon | grep -q up")
            # Units imported from the NixOS unit tree: polkit's per-connection agent
            # helper (Accept=yes socket), timers (fstrim, logrotate).
            machine.succeed("r6 status polkit-agent-helper | grep -q listening")
            machine.succeed("grep -q '^timer fstrim' /etc/r6/import-report")
            machine.succeed("pgrep -x r6-timers")
            machine.succeed("logger -t raze-test hello-syslog")
            machine.wait_until_succeeds("r6 logs syslog | grep -q hello-syslog")
        else:
            machine.wait_for_unit("multi-user.target")
            machine.wait_for_unit("greetd.service")

    with subtest("sudo is sudo-rs"):
        out = machine.succeed("sudo --version")
        assert "sudo-rs" in out, out
        machine.succeed("su - raze -c 'echo raze | sudo -S -k true'")
        machine.fail("su - raze -c 'echo wrong | sudo -S -k true'")

    with subtest("release and branding"):
        machine.succeed("grep -q 'PRETTY_NAME=\"Raze Linux ${version}\"' /etc/os-release")
        machine.succeed("r6 --version | grep -q ${version}")
        machine.succeed("rix --version | grep -q ${version}")
        out = machine.succeed("fastfetch --pipe true --logo none -s os:initsystem")
        assert "Raze Linux ${version}" in out, out
        if r6:
            assert "r6-init" in out, out

    with subtest("networking and time"):
        machine.wait_until_succeeds("nmcli -t -f STATE general | grep -q connected", timeout=60)
        if r6:
            machine.succeed("pgrep -u ntpd-rs -x ntp-daemon")

    with subtest("Rift and Rift Shell"):
        machine.wait_until_succeeds("pgrep -u raze -x rift", timeout=120)
        machine.wait_until_succeeds("ls /run/user/1000/rift.*.sock", timeout=60)
        out = machine.succeed("${asUser "rift msg version"}")
        # No 3D acceleration in the test VM: Rift must pick its pixman renderer.
        assert "drm-pixman" in out, out
        machine.wait_until_succeeds("pgrep -u raze -f 'quickshell -p'", timeout=120)
        machine.wait_until_succeeds("pgrep -u raze -f 'wl-paste --watch .*cliphist store'", timeout=60)
        machine.wait_until_succeeds("${asUser "rift-shell ipc notifications count"}", timeout=60)

    with subtest("windows, screencopy and recording"):
        machine.succeed("${asUser "rift msg action spawn foot"}")
        machine.wait_until_succeeds("${asUser "rift msg windows"} | grep -q foot", timeout=60)
        machine.succeed("${asUser "grim /tmp/shot.png"}")
        machine.succeed("test $(stat -c %s /tmp/shot.png) -gt 10000")
        machine.copy_from_machine("/tmp/shot.png", "")
        machine.succeed("${asUser "timeout -s INT 3 wf-recorder -f /tmp/rec.mp4 || true"}")
        machine.succeed("test -s /tmp/rec.mp4")
        machine.screenshot("rift-desktop")

    with subtest("screen sharing through the XDG portal"):
        # As a browser does: portal ScreenCast session -> xdg-desktop-portal-wlr ->
        # Rift's screencopy -> PipeWire stream, read through the portal's connection.
        status, out = machine.execute("${asUser "timeout 90 ${portalClient} ${gstFrames}"}")
        print(out)
        assert status == 0 and "Got EOS" in out, f"no frames from the screen cast (exit {status})"
        machine.succeed("pgrep -u raze -f xdg-desktop-portal-wlr")

    with subtest("notifications reach Rift Shell"):
        machine.succeed("${asUser "notify-send raze-test hello"}")
        machine.wait_until_succeeds("${asUser "rift-shell ipc notifications count"} | grep -qx '[1-9][0-9]*'", timeout=30)

    with subtest("polkit authentication through Rift Shell"):
        # An admin action asked for from the session (as apps do over D-Bus) ->
        # polkitd -> Rift Shell's agent dialog -> password -> polkit's agent helper
        # (an r6 per-connection socket service) -> authorized.
        machine.succeed("${asUser "rift msg action spawn ${polkitCheck}"}")
        machine.sleep(4)
        machine.screenshot("polkit-dialog")
        machine.send_chars("raze\n")
        machine.wait_until_succeeds("test -e /tmp/pk-ok", timeout=30)

    with subtest("session lock"):
        machine.succeed("${asUser "rift-shell lock"}")
        machine.wait_until_succeeds("${asUser "rift msg state"} | grep -q '\"locked\":true'", timeout=30)

    if r6:
        with subtest("r6 switch-to-configuration"):
            out = machine.succeed("/run/current-system/bin/switch-to-configuration dry-activate")
            assert "would activate" in out, out

    with subtest("rix native hashing matches Nix"):
        p = machine.succeed("readlink -f /run/current-system/sw/bin/r6").strip().split("/bin/")[0]
        native = machine.succeed(f"rix hash path {p}").strip()
        reference = machine.succeed(f"nix hash path --base32 {p}").strip()
        assert native == reference, (native, reference)

    machine.shutdown()
  '';
}
