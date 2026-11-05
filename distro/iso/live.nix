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
in
{
  imports = [ "${modulesPath}/installer/cd-dvd/installation-cd-base.nix" ];

  raze.profile = "desktop";
  # The live session is the default install: r6 as PID 1, Rift + Rift Shell.
  raze.init = "r6";
  raze.desktop = "rift";
  raze.autologinUser = "nixos";

  isoImage = {
    volumeID = "RAZE_${lib.replaceStrings [ "." ] [ "_" ] razeVersion}";
    appendToMenuLabel = " Live";
    # xz + x86 BCJ filter: zstd -15 produced 2.40 GB, over GitHub's 2 GiB release
    # asset limit. xz decompresses more slowly, which only affects the live session.
    squashfsCompression = "xz -Xdict-size 100% -Xbcj x86";
    # The live system already contains the default desktop's packages, which the
    # installer copies to the target. A separate reference system was dropped: its
    # unique part was mostly an initrd that installs rebuild for their own hardware.
    storeContents = [ razeSelf.inputs.nixpkgs.outPath ];
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

    Raze Linux ${razeVersion} live system (r6 init, Rift desktop). The graphical session starts automatically.
    The installer: `calamares-raze-launch`. Console login: user "nixos", no password.
  '';

  environment.etc."raze-live".text = "${razeVersion}\n";
  environment.etc."raze/source".source = pkgs.raze-source;

  environment.systemPackages = [
    launcher
    # Replaces Calamares' own entry (plain `pkexec calamares`) with the Raze launcher.
    (lib.hiPrio (pkgs.makeDesktopItem {
      name = "calamares";
      desktopName = "Install Raze Linux";
      comment = "Install Raze Linux on this computer";
      exec = "calamares-raze-launch";
      icon = "raze-logo";
      categories = [ "System" ];
    }))
    calamares-raze
    pkgs.raze-calamares-extensions
  ];
  # Calamares / kpmcore
  security.polkit.enable = true;
  programs.partition-manager.enable = true;

  # VM guest helpers for the most common test environments, as r6 services (they
  # idle when the VM does not provide the device).
  raze.r6.services = {
    # Register the ISO's store in the (tmpfs) Nix database, as NixOS's systemd unit
    # does. nixos-install copies the system from this store; without registration it
    # would try to download or build everything.
    nix-load-db = {
      description = "Register the live store in the Nix database";
      type = "oneshot";
      command = [ "${pkgs.runtimeShell}" "-c" ''
        ${lib.getExe' config.nix.package.out "nix-store"} --load-db < /nix/store/nix-path-registration
        touch /etc/NIXOS
        ${lib.getExe' config.nix.package.out "nix-env"} -p /nix/var/nix/profiles/system --set /run/current-system
      '' ];
    };
    nix-daemon.depends = [ "nix-load-db" ];
    qemu-ga = {
      description = "QEMU guest agent";
      depends = [ "udev-settle" ];
      restart = "always";
      command = [ "${pkgs.runtimeShell}" "-c" ''
        dev=/dev/virtio-ports/org.qemu.guest_agent.0
        [ -e $dev ] || exec ${pkgs.coreutils}/bin/sleep infinity
        exec ${pkgs.qemu_kvm.ga}/bin/qemu-ga -m virtio-serial -p $dev
      '' ];
    };
    spice-vdagentd = {
      description = "SPICE guest agent daemon";
      depends = [ "udev-settle" ];
      restart = "always";
      command = [ "${pkgs.runtimeShell}" "-c" ''
        [ -e /dev/virtio-ports/com.redhat.spice.0 ] || exec ${pkgs.coreutils}/bin/sleep infinity
        mkdir -p /run/spice-vdagentd
        exec ${pkgs.spice-vdagent}/bin/spice-vdagentd -x
      '' ];
    };
  };

  networking.hostName = lib.mkForce "raze-live";
  # Raze installs from its own pinned flake (/etc/raze/source), not a channel.
  system.installer.channel.enable = false;
  # ZFS does not build against Linux 7.3 yet, and Raze does not offer it.
  boot.supportedFilesystems.zfs = lib.mkForce false;
  # installation-cd-base enables wpa_supplicant; Raze uses NetworkManager.
  networking.wireless.enable = lib.mkForce false;
}
