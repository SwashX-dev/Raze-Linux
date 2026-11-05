# Raze Linux live installer ISO.
{ config, lib, pkgs, modulesPath, razeVersion, razeSelf, ... }:
let
  calamares-raze = pkgs.raze-calamares-extensions.calamares;
  launcher = pkgs.writeShellScriptBin "calamares-raze-launch" ''
    # Calamares needs root. sudo-rs resets the environment, so pass only the
    # variables a Wayland/X11 client needs, explicitly.
    exec sudo ${pkgs.coreutils}/bin/env \
      WAYLAND_DISPLAY="''${WAYLAND_DISPLAY:-}" DISPLAY="''${DISPLAY:-}" \
      XDG_RUNTIME_DIR="''${XDG_RUNTIME_DIR:-}" XDG_SESSION_TYPE="''${XDG_SESSION_TYPE:-}" \
      QT_QPA_PLATFORM="wayland;xcb" \
      ${calamares-raze}/bin/calamares "$@"
  '';
  # The reference installed system (default desktop). Shipping its closure in the ISO
  # lets the default installation complete offline.
  reference = razeSelf.nixosConfigurations.raze-desktop.config.system.build.toplevel;
in
{
  imports = [ "${modulesPath}/installer/cd-dvd/installation-cd-base.nix" ];

  raze.profile = "desktop";
  raze.desktop = "niri";
  raze.autologinUser = "nixos";

  isoImage = {
    volumeID = "RAZE_${lib.replaceStrings [ "." ] [ "_" ] razeVersion}";
    appendToMenuLabel = " Live";
    # xz: zstd -15 produced a 2.40 GB image, over GitHub's 2 GiB release asset limit.
    # xz decompresses more slowly, which only affects the live session.
    squashfsCompression = "xz -Xdict-size 100%";
    storeContents = [ reference razeSelf.inputs.nixpkgs.outPath ];
    includeSystemBuildDependencies = false;
    splashImage = "${pkgs.raze-branding}/share/backgrounds/raze/wallpaper-1080.png";
    efiSplashImage = "${pkgs.raze-branding}/share/backgrounds/raze/wallpaper-1080.png";
    grubTheme = null;
  };
  image.baseName = lib.mkForce "raze-linux-v${razeVersion}-x86_64";
  image.fileName = lib.mkForce "raze-linux-v${razeVersion}-x86_64.iso";
  system.nixos.tags = [ "live" ];

  # Live user: passwordless sudo-rs (installation media only).
  security.sudo-rs.wheelNeedsPassword = lib.mkForce false;
  security.sudo-rs.execWheelOnly = lib.mkForce true;
  services.getty.helpLine = lib.mkForce ''

    Raze Linux ${razeVersion} live system. The graphical session starts automatically.
    The installer: `calamares-raze-launch`. Console login: user "nixos", no password.
  '';

  environment.etc."raze-live".text = "${razeVersion}\n";
  environment.etc."raze/source".source = pkgs.raze-source;

  environment.systemPackages = [
    launcher
    calamares-raze
    pkgs.raze-calamares-extensions
    pkgs.glibcLocales
    pkgs.gparted
  ];
  # Calamares / kpmcore
  security.polkit.enable = true;
  programs.partition-manager.enable = true;
  i18n.supportedLocales = [ "all" ];

  # VM guest helpers for the most common test environments.
  services.qemuGuest.enable = true;
  services.spice-vdagentd.enable = true;

  networking.hostName = lib.mkForce "raze-live";
  # Raze installs from its own pinned flake (/etc/raze/source), not a channel.
  system.installer.channel.enable = false;
  # ZFS does not build against Linux 7.3 yet, and Raze does not offer it.
  boot.supportedFilesystems.zfs = lib.mkForce false;
  # installation-cd-base enables wpa_supplicant; Raze uses NetworkManager.
  networking.wireless.enable = lib.mkForce false;
}
