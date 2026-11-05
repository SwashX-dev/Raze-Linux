# Boots a Raze Desktop system in a VM and checks the integrated stack: sudo-rs,
# r6/rix availability, branding, greetd autologin + waybar, NetworkManager, zram,
# dbus-broker. The test VM has no 3D acceleration, so raze-session must pick the
# software-rendered fallback session (sway + pixman) instead of niri.
{ pkgs, self, lib }:
pkgs.testers.runNixOSTest {
  name = "raze-desktop";
  node.specialArgs = { razeVersion = "1.0.0"; razeSelf = self; };
  nodes.machine = { ... }: {
    imports = [ self.nixosModules.raze ];
    raze.profile = "desktop";
    raze.desktop = "niri";
    raze.autologinUser = "raze";
    users.users.raze = {
      isNormalUser = true;
      extraGroups = [ "wheel" "networkmanager" "video" ];
      password = "raze";
    };
    virtualisation.memorySize = 2048;
    virtualisation.cores = 2;
    virtualisation.qemu.options = [ "-vga none" "-device virtio-vga" ];
  };
  testScript = ''
    machine.wait_for_unit("multi-user.target")

    with subtest("sudo is sudo-rs and classic sudo is absent"):
        out = machine.succeed("sudo --version")
        assert "sudo-rs" in out, out
        machine.fail("ls /run/current-system/sw/bin/ | grep -x visudo.orig")
        machine.succeed("su - raze -c 'echo raze | sudo -S -k true'")
        machine.fail("su - raze -c 'echo wrong | sudo -S -k true'")

    with subtest("Raze components and branding"):
        machine.succeed("r6 --version | grep 1.0.0")
        machine.succeed("rix --version | grep 1.0.0")
        machine.succeed("grep -qE '^RAZE_VERSION=\"?1.0.0' /etc/os-release")
        machine.succeed("grep -q 'NAME=\"Raze Linux\"' /etc/os-release")
        machine.succeed("r6-activate --dir /etc/r6/services")

    with subtest("performance defaults"):
        machine.succeed("systemctl is-active dbus-broker.service")
        machine.wait_until_succeeds("grep -q zram /proc/swaps")

    with subtest("networking"):
        machine.wait_for_unit("NetworkManager.service")
        machine.succeed("nmcli -t general status")

    with subtest("desktop session"):
        machine.wait_for_unit("greetd.service")
        machine.fail("raze-gpu-check")
        machine.wait_until_succeeds("pgrep -u raze -x sway", timeout=120)
        machine.fail("pgrep -u raze -x niri")
        machine.wait_until_succeeds("pgrep -u raze waybar", timeout=60)
        machine.wait_until_succeeds("pgrep -u raze swaybg", timeout=60)
        machine.sleep(5)
        machine.screenshot("raze-desktop")
        machine.succeed("su - raze -c 'XDG_RUNTIME_DIR=/run/user/$(id -u raze) WAYLAND_DISPLAY=wayland-1 grim /tmp/shot.png'")
        machine.copy_from_machine("/tmp/shot.png", "")

    with subtest("rix native hashing and store verification match Nix"):
        p = machine.succeed("readlink -f /run/current-system/sw/bin/r6").strip().split("/bin/")[0]
        native = machine.succeed(f"rix hash path {p}").strip()
        reference = machine.succeed(f"nix hash path --base32 {p}").strip()
        assert native == reference, (native, reference)
        machine.succeed(f"rix verify {p} | grep -q '^ok'")
  '';
}
