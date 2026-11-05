# Reference installed system: the default Raze Desktop (niri) on a single ext4 disk
# with systemd-boot. Its closure is embedded in the ISO for offline installation and
# it is the system booted by the VM tests.
{ lib, pkgs, modulesPath, ... }:
{
  imports = [ "${modulesPath}/profiles/qemu-guest.nix" ];

  raze.profile = "desktop";
  raze.desktop = "niri";

  boot.loader.systemd-boot.enable = true;
  boot.loader.efi.canTouchEfiVariables = true;
  fileSystems."/" = { device = "/dev/disk/by-label/raze"; fsType = "ext4"; };
  fileSystems."/boot" = { device = "/dev/disk/by-label/RAZE_EFI"; fsType = "vfat"; };

  # `nix build .#nixosConfigurations.raze-desktop.config.system.build.vm` for quick
  # boot testing; the serial console is on ttyS0.
  virtualisation.vmVariant = {
    virtualisation.memorySize = 3072;
    virtualisation.cores = 4;
    boot.kernelParams = [ "console=tty0" "console=ttyS0,115200" ];
    # Test instrumentation: a QEMU guest agent under r6 (attach a virtio-serial port
    # named org.qemu.guest_agent.0) so tests can run commands in the guest.
    raze.r6.services.qemu-ga = {
      description = "QEMU guest agent (test VMs only)";
      command = [ "${pkgs.qemu_kvm.ga}/bin/qemu-ga" "-m" "virtio-serial" "-p" "/dev/virtio-ports/org.qemu.guest_agent.0" ];
      depends = [ "udev-settle" ];
      restart = "always";
    };
  };

  users.users.raze = {
    isNormalUser = true;
    extraGroups = [ "wheel" "networkmanager" "video" "audio" ];
    initialPassword = "raze";
  };
}
