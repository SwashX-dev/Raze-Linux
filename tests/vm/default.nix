# Flake checks. Run all with `nix flake check`, one with
# `nix build .#checks.x86_64-linux.<name>`. VM checks need KVM.
{ pkgs, self, lib }:
{
  # Unit + integration tests of the Rust components run inside their builds.
  r6 = pkgs.r6;
  rix = pkgs.rix;

  # Boot the Raze Core image with r6 as PID 1 and run its self-test.
  core-boot = pkgs.callPackage ./core-boot.nix { image = self.packages.x86_64-linux.core-image; };

  # Rift + Rift Shell desktop with r6 as PID 1 (the default), and with the systemd
  # fallback (raze.init = "systemd").
  desktop = import ./desktop.nix { inherit pkgs self lib; };
  desktop-systemd = import ./desktop.nix { inherit pkgs self lib; init = "systemd"; };
}
