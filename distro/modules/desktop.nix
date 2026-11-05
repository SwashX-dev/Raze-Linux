# Desktop selection. Exactly one desktop is installed; only the services it needs are
# enabled. The default is Rift, Raze's own compositor, with Rift Shell (panel, launcher,
# notifications, lock screen, polkit agent). The other tiling Wayland sessions (niri,
# Hyprland, dwl) share the waybar/fuzzel/mako/swaylock session tooling.
{ config, lib, pkgs, ... }:
let
  cfg = config.raze;
  d = cfg.desktop;
  graphical = d != "none";
  waylandTiling = builtins.elem d [ "rift" "niri" "hyprland" "dwl" ];
  # Sessions using the waybar/fuzzel/mako stack (Rift brings its own shell).
  barSession = builtins.elem d [ "niri" "hyprland" "dwl" ];
  greetdSession = {
    rift = "raze-session";
    # raze-session runs niri on hardware-accelerated GPUs and falls back to a
    # software-rendered sway session elsewhere (niri refuses software renderers).
    niri = "raze-session";
    hyprland = "Hyprland";
    dwl = "dwl";
  }.${d} or null;
in
{
  options.raze = {
    desktop = lib.mkOption {
      type = lib.types.enum [ "rift" "niri" "hyprland" "dwl" "xfce" "lxqt" "plasma" "gnome" "dwm" "none" ];
      default = "rift";
      description = ''
        The desktop session to install. "rift" (Rift compositor + Rift Shell) is the
        default.
      '';
    };
    autologinUser = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      description = "Log this user straight into the desktop session (live ISO).";
    };
  };

  config = lib.mkMerge [
    (lib.mkIf graphical {
      hardware.graphics.enable = true;
      services.pipewire = {
        enable = true;
        alsa.enable = true;
        pulse.enable = true;
        wireplumber.enable = true;
      };
      security.rtkit.enable = true;
      security.polkit.enable = true;
      services.udisks2.enable = true;
      services.gvfs.enable = true;
      # gvfs without the SMB backend: samba alone is ~90 MiB.
      services.gvfs.package = pkgs.gvfs.override { samba = null; };
      # Speech synthesis pulls in ~700 MiB of voices; opt-in for those who need it.
      services.speechd.enable = lib.mkOverride 900 false;
      services.upower.enable = true;
      fonts = {
        # Explicit, small font set (the NixOS default set adds ~150 MiB incl. CJK).
        # CJK users: add noto-fonts-cjk-sans to fonts.packages.
        enableDefaultPackages = lib.mkForce false;
        packages = with pkgs; [ inter jetbrains-mono noto-fonts noto-fonts-color-emoji dejavu_fonts ];
        fontconfig.defaultFonts = {
          sansSerif = [ "Inter" "Noto Sans" ];
          monospace = [ "JetBrains Mono" ];
          emoji = [ "Noto Color Emoji" ];
        };
      };
      environment.systemPackages = with pkgs; [ xdg-utils networkmanagerapplet ];
    })

    (lib.mkIf waylandTiling {
      environment.systemPackages = with pkgs; [
        foot
        grim
        slurp
        wl-clipboard
        brightnessctl
        wiremix
        pcmanfm
        xwayland-satellite
        adwaita-icon-theme
      ];
      programs.dconf.enable = true;
      xdg.portal = {
        enable = true;
        extraPortals = [ pkgs.xdg-desktop-portal-gtk ];
        config.common.default = [ "gtk" ];
      };
      environment.sessionVariables.NIXOS_OZONE_WL = "1";

      environment.etc."xdg/foot/foot.ini".source = "${pkgs.raze-branding}/share/raze/desktop/foot.ini";

      # Graphical session services (r6-user). Started by raze-session-ready once the
      # compositor's Wayland socket exists; components add themselves as dependencies.
      raze.r6.userServices.graphical-session = {
        description = "Graphical session services";
        type = "oneshot";
        command = [ "${pkgs.coreutils}/bin/true" ];
        enable = false;
      };

      # Lightweight TTY greeter; no X server, no heavy display manager.
      services.greetd = {
        enable = true;
        settings = {
          default_session.command =
            "${pkgs.tuigreet}/bin/tuigreet --time --remember --asterisks --greeting 'Raze Linux ${cfg.version}' --cmd ${greetdSession}";
        } // lib.optionalAttrs (cfg.autologinUser != null) {
          initial_session = {
            command = greetdSession;
            user = cfg.autologinUser;
          };
        };
      };
    })

    (lib.mkIf barSession {
      environment.systemPackages = with pkgs; [ fuzzel waybar mako swaybg swaylock swayidle lxqt.lxqt-policykit ];
      security.pam.services.swaylock = { };
      # Raze session defaults (users can override in ~/.config).
      environment.etc = {
        "xdg/waybar/config".source = "${pkgs.raze-branding}/share/raze/desktop/waybar/config.jsonc";
        "xdg/waybar/style.css".source = "${pkgs.raze-branding}/share/raze/desktop/waybar/style.css";
        "xdg/fuzzel/fuzzel.ini".source = "${pkgs.raze-branding}/share/raze/desktop/fuzzel.ini";
        "xdg/mako/config".source = "${pkgs.raze-branding}/share/raze/desktop/mako.conf";
      };
    })

    (lib.mkIf (d == "rift") {
      environment.systemPackages = with pkgs; [ rift rift-shell cliphist libnotify wl-screenrec playerctl ];
      services.displayManager.sessionPackages = [ pkgs.rift ];
      # The shell's icon glyphs (a subset of Material Symbols Rounded).
      fonts.packages = [ pkgs.rift-shell ];
      environment.etc."rift/config.toml".source = "${pkgs.rift}/share/rift/default-config.toml";
      # Lock screen authentication (Rift Shell's PamContext uses this service).
      security.pam.services.rift-shell = { };
      # NetworkManager and Bluetooth control from the shell; power profiles.
      services.power-profiles-daemon.enable = lib.mkDefault true;

      raze.r6.userServices = {
        graphical-session.wants = [ "rift-shell" "cliphist" ];
        rift-shell = {
          description = "Rift Shell (panel, notifications, lock screen, polkit agent)";
          depends = [ "dbus" ];
          command = [ "${pkgs.rift-shell}/bin/rift-shell" ];
          restart = "always";
          enable = false;
        };
        cliphist = {
          description = "Clipboard history";
          command = [ "${pkgs.wl-clipboard}/bin/wl-paste" "--watch" "${pkgs.cliphist}/bin/cliphist" "store" ];
          restart = "always";
          enable = false;
        };
      };
    })

    (lib.mkIf (d == "niri") {
      programs.niri.enable = true;
      environment.etc."niri/config.kdl".source = "${pkgs.raze-branding}/share/raze/desktop/niri/config.kdl";
      # Software-rendering fallback session (see raze-session).
      environment.systemPackages = with pkgs; [ sway raze-gpu-check ];
      environment.etc."sway/raze.conf".source = "${pkgs.raze-branding}/share/raze/desktop/sway/raze.conf";
    })

    (lib.mkIf (d == "hyprland") {
      programs.hyprland.enable = true;
    })

    (lib.mkIf (d == "dwl") {
      programs.dwl.enable = true;
    })

    (lib.mkIf (d == "xfce") {
      services.xserver.enable = true;
      services.xserver.desktopManager.xfce.enable = true;
      services.xserver.displayManager.lightdm.enable = true;
    })

    (lib.mkIf (d == "lxqt") {
      services.xserver.enable = true;
      services.xserver.desktopManager.lxqt.enable = true;
      services.xserver.displayManager.lightdm.enable = true;
    })

    (lib.mkIf (d == "dwm") {
      services.xserver.enable = true;
      services.xserver.windowManager.dwm.enable = true;
      services.xserver.displayManager.lightdm.enable = true;
      environment.systemPackages = with pkgs; [ st dmenu ];
    })

    (lib.mkIf (d == "plasma") {
      services.desktopManager.plasma6.enable = true;
      services.displayManager.sddm = {
        enable = true;
        wayland.enable = true;
      };
    })

    (lib.mkIf (d == "gnome") {
      services.desktopManager.gnome.enable = true;
      services.displayManager.gdm.enable = true;
    })

    (lib.mkIf (graphical && !waylandTiling && cfg.autologinUser != null) {
      services.displayManager.autoLogin = {
        enable = true;
        user = cfg.autologinUser;
      };
    })
  ];
}
