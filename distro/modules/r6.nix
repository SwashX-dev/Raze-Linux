# r6 integration.
#
# `raze.r6.services` declares r6 services in Nix; they are rendered to TOML under
# /etc/r6/services and validated at build time by `r6-activate` (unknown deps and
# cycles fail the system build, not the boot).
#
# `raze.r6.bootEntry.enable` adds a second boot entry, "r6", in which NixOS stage 2
# execs r6-init as PID 1 instead of systemd. That mode runs a console/server system
# (udev, dbus, NetworkManager, nix-daemon, gettys, optional sshd) entirely under r6.
# Graphical sessions still require the default (systemd) entry: they depend on
# logind for seat and device access.
{ config, lib, pkgs, ... }:
let
  cfg = config.raze.r6;
  toml = pkgs.formats.toml { };

  serviceModule = { name, ... }: {
    options = {
      description = lib.mkOption { type = lib.types.str; default = ""; };
      command = lib.mkOption { type = lib.types.listOf lib.types.str; };
      type = lib.mkOption { type = lib.types.enum [ "longrun" "oneshot" ]; default = "longrun"; };
      depends = lib.mkOption { type = lib.types.listOf lib.types.str; default = [ ]; };
      wants = lib.mkOption { type = lib.types.listOf lib.types.str; default = [ ]; };
      ready = lib.mkOption { type = lib.types.enum [ "immediate" "notify" ]; default = "immediate"; };
      restart = lib.mkOption { type = lib.types.nullOr (lib.types.enum [ "always" "on-failure" "never" ]); default = null; };
      user = lib.mkOption { type = lib.types.nullOr lib.types.str; default = null; };
      environment = lib.mkOption { type = lib.types.attrsOf lib.types.str; default = { }; };
      log = lib.mkOption { type = lib.types.enum [ "file" "console" "null" ]; default = "file"; };
      enable = lib.mkOption { type = lib.types.bool; default = true; description = "Start at boot."; };
      extraConfig = lib.mkOption { type = toml.type; default = { }; };
    };
  };

  render = name: s: toml.generate "${name}.toml" (lib.filterAttrs (_: v: v != null && v != [ ] && v != { }) {
    inherit (s) description command type depends wants ready restart user environment log enable;
  } // s.extraConfig);

  serviceDir = pkgs.runCommand "r6-services" { nativeBuildInputs = [ pkgs.r6 ]; } ''
    mkdir -p $out
    ${lib.concatStrings (lib.mapAttrsToList (n: s: "cp ${render n s} $out/${n}.toml\n") cfg.services)}
    # Fail the build on an invalid graph.
    r6-activate --dir $out
  '';

  sd = config.systemd.package;
  modulesDir = "/run/booted-system/kernel-modules/lib/modules";
  sh = script: [ "${pkgs.runtimeShell}" "-c" script ];
  getty = tty: extra: {
    description = "Login prompt on ${tty}";
    command = [ "${pkgs.util-linux}/bin/agetty" "--login-program" "${pkgs.shadow}/bin/login" "--noclear" ] ++ extra ++ [ tty ];
    depends = [ "udev-settle" ];
    restart = "always";
    log = "null";
  };
in
{
  options.raze.r6 = {
    services = lib.mkOption {
      type = lib.types.attrsOf (lib.types.submodule serviceModule);
      default = { };
      description = "r6 service definitions rendered to /etc/r6/services.";
    };
    bootEntry.enable = lib.mkEnableOption "an additional boot entry that runs r6 as PID 1";
  };

  config = lib.mkMerge [
    {
      environment.etc."r6/services".source = serviceDir;
      environment.etc."r6/r6.toml".source = toml.generate "r6.toml" {
        log_dir = "/var/log/r6";
        log_max_bytes = 1048576;
        log_keep = 3;
        shutdown_timeout_ms = 15000;
      };
    }

    (lib.mkIf cfg.bootEntry.enable {
      raze.r6.services = {
        modules = {
          description = "Load kernel modules listed in /etc/modules-load.d";
          type = "oneshot";
          environment.MODULE_DIR = modulesDir;
          command = sh ''
            for f in /etc/modules-load.d/*.conf; do
              [ -e "$f" ] || continue
              grep -v '^\s*#' "$f" | while read -r m; do [ -n "$m" ] && ${pkgs.kmod}/bin/modprobe "$m"; done
            done
            exit 0
          '';
        };
        sysctl = {
          description = "Apply kernel parameters";
          type = "oneshot";
          command = sh "${pkgs.procps}/bin/sysctl -q --system || true";
          depends = [ "modules" ];
        };
        udevd = {
          description = "Device manager";
          command = [ "${sd}/lib/systemd/systemd-udevd" ];
          environment.MODULE_DIR = modulesDir;
          restart = "always";
        };
        udev-settle = {
          description = "Coldplug devices and wait for udev";
          type = "oneshot";
          depends = [ "udevd" ];
          command = sh ''
            ${sd}/bin/udevadm trigger --action=add --type=subsystems
            ${sd}/bin/udevadm trigger --action=add --type=devices
            ${sd}/bin/udevadm settle --timeout=30 || true
          '';
        };
        filesystems = {
          description = "Mount remaining filesystems and swap from /etc/fstab";
          type = "oneshot";
          depends = [ "udev-settle" ];
          command = sh ''
            ${pkgs.util-linux}/bin/mount -a -O no_netdev || true
            ${pkgs.util-linux}/bin/swapon -a || true
            ${pkgs.iproute2}/bin/ip link set lo up
            mkdir -p /var/log/r6 /var/lib/r6
          '';
        };
        dbus = {
          description = "D-Bus system message bus";
          depends = [ "filesystems" ];
          ready = "notify";
          command = sh ''
            mkdir -p /run/dbus
            exec ${pkgs.dbus}/bin/dbus-daemon --system --nofork --nopidfile --nosyslog --print-address=3
          '';
          restart = "always";
        };
        networkmanager = {
          description = "Network connections";
          depends = [ "dbus" "udev-settle" ];
          command = [ "${config.networking.networkmanager.package}/bin/NetworkManager" "--no-daemon" ];
          restart = "always";
        };
        nix-daemon = {
          description = "Nix build and store daemon";
          depends = [ "filesystems" ];
          command = [ "${config.nix.package}/bin/nix-daemon" ];
          environment = {
            NIX_SSL_CERT_FILE = "/etc/ssl/certs/ca-certificates.crt";
            PATH = lib.makeBinPath [ config.nix.package pkgs.coreutils pkgs.gnutar pkgs.xz pkgs.gzip pkgs.git ];
          };
          restart = "always";
        };
        getty-tty1 = getty "tty1" [ ];
        getty-tty2 = getty "tty2" [ ];
        getty-ttyS0 = getty "ttyS0" [ "-L" "115200" ];
      } // lib.optionalAttrs config.services.openssh.enable {
        sshd = {
          description = "OpenSSH server";
          depends = [ "filesystems" "networkmanager" ];
          command = sh ''
            ${config.programs.ssh.package}/bin/ssh-keygen -A
            exec ${config.programs.ssh.package}/bin/sshd -D -e -f /etc/ssh/sshd_config
          '';
          restart = "always";
        };
      };

      specialisation.r6.configuration = {
        system.nixos.tags = [ "r6" ];
        # NixOS stage 2 execs boot.systemdExecutable at the end of activation. It is
        # only used by the scripted stage 1, so this entry uses that initrd.
        boot.initrd.systemd.enable = lib.mkForce false;
        boot.systemdExecutable = lib.mkForce "${pkgs.r6}/bin/r6-init";
      };
    })
  ];
}
