# Stand-in for the hardware-configuration.nix nixos-generate-config writes.
{
  fileSystems."/" = { device = "/dev/disk/by-label/raze"; fsType = "ext4"; };
  fileSystems."/boot/efi" = { device = "/dev/disk/by-label/EFI"; fsType = "vfat"; };
  boot.initrd.availableKernelModules = [ "virtio_pci" "virtio_blk" "ahci" "nvme" ];
}
