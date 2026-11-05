# Reference installed system: the default Raze Desktop (niri) on a single ext4 disk
# with systemd-boot. Its closure is embedded in the ISO for offline installation and
# it is the system booted by the VM tests.
{ lib, modulesPath, ... }:
{
  imports = [ "${modulesPath}/profiles/qemu-guest.nix" ];

  raze.profile = "desktop";
  raze.desktop = "niri";

  boot.loader.systemd-boot.enable = true;
  boot.loader.efi.canTouchEfiVariables = true;
  fileSystems."/" = { device = "/dev/disk/by-label/raze"; fsType = "ext4"; };
  fileSystems."/boot" = { device = "/dev/disk/by-label/RAZE_EFI"; fsType = "vfat"; };

  users.users.raze = {
    isNormalUser = true;
    extraGroups = [ "wheel" "networkmanager" "video" "audio" ];
    initialPassword = "raze";
  };
}
