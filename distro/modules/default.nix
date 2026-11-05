# The Raze Linux NixOS module set. Import this (flake: `raze.nixosModules.raze`) and
# select a profile, desktop and kernel profile through the `raze.*` options.
{
  imports = [
    ./base.nix
    ./sudo-rs.nix
    ./kernel.nix
    ./performance.nix
    ./profiles.nix
    ./desktop.nix
    ./r6.nix
  ];
}
