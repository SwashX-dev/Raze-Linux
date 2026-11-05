# r6 as the system init and service manager.
#
# `raze.init = "r6"` (the default since 1.1). Boot sequence:
#   1. initramfs: NixOS's systemd-based stage 1 mounts the root filesystem and runs
#      NixOS activation (`prepare-root`: users, /etc, wrappers). This nixpkgs no longer
#      has a scripted stage 1, so systemd runs here - inside the initramfs only.
#   2. switch-root hands PID 1 to `r6-init` on the real root instead of systemd (the
#      initrd-switch-root override below). No systemd process survives this step.
#   3. Everything afterwards is an r6 service defined below.
#
# Replacements for systemd components:
#   systemd (PID 1, service manager) -> r6-init + r6-supervise
#   systemd-tmpfiles                 -> r6-tmpfiles
#   systemd user instance            -> r6-user
#   systemd-udevd                    -> eudev
#   systemd-logind                   -> elogind (seats, sessions, XDG_RUNTIME_DIR, suspend)
#   dbus-broker (needs systemd)      -> dbus-daemon
#   systemd-timesyncd                -> ntpd-rs
#   systemd-journald                 -> per-service logs (r6-log)
#   systemd-boot                     -> GRUB (see installer)
#   systemd-zram-setup               -> zram oneshot below
#
# `raze.init = "systemd"` keeps stock NixOS behaviour (systemd PID 1).
#
# `raze.r6.services` / `raze.r6.userServices` declare r6 services in Nix; they are
# rendered to /etc/r6/services and /etc/r6/user-services and validated at build time.
{ config, lib, pkgs, ... }:
let
  cfg = config.raze.r6;
  useR6 = config.raze.init == "r6";
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
      group = lib.mkOption { type = lib.types.nullOr lib.types.str; default = null; };
      environment = lib.mkOption { type = lib.types.attrsOf lib.types.str; default = { }; };
      log = lib.mkOption { type = lib.types.enum [ "file" "console" "null" ]; default = "file"; };
      enable = lib.mkOption { type = lib.types.bool; default = true; description = "Start at boot (or at session start for user services)."; };
      extraConfig = lib.mkOption { type = toml.type; default = { }; };
    };
  };

  render = name: s: toml.generate "${name}.toml" (lib.filterAttrs (_: v: v != null && v != [ ] && v != { }) {
    inherit (s) description command type depends wants ready restart user group environment log enable;
  } // s.extraConfig);

  serviceDir = dirName: services: pkgs.runCommand dirName { nativeBuildInputs = [ pkgs.r6 ]; } ''
    mkdir -p $out
    ${lib.concatStrings (lib.mapAttrsToList (n: s: "cp ${render n s} $out/${n}.toml\n") services)}
    # Fail the system build on an invalid service graph.
    r6-activate --dir $out
  '';

  sh = script: [ "${pkgs.runtimeShell}" "-c" script ];
  udev = pkgs.eudev;
  modulesDir = "/run/booted-system/kernel-modules/lib/modules";

  # Reuse NixOS's own start script of a script-based systemd unit (oneshot logic such
  # as setting up setuid wrappers), so r6 runs exactly what NixOS intends.
  # The command of a NixOS unit's ExecStart, as an argv list. Handles systemd's prefixes
  # ("@path argv0 args": run path with argv[0] replaced; "-", "+", "!", ":").
  unitExec = name:
    let sc = config.systemd.services.${name}.serviceConfig;
        exec = if builtins.isList sc.ExecStart then lib.last sc.ExecStart else sc.ExecStart;
        words = lib.filter (w: w != "") (lib.splitString " " exec);
        first = lib.head words;
        stripped = lib.removePrefix "@" (lib.removePrefix "-" (lib.removePrefix "+" (lib.removePrefix "!" (lib.removePrefix ":" first))));
    in [ stripped ] ++ lib.drop (if lib.hasPrefix "@" (lib.removePrefix "-" first) then 2 else 1) words;
  # The unit's environment (PATH from `path`, plus `environment`).
  unitEnv = name: lib.filterAttrs (_: v: v != null) (lib.mapAttrs (_: toString) config.systemd.services.${name}.environment);

  # r6 has no timers: a longrun that runs `command` at most once per `interval`
  # seconds (persisted across reboots), first 10 minutes after boot.
  periodic = name: interval: command: {
    description = "Periodic: ${name} (every ${toString interval} s)";
    restart = "always";
    command = sh ''
      stamp=/var/lib/r6/periodic/${name}
      mkdir -p /var/lib/r6/periodic
      sleep 600
      while :; do
        now=$(date +%s); last=$(cat $stamp 2>/dev/null || echo 0)
        if [ $((now - last)) -ge ${toString interval} ]; then
          ${lib.escapeShellArgs command} && echo "$now" > $stamp
        fi
        sleep 3600
      done
    '';
  };

  # NixOS units that r6 replaces or that do not apply to it. Any other enabled unit
  # without an r6 service triggers a build warning (see below).
  coveredUnits = [
    "NetworkManager" "NetworkManager-dispatcher" "NetworkManager-wait-online" "wpa_supplicant"
    "firewall" "nftables" "post-boot" "network-local-commands" "nscd" "ntpd-rs" "resolvconf"
    "greetd" "sshd" "suid-sgid-wrappers" "systemd-modules-load" "systemd-sysctl" "scx"
    "polkit" "rtkit-daemon" "upower" "udisks2" "power-profiles-daemon" "bluetooth"
    "dbus" "nix-daemon" "fstrim" "logrotate" "backdoor" "qemu-guest-agent" "spice-vdagentd"
    # Not applicable under r6 (systemd-internal, kexec, one-time migrations, pstore).
    "generate-shutdown-ramfs" "prepare-kexec" "lastlog2-import" "linger-users"
    "logrotate-checkconf" "reload-systemd-vconsole-setup" "sleep-actions" "systemd-pstore"
    "nix-load-db"
  ];

  getty = tty: {
    description = "Login prompt on ${tty}";
    command = [ "${pkgs.util-linux}/bin/agetty" "--login-program" "${pkgs.shadow}/bin/login" "--noclear" tty "linux" ];
    wants = [ "udev-settle" ];
    restart = "always";
    log = "null";
  };

  # sysvinit-style shutdown commands (also used by elogind) -> r6.
  r6Compat = pkgs.runCommand "r6-sysvinit-compat" { } ''
    mkdir -p $out/bin
    for c in poweroff reboot halt shutdown; do ln -s ${pkgs.r6}/bin/r6 $out/bin/$c; done
  '';
in
{
  options.raze = {
    init = lib.mkOption {
      type = lib.types.enum [ "r6" "systemd" ];
      default = "r6";
      description = "PID 1 and service manager: r6 (default) or systemd (stock NixOS).";
    };
    r6 = {
      services = lib.mkOption {
        type = lib.types.attrsOf (lib.types.submodule serviceModule);
        default = { };
        description = "System r6 services, rendered to /etc/r6/services.";
      };
      userServices = lib.mkOption {
        type = lib.types.attrsOf (lib.types.submodule serviceModule);
        default = { };
        description = "Per-user r6 services (run by r6-user in each session), rendered to /etc/r6/user-services.";
      };
      # Raze 1.0 had r6 as an extra boot entry; 1.1 runs it as PID 1 (raze.init).
      # Kept so 1.0 configurations still evaluate when upgrading.
      bootEntry.enable = lib.mkOption {
        type = lib.types.bool;
        default = false;
        visible = false;
        description = "Removed in Raze 1.1 (no effect): r6 is the default PID 1, see raze.init.";
      };
    };
  };

  config = lib.mkMerge [
    {
      warnings = lib.optional cfg.bootEntry.enable
        "raze.r6.bootEntry.enable has no effect since Raze 1.1: r6 is PID 1 by default (raze.init = \"${config.raze.init}\"). Remove the line from configuration.nix.";
    }

    # NixOS test driver support: its root shell on /dev/hvc0 is a systemd unit in
    # stock NixOS; under r6 it is an r6 service (only in test VMs).
    (lib.mkIf (useR6 && (config.testing.backdoor or false)) {
      raze.r6.services.backdoor = {
        description = "NixOS test driver backdoor (test VMs only)";
        restart = "always";
        log = "null";
        extraConfig.stop_signal = "HUP";
        command = [ "${pkgs.bash}/bin/bash" "-c" ''
          export USER=root HOME=/root DISPLAY=:0.0 PAGER=
          source /etc/profile || true
          cd /tmp
          exec < /dev/hvc0 > /dev/hvc0
          while ! exec 2> /dev/ttyS0; do sleep 0.1; done
          echo "connecting to host..." >&2
          stty -F /dev/hvc0 raw -echo
          echo "Spawning backdoor root shell..."
          PS1="" exec ${pkgs.bashNonInteractive}/bin/bash --norc /dev/hvc0
        '' ];
      };
    })

    (lib.mkIf useR6 {
      # Only r6 systems get (and validate) an r6 service set; with raze.init =
      # "systemd" the r6 services other modules declare are inert.
      environment.etc."r6/services".source = serviceDir "r6-services" cfg.services;
      environment.etc."r6/user-services".source = serviceDir "r6-user-services" cfg.userServices;
      environment.etc."r6/r6.toml".source = toml.generate "r6.toml" {
        log_dir = "/var/log/r6";
        log_max_bytes = 1048576;
        log_keep = 3;
        shutdown_timeout_ms = 15000;
      };

      # NixOS's switch-to-configuration drives systemd over D-Bus; r6 systems get their
      # own (boot loader + activation script + r6 reload/restart of changed services),
      # so `nixos-rebuild switch` and `rix system switch` work under r6.
      system.switch.enable = false;
      system.activatableSystemBuilderCommands = ''
        mkdir -p $out/bin
        substitute ${./r6/switch-to-configuration.sh} $out/bin/switch-to-configuration \
          --subst-var-by shell ${pkgs.runtimeShell} \
          --subst-var-by path ${lib.makeBinPath [ pkgs.r6 pkgs.coreutils pkgs.gnugrep pkgs.gawk pkgs.diffutils pkgs.gnused ]} \
          --subst-var-by toplevel ''${!toplevelVar} \
          --subst-var-by preSwitchCheck ${config.system.preSwitchChecksScript} \
          --subst-var-by installBootLoader ${config.system.build.installBootLoader}
        chmod +x $out/bin/switch-to-configuration
      '';

      # Hand PID 1 to r6-init at switch-root. The path refers to the real root's store;
      # r6 is part of the system closure, so the initrd need not carry it.
      boot.initrd.systemd.services.initrd-switch-root.serviceConfig.ExecStart = lib.mkForce [
        ""
        "systemctl --no-block switch-root /sysroot ${builtins.unsafeDiscardStringContext "${pkgs.r6}/bin/r6-init"}"
      ];
      assertions = [{
        assertion = !config.system.nixos-init.enable;
        message = "raze.init = \"r6\" is not compatible with system.nixos-init (it replaces initrd-switch-root).";
      }];
      services.dbus.implementation = lib.mkForce "dbus";
      services.ntpd-rs.enable = lib.mkDefault true;
      services.timesyncd.enable = lib.mkForce false;
      systemd.oomd.enable = lib.mkForce false;

      environment.systemPackages = [ (lib.hiPrio r6Compat) pkgs.elogind ];

      # NetworkManager drives wpa_supplicant over D-Bus; its bus policy is only added by
      # NixOS's own wireless module, so register it here.

      # Static users for services that use systemd's DynamicUser.
      users.users.ntpd-rs = lib.mkIf config.services.ntpd-rs.enable { isSystemUser = true; group = "ntpd-rs"; };
      users.groups.ntpd-rs = lib.mkIf config.services.ntpd-rs.enable { };

      # elogind registers login sessions (seat, XDG_RUNTIME_DIR) for console and
      # greeter logins. pam_systemd stays in the stack but is a no-op when systemd is
      # not PID 1.
      security.pam.services = lib.genAttrs
        ([ "login" ] ++ lib.optional config.services.greetd.enable "greetd"
          ++ lib.optional config.services.openssh.enable "sshd")
        (svc: {
        rules.session.elogind = {
          control = "optional";
          modulePath = "${pkgs.elogind}/lib/security/pam_elogind.so";
          order = (config.security.pam.services.${svc}.rules.session.systemd.order or 12000) + 10;
        };
      });
      services.dbus.packages = [ pkgs.elogind ]
        ++ lib.optional config.networking.networkmanager.enable pkgs.wpa_supplicant;
      services.udev.packages = [ pkgs.elogind ];
      environment.etc."elogind/logind.conf".text = ''
        [Login]
        KillUserProcesses=no
        HandlePowerKey=poweroff
        HandleLidSwitch=suspend
      '';
      environment.etc."elogind/sleep.conf".text = ''
        [Sleep]
        AllowSuspend=yes
        AllowHibernation=no
      '';

      # Members of wheel may manage power and devices without a password prompt from
      # the graphical session.
      security.polkit.extraConfig = ''
        polkit.addRule(function(action, subject) {
          if (subject.isInGroup("wheel") &&
              (action.id.indexOf("org.freedesktop.login1.") == 0 ||
               action.id.indexOf("org.freedesktop.udisks2.") == 0 ||
               action.id.indexOf("org.freedesktop.UPower.") == 0 ||
               action.id.indexOf("net.hadess.PowerProfiles.") == 0)) {
            return polkit.Result.YES;
          }
        });
      '';

      warnings =
        let
          wanted = lib.filterAttrs (n: v: v.enable && v.wantedBy != [ ] && !(lib.elem n coveredUnits)
            && !(cfg.services ? ${n})) config.systemd.services;
        in
        lib.optional (wanted != { })
          "raze.init = \"r6\": these enabled NixOS services have no r6 service and will not run: ${lib.concatStringsSep ", " (lib.attrNames wanted)}. Declare them in raze.r6.services (see docs/architecture/r6.md) or use raze.init = \"systemd\".";

      raze.r6.services = {
        syslog = {
          description = "syslog(3) receiver on /dev/log";
          command = [ "${pkgs.r6}/bin/r6-syslogd" ];
          restart = "always";
        };
        machine-id = {
          description = "Create /etc/machine-id on first boot";
          type = "oneshot";
          command = sh ''
            if [ ! -s /etc/machine-id ]; then
              ${pkgs.dbus}/bin/dbus-uuidgen > /etc/machine-id.tmp && mv /etc/machine-id.tmp /etc/machine-id
            fi
            mkdir -p /var/lib/dbus && ln -sfn /etc/machine-id /var/lib/dbus/machine-id
          '';
        };
        tmpfiles = {
          description = "Create volatile files and directories (tmpfiles.d)";
          type = "oneshot";
          depends = [ "machine-id" ];
          command = [ "${pkgs.r6}/bin/r6-tmpfiles" "--create" "--remove" "--boot" ];
        };
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
          depends = [ "modules" ];
          command = sh "${pkgs.procps}/bin/sysctl -q --system || true";
        };
        udevd = {
          description = "Device manager (eudev)";
          wants = [ "tmpfiles" ];
          command = [ "${udev}/bin/udevd" ];
          environment.MODULE_DIR = modulesDir;
          restart = "always";
        };
        udev-settle = {
          description = "Coldplug devices and wait for udev";
          type = "oneshot";
          depends = [ "udevd" ];
          command = sh ''
            ${udev}/bin/udevadm trigger --action=add --type=subsystems
            ${udev}/bin/udevadm trigger --action=add --type=devices
            ${udev}/bin/udevadm settle --timeout=30 || true
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
        wrappers = {
          description = "setuid/setcap wrappers in /run/wrappers (NixOS unit script)";
          type = "oneshot";
          # /run/wrappers is a systemd mount unit on NixOS; mount it here.
          command = sh ''
            mkdir -p /run/wrappers
            ${pkgs.util-linux}/bin/mountpoint -q /run/wrappers || \
              ${pkgs.util-linux}/bin/mount -t tmpfs -o nodev,mode=755,size=50% tmpfs /run/wrappers
            exec ${lib.concatStringsSep " " (unitExec "suid-sgid-wrappers")}
          '';
        };
        nscd = {
          description = "Name service cache (nsncd)";
          wants = [ "tmpfiles" ];
          command = [ "${pkgs.nsncd}/bin/nsncd" ];
          user = "nscd";
          group = "nscd";
          extraConfig.runtime_directory = "nscd";
          restart = "always";
        };
        dbus = {
          description = "D-Bus system message bus";
          depends = [ "machine-id" ];
          wants = [ "filesystems" "tmpfiles" ];
          ready = "notify";
          command = sh ''
            mkdir -p /run/dbus
            exec ${pkgs.dbus}/bin/dbus-daemon --config-file=/etc/dbus-1/system.conf --nofork --nopidfile --nosyslog --print-address=3
          '';
          restart = "always";
        };
        elogind = {
          description = "Login, seat and session manager (elogind)";
          depends = [ "dbus" "udev-settle" ];
          command = [ "${pkgs.elogind}/libexec/elogind" ];
          restart = "always";
        };
        polkit = {
          description = "Authorization manager";
          depends = [ "dbus" ];
          wants = [ "elogind" ];
          command = [ "${config.security.polkit.package.out}/lib/polkit-1/polkitd" "--no-debug" "--log-level=notice" ];
          restart = "always";
        };
        nix-daemon = {
          description = "Nix build and store daemon";
          wants = [ "filesystems" "wrappers" ];
          command = [ "${config.nix.package}/bin/nix-daemon" ];
          environment = {
            CURL_CA_BUNDLE = "/etc/ssl/certs/ca-certificates.crt";
            NIX_SSL_CERT_FILE = "/etc/ssl/certs/ca-certificates.crt";
            PATH = lib.makeBinPath [ config.nix.package pkgs.coreutils pkgs.gnutar pkgs.xz pkgs.gzip pkgs.git pkgs.util-linux ];
          };
          extraConfig.rlimit_nofile = 1048576;
          restart = "always";
        };
        getty-tty2 = getty "tty2";
        getty-tty3 = getty "tty3";
      }
      // lib.optionalAttrs config.networking.networkmanager.enable {
        networkmanager = {
          description = "Network connections";
          depends = [ "dbus" "udev-settle" ];
          # The firewall is loaded before any interface comes up.
          wants = [ "polkit" "wpa_supplicant" "firewall" "nftables" ];
          command = [ "${config.networking.networkmanager.package}/bin/NetworkManager" "--no-daemon" ];
          restart = "always";
        };
        wpa_supplicant = {
          description = "Wi-Fi authentication (D-Bus mode, driven by NetworkManager)";
          depends = [ "dbus" ];
          command = [ "${pkgs.wpa_supplicant}/bin/wpa_supplicant" "-u" "-s" ];
          # Runs as root, as permitted by the upstream D-Bus policy registered below.
          extraConfig.runtime_directory = "wpa_supplicant";
          restart = "always";
        };
        resolvconf = {
          description = "Initialise /etc/resolv.conf (NixOS unit script)";
          type = "oneshot";
          depends = [ "filesystems" ];
          command = unitExec "resolvconf";
        };
      }
      // lib.optionalAttrs config.services.ntpd-rs.enable {
        ntpd-rs = {
          description = "Network time (ntpd-rs)";
          depends = [ "filesystems" ];
          wants = [ "networkmanager" ];
          # The systemd unit uses a DynamicUser; r6 uses a static system user with the
          # same ambient capabilities.
          command = unitExec "ntpd-rs";
          user = "ntpd-rs";
          group = "ntpd-rs";
          extraConfig.capabilities = [ "CAP_SYS_TIME" "CAP_NET_BIND_SERVICE" ];
          restart = "always";
        };
      }
      // lib.optionalAttrs config.zramSwap.enable {
        zram = {
          description = "Compressed swap in RAM";
          type = "oneshot";
          depends = [ "modules" "udev-settle" ];
          command = sh ''
            ${pkgs.kmod}/bin/modprobe zram num_devices=1 || exit 0
            mem=$(${pkgs.gawk}/bin/awk '/MemTotal/ {print $2}' /proc/meminfo)
            size=$(( mem * ${toString config.zramSwap.memoryPercent} / 100 ))K
            ${pkgs.util-linux}/bin/zramctl /dev/zram0 --algorithm ${config.zramSwap.algorithm} --size "$size"
            ${pkgs.util-linux}/bin/mkswap -q /dev/zram0
            ${pkgs.util-linux}/bin/swapon --priority ${toString config.zramSwap.priority} /dev/zram0
          '';
        };
      }
      // lib.optionalAttrs config.security.rtkit.enable {
        rtkit = {
          description = "Realtime scheduling broker";
          depends = [ "dbus" ];
          command = [ "${pkgs.rtkit}/libexec/rtkit-daemon" ];
          restart = "always";
        };
      }
      // lib.optionalAttrs config.services.upower.enable {
        upower = {
          description = "Power and battery information";
          depends = [ "dbus" "udev-settle" ];
          command = [ "${config.services.upower.package}/libexec/upowerd" ];
          restart = "always";
        };
      }
      // lib.optionalAttrs config.hardware.bluetooth.enable {
        bluetooth = {
          description = "Bluetooth";
          depends = [ "dbus" "udev-settle" ];
          command = [ "${config.hardware.bluetooth.package}/libexec/bluetooth/bluetoothd" "-n" ];
          restart = "always";
        };
      }
      // lib.optionalAttrs config.services.power-profiles-daemon.enable {
        power-profiles = {
          description = "Power profiles";
          depends = [ "dbus" ];
          command = [ "${config.services.power-profiles-daemon.package}/libexec/power-profiles-daemon" ];
          restart = "always";
        };
      }
      // lib.optionalAttrs config.services.udisks2.enable {
        udisks2 = {
          description = "Disk management";
          depends = [ "dbus" "udev-settle" ];
          wants = [ "polkit" ];
          command = [ "${config.services.udisks2.package}/libexec/udisks2/udisksd" ];
          restart = "always";
        };
      }
      // lib.optionalAttrs config.services.greetd.enable {
        greetd = {
          description = "Login greeter";
          depends = [ "elogind" "udev-settle" ];
          wants = [ "networkmanager" "polkit" ];
          command = unitExec "greetd";
          restart = "always";
          log = "null";
        };
      }
      // lib.optionalAttrs (config.networking.firewall.enable && !config.networking.nftables.enable) {
        firewall = {
          description = "Firewall (NixOS rules, iptables)";
          type = "oneshot";
          wants = [ "modules" ];
          command = unitExec "firewall";
          environment = unitEnv "firewall";
        };
      }
      // lib.optionalAttrs config.networking.nftables.enable {
        nftables = {
          description = "Firewall (NixOS rules, nftables)";
          type = "oneshot";
          wants = [ "modules" ];
          command = unitExec "nftables";
          environment = unitEnv "nftables";
        };
      }
      // {
        post-boot = {
          description = "powerManagement boot/power-up commands (NixOS unit script)";
          type = "oneshot";
          wants = [ "filesystems" ];
          command = unitExec "post-boot";
          environment = unitEnv "post-boot";
        };
        network-local-commands = {
          description = "networking.localCommands and static DNS (NixOS unit script)";
          type = "oneshot";
          wants = [ "resolvconf" ];
          command = unitExec "network-local-commands";
          environment = unitEnv "network-local-commands";
        };
      }
      // lib.optionalAttrs config.services.scx.enable {
        scx = {
          description = "sched_ext scheduler (${config.services.scx.scheduler})";
          wants = [ "modules" ];
          command = [ "${config.services.scx.package}/bin/${config.services.scx.scheduler}" ] ++ config.services.scx.extraArgs;
          restart = "on-failure";
        };
      }
      // lib.optionalAttrs config.services.fstrim.enable {
        # util-linux ships fstrim.service; this is its ExecStart.
        fstrim = periodic "fstrim" 604800 [ "${pkgs.util-linux}/bin/fstrim" "--listed-in" "/etc/fstab:/proc/self/mountinfo" "--verbose" "--quiet-unsupported" ];
      }
      // lib.optionalAttrs config.services.logrotate.enable {
        logrotate = periodic "logrotate" 3600 (unitExec "logrotate");
      }
      // lib.optionalAttrs config.services.openssh.enable {
        sshd = {
          description = "OpenSSH server";
          depends = [ "filesystems" ];
          wants = [ "networkmanager" ];
          command = sh ''
            ${config.programs.ssh.package}/bin/ssh-keygen -A
            exec ${config.programs.ssh.package}/bin/sshd -D -e -f /etc/ssh/sshd_config
          '';
          restart = "always";
        };
      };

      # Per-user session services (r6-user).
      raze.r6.userServices = {
        dbus = {
          description = "D-Bus session bus";
          ready = "notify";
          command = sh ''exec ${pkgs.dbus}/bin/dbus-daemon --session --address="unix:path=$XDG_RUNTIME_DIR/bus" --nofork --nopidfile --nosyslog --print-address=3'';
          restart = "always";
        };
      } // lib.optionalAttrs config.services.pipewire.enable {
        pipewire = {
          description = "PipeWire media server";
          depends = [ "dbus" ];
          command = [ "${config.services.pipewire.package}/bin/pipewire" ];
          restart = "always";
        };
        pipewire-pulse = {
          description = "PulseAudio compatibility";
          depends = [ "pipewire" ];
          command = [ "${config.services.pipewire.package}/bin/pipewire-pulse" ];
          restart = "always";
        };
        wireplumber = {
          description = "PipeWire session manager";
          depends = [ "pipewire" ];
          command = [ "${config.services.pipewire.wireplumber.package}/bin/wireplumber" ];
          restart = "always";
        };
      };
    })
  ];
}
