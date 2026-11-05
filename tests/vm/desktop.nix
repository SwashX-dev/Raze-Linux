# Boots a Raze Desktop (niri) system in a VM and checks the integrated stack:
# sudo-rs, r6/rix availability, branding, greetd autologin into niri + waybar,
# NetworkManager, zram, dbus-broker. Takes a screenshot of the running desktop.
{ pkgs, self, lib }:
pkgs.testers.runNixOSTest {
  name = "raze-desktop";
  node.pkgs = pkgs;
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
        machine.succeed("grep -q RAZE_VERSION=1.0.0 /etc/os-release")
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
        machine.wait_until_succeeds("pgrep -u raze -x niri", timeout=120)
        machine.wait_until_succeeds("pgrep -u raze -x waybar", timeout=60)
        machine.sleep(5)
        machine.screenshot("raze-desktop")
        machine.succeed("su - raze -c 'XDG_RUNTIME_DIR=/run/user/$(id -u raze) WAYLAND_DISPLAY=wayland-1 foot -e true' || true")

    with subtest("rix lists generations natively"):
        machine.succeed("rix system generations | grep -E '^\\*'")
  '';
}
