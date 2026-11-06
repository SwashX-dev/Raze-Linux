# Boots each selectable desktop as installed by the Calamares installer (greetd
# autologin, r6 as PID 1; GNOME with systemd, which it requires) in a VM without 3D
# acceleration, and checks that the session, its compositor or X server and the r6
# user services come up.
{ pkgs, self, lib, desktop }:
let
  version = self.packages.x86_64-linux.r6.version;
  r6 = desktop != "gnome";
  # Processes that show the desktop is running (all must run as the user).
  procs = {
    niri = [ "sway" ];          # no 3D in the VM: raze-session's software session
    hyprland = [ "sway" "waybar" ]; # likewise; Hyprland itself is tested with virgl
    dwl = [ "dwl" "waybar" ];
    plasma = [ "kwin_wayland" "plasmashell" ];
    xfce = [ "xfwm4" "xfce4-panel" ];
    lxqt = [ "lxqt-panel" "openbox" ];
    dwm = [ "dwm" ];
    gnome = [ "gnome-shell" ];
  }.${desktop};
  x11 = builtins.elem desktop [ "xfce" "lxqt" "dwm" ];
  py = b: if b then "True" else "False";
in
pkgs.testers.runNixOSTest {
  name = "raze-desktop-${desktop}";
  node.specialArgs = { razeVersion = version; razeSelf = self; };
  nodes.machine = { ... }: {
    imports = [ self.nixosModules.raze ];
    raze.profile = "desktop";
    raze.desktop = desktop;
    raze.autologinUser = "raze";
    users.users.raze = {
      isNormalUser = true;
      extraGroups = [ "wheel" "networkmanager" "video" ];
      password = "raze";
    };
    virtualisation.memorySize = if desktop == "plasma" || desktop == "gnome" then 4096 else 3072;
    virtualisation.cores = 2;
    environment.systemPackages = [ (pkgs.xprop or pkgs.xorg.xprop) ];
    virtualisation.qemu.options = [ "-vga none" "-device virtio-vga" ];
  };
  testScript = ''
    machine.start()
    if ${py r6}:
        machine.wait_until_succeeds("test \"$(cat /proc/1/comm)\" = r6-init", timeout=120)
        machine.fail("pgrep -x systemd")
        machine.wait_until_succeeds("r6 status greetd | grep -q up", timeout=120)
    else:
        machine.wait_for_unit("multi-user.target")

    def diagnostics():
        script = r"""
    set +e
    for f in /home/raze/.local/state/*.log /home/raze/.local/share/xorg/*.log /run/user/1000/r6-user.log /run/user/1000/hypr/*/hyprland.log; do
        [ -f "$f" ] && { echo "== $f"; tail -n 60 "$f"; }
    done
    echo '== processes'; ps -eo user,pid,args | grep -v ' \\[' | grep -v '^root' | tail -n 60
    echo '== logind'; loginctl list-sessions
    echo '== r6-user'; su - raze -c 'XDG_RUNTIME_DIR=/run/user/1000 r6 --user list'
    true
    """
        print(machine.execute(script)[1])

    with subtest("session runs as the user"):
        try:
            for p in ${builtins.toJSON procs}:
                # Nix-wrapped programs run as .NAME-wrapped: match the command line.
                machine.wait_until_succeeds(f"pgrep -u raze -f '^(\\S*/)?\\.?{p}(-wrapped)?( |$)'", timeout=240)
            if "${desktop}" == "plasma":
                # The splash screen exits once Plasma's startup has completed.
                machine.wait_until_fails("pgrep -u raze ksplashqml", timeout=240)
                # plasmashell created its default layout, panel included.
                machine.wait_until_succeeds("grep -q 'plugin=org.kde.panel' /home/raze/.config/plasma-org.kde.plasma.desktop-appletsrc", timeout=120)
        except Exception:
            diagnostics()
            raise
        if ${py x11}:
            # Rootless X server (devices through elogind), not a root display manager.
            machine.wait_until_succeeds("pgrep -u raze -f '/bin/X '", timeout=60)
            machine.fail("pgrep -u root -f '/bin/X '")
            # A window manager owns the X screen.
            machine.succeed("su - raze -c 'DISPLAY=:0 xprop -root _NET_SUPPORTING_WM_CHECK' | grep -q 'window id'")

    if ${py r6}:
        with subtest("r6 services and user services"):
            machine.wait_until_succeeds("! r6 list | awk '$3 == \"enabled\" && $2 != \"up\" && $2 != \"done\" && $2 != \"listening\"' | grep .", timeout=120)
            machine.wait_until_succeeds("pgrep -u raze -x r6-user", timeout=60)
            machine.wait_until_succeeds("pgrep -u raze -x pipewire", timeout=60)
            machine.wait_until_succeeds("pgrep -u raze -x wireplumber", timeout=60)
            # The session was registered with elogind (seat, active).
            machine.succeed("loginctl list-sessions --no-legend | grep -q raze")

    machine.sleep(10)
    machine.screenshot("${desktop}")
  '';
}
