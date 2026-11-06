{ config, lib, pkgs, razeVersion, ... }:
let
  cfg = config.raze;
in
{
  options.raze = {
    version = lib.mkOption {
      type = lib.types.str;
      default = razeVersion;
      readOnly = true;
      description = "Raze Linux release version.";
    };
  };

  config = {
    system.nixos = {
      distroName = "Raze Linux";
      distroId = "raze";
      vendorName = "Raze Linux";
      vendorId = "raze";
      extraOSReleaseArgs = {
        RAZE_VERSION = razeVersion;
        # Report the Raze release (fastfetch, hostnamectl, ...); NixOS's own release
        # number stays available as NIXOS_RELEASE.
        PRETTY_NAME = "Raze Linux ${razeVersion}";
        VERSION = razeVersion;
        VERSION_ID = razeVersion;
        NIXOS_RELEASE = config.system.nixos.release;
        HOME_URL = "https://github.com/SwashX-dev/Raze-Linux";
        DOCUMENTATION_URL = "https://github.com/SwashX-dev/Raze-Linux/tree/main/docs";
        SUPPORT_URL = "https://github.com/SwashX-dev/Raze-Linux/issues";
        BUG_REPORT_URL = "https://github.com/SwashX-dev/Raze-Linux/issues";
        ANSI_COLOR = "1;31";
      };
    };

    # GRUB menu background (the NixOS default shows the NixOS logo).
    boot.loader.grub.splashImage = lib.mkOverride 900 "${pkgs.raze-branding}/share/backgrounds/raze/wallpaper-1080.png";

    environment.etc."raze-release".text = ''
      RAZE_VERSION=${razeVersion}
      RAZE_PROFILE=${cfg.profile}
      RAZE_DESKTOP=${cfg.desktop}
      RAZE_KERNEL_PROFILE=${cfg.kernel.profile}
      NIXPKGS_REVISION=${lib.trivial.revisionWithDefault "unknown"}
    '';

    # Nix: flakes on, and the system's own pinned nixpkgs is the default package
    # source, so `rix install` resolves against the same revision the system was
    # built from (reproducible, and mostly served from the local store).
    nix = {
      settings = {
        experimental-features = [ "nix-command" "flakes" ];
        auto-optimise-store = true;
        trusted-users = [ "root" "@wheel" ];
      };
      registry.nixpkgs.to = { type = "path"; path = pkgs.path; };
      nixPath = [ "nixpkgs=${pkgs.path}" ];
      channel.enable = false;
    };

    # fastfetch shows the Raze logo; `neofetch` (unmaintained upstream) runs fastfetch.
    environment.etc."xdg/fastfetch/config.jsonc".source = "${pkgs.raze-branding}/share/raze/fastfetch/config.jsonc";

    environment.systemPackages = with pkgs; [
      (writeShellScriptBin "neofetch" ''exec ${lib.getExe fastfetch} "$@"'')
      r6
      rix
      raze-branding
      fastfetch
      git
      htop
      curl
      pciutils
      usbutils
      file
      nano
    ];

    # The NixOS manual is large and slow to build; Raze ships its own docs online.
    documentation.nixos.enable = false;
    documentation.doc.enable = false;
    documentation.info.enable = false;

    networking.networkmanager.enable = lib.mkDefault true;
    networking.hostName = lib.mkOverride 1100 "raze";

    users.mutableUsers = lib.mkDefault true;

    console = {
      earlySetup = true;
      colors = [
        "0f0f10" "dc2626" "22c55e" "eab308" "3b82f6" "a855f7" "06b6d4" "d4d4d8"
        "52525b" "ef4444" "4ade80" "facc15" "60a5fa" "c084fc" "22d3ee" "fafafa"
      ];
    };

    environment.etc."issue".text = lib.mkForce ''

      \e[1;31m  ██████   █████  ███████ ███████\e[0m
      \e[1;31m  ██   ██ ██   ██    ███  ██\e[0m
      \e[1;31m  ██████  ███████   ███   █████\e[0m
      \e[1;31m  ██   ██ ██   ██  ███    ██\e[0m
      \e[1;31m  ██   ██ ██   ██ ███████ ███████\e[0m  Linux ${razeVersion}

      Rust-Powered. Performance-Driven. Minimal by Design.
      Kernel \r on \m (\l)

    '';

    system.stateVersion = lib.mkOverride 1100 "26.05";
  };
}
