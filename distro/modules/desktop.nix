# Desktop selection. Exactly one desktop is installed; only the services it needs are
# enabled. Scrollable-tiling Wayland sessions (rift, niri) share the Raze session
# tooling: waybar, fuzzel, foot, mako, swaybg, swaylock.
{ config, lib, pkgs, ... }:
let
  cfg = config.raze;
  d = cfg.desktop;
  graphical = d != "none";
  waylandTiling = builtins.elem d [ "rift" "niri" "hyprland" "dwl" ];
  greetdSession = {
    rift = "rift-session";
    niri = "niri-session";
    hyprland = "Hyprland";
    dwl = "dwl";
  }.${d} or null;
in
{
  options.raze = {
    desktop = lib.mkOption {
      type = lib.types.enum [ "rift" "niri" "hyprland" "dwl" "xfce" "lxqt" "plasma" "gnome" "dwm" "none" ];
      default = "niri";
      description = ''
        The desktop session to install. "niri" is the default until Rift meets the
        release criteria (see docs/architecture/rift.md); "rift" installs the
        experimental Rift compositor.
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
      services.upower.enable = true;
      fonts = {
        packages = with pkgs; [ inter jetbrains-mono noto-fonts noto-fonts-color-emoji ];
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
        fuzzel
        waybar
        mako
        swaybg
        swaylock
        swayidle
        grim
        slurp
        wl-clipboard
        brightnessctl
        pavucontrol
        pcmanfm
        lxqt.lxqt-policykit
        xwayland-satellite
        adwaita-icon-theme
      ];
      programs.dconf.enable = true;
      security.pam.services.swaylock = { };
      xdg.portal = {
        enable = true;
        extraPortals = [ pkgs.xdg-desktop-portal-gtk ];
        config.common.default = [ "gtk" ];
      };
      environment.sessionVariables.NIXOS_OZONE_WL = "1";

      # Raze session defaults (users can override in ~/.config).
      environment.etc = {
        "xdg/waybar/config".source = "${pkgs.raze-branding}/share/raze/desktop/waybar/config.jsonc";
        "xdg/waybar/style.css".source = "${pkgs.raze-branding}/share/raze/desktop/waybar/style.css";
        "xdg/fuzzel/fuzzel.ini".source = "${pkgs.raze-branding}/share/raze/desktop/fuzzel.ini";
        "xdg/foot/foot.ini".source = "${pkgs.raze-branding}/share/raze/desktop/foot.ini";
        "xdg/mako/config".source = "${pkgs.raze-branding}/share/raze/desktop/mako.conf";
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

    (lib.mkIf (d == "niri") {
      programs.niri.enable = true;
      environment.etc."niri/config.kdl".source = "${pkgs.raze-branding}/share/raze/desktop/niri/config.kdl";
    })

    (lib.mkIf (d == "rift") {
      environment.systemPackages = [ pkgs.rift ];
      services.displayManager.sessionPackages = [ pkgs.rift ];
      environment.etc."rift/config.toml".source = "${pkgs.rift}/share/rift/config.toml";
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
