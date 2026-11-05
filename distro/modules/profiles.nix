# Installation profiles: minimal, desktop, custom, development.
{ config, lib, pkgs, ... }:
let
  cfg = config.raze;
in
{
  options.raze.profile = lib.mkOption {
    type = lib.types.enum [ "minimal" "desktop" "custom" "development" ];
    default = "desktop";
    description = ''
      minimal:     console system with essential command-line tools, no graphics.
      desktop:     the default lightweight graphical configuration (raze.desktop).
      custom:      like desktop, with the user's choice of desktop environment.
      development: desktop plus compilers, debuggers and developer tooling.
    '';
  };

  config = lib.mkMerge [
    (lib.mkIf (cfg.profile == "minimal") {
      raze.desktop = lib.mkForce "none";
      environment.systemPackages = with pkgs; [ tmux ];
    })

    (lib.mkIf (cfg.profile == "development") {
      environment.systemPackages = with pkgs; [
        gcc
        gnumake
        cmake
        pkg-config
        gdb
        cargo
        rustc
        rust-analyzer
        clippy
        rustfmt
        python3
        nodejs
        neovim
        ripgrep
        fd
        jq
        direnv
        nix-direnv
      ];
      programs.direnv.enable = true;
      virtualisation.podman.enable = true;
    })
  ];
}
