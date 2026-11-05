# Manual installation (from the live ISO terminal)

This is what the graphical installer does, step by step. Example for UEFI and an
empty disk `/dev/vda` — **this erases the disk**.

```bash
sudo -i
parted /dev/vda -- mklabel gpt
parted /dev/vda -- mkpart ESP fat32 1MiB 1GiB
parted /dev/vda -- set 1 esp on
parted /dev/vda -- mkpart raze ext4 1GiB 100%
mkfs.fat -F 32 -n RAZE_EFI /dev/vda1
mkfs.ext4 -L raze /dev/vda2
mount /dev/disk/by-label/raze /mnt
mkdir -p /mnt/boot && mount -o umask=077 /dev/disk/by-label/RAZE_EFI /mnt/boot

nixos-generate-config --root /mnt
cp -rL /etc/raze/source /mnt/etc/nixos/raze && chmod -R u+w /mnt/etc/nixos/raze
cat > /mnt/etc/nixos/flake.nix <<'NIX'
{
  inputs.raze.url = "path:./raze";
  outputs = { self, raze }: {
    nixosConfigurations.raze = raze.lib.mkSystem {
      modules = [ ./configuration.nix ./hardware-configuration.nix ];
    };
  };
}
NIX
cat > /mnt/etc/nixos/configuration.nix <<'NIX'
{ ... }:
{
  raze.profile = "desktop";
  raze.desktop = "rift";
  raze.kernel.profile = "performance";
  boot.loader.grub = { enable = true; efiSupport = true; device = "nodev"; };
  boot.loader.efi.canTouchEfiVariables = true;
  networking.hostName = "raze";
  users.users.me = { isNormalUser = true; extraGroups = [ "wheel" "networkmanager" "video" ]; };
  system.stateVersion = "26.05";
}
NIX
nix flake lock /mnt/etc/nixos       # lock first (see installer notes)
nixos-install --root /mnt --flake /mnt/etc/nixos#raze --no-channel-copy
nixos-enter --root /mnt -c 'passwd me'
reboot
```

## Options reference

| Option | Values |
|---|---|
| `raze.profile` | `minimal`, `desktop`, `custom`, `development` |
| `raze.desktop` | `rift` (default), `niri`, `hyprland`, `dwl`, `xfce`, `lxqt`, `plasma`, `gnome`, `dwm`, `none` |
| `raze.kernel.profile` | `generic`, `performance`, `legacy`, `minimal` |
| `raze.init` | `r6` (default) or `systemd` |
| `raze.r6.services`, `raze.r6.userServices` | extra r6 system / session services (see [r6](../architecture/r6.md)) |
| `raze.r6.services.<name>` | r6 service definitions (validated at build time) |
