# Changelog

## 1.1.0 — 2026-11-06

Release notes: [docs/release-notes/v1.1.0.md](docs/release-notes/v1.1.0.md).

### Added
- **r6 as PID 1 and service manager everywhere** (live ISO, installed systems,
  `r6-user` for desktop sessions); socket activation, sd_notify readiness, timers,
  `r6-import` of the systemd units NixOS generates, r6 switch-to-configuration.
- **Rift**, Raze's Smithay-based scrollable-tiling Wayland compositor, and **Rift
  Shell** (panel, launcher, control centre, notifications, lock screen, screenshots,
  recording, clipboard history, polkit agent); touchscreen, touchpad swipes, screen
  sharing.
- Desktop choice at install: Rift, niri, Hyprland, dwl, KDE Plasma, Xfce, LXQt, dwm
  on r6; GNOME on systemd.
- Installer: dual boot (install alongside, replace a partition, reuse an existing EFI
  partition, other systems in the GRUB menu); `installer-configs` flake check.
- fastfetch with Raze support; os-release reports the Raze version.

### Changed
- Default desktop Rift + Rift Shell (was niri + waybar); default PID 1 r6 (was
  systemd); EFI installs use GRUB (was systemd-boot); GRUB uses the Raze background.

### Fixed
- r6-init flushes `/etc/passwd`/`/etc/shadow` written by activation at startup.
- Installer: invalid EFI GRUB option in the generated configuration.

## 1.0.0 — 2026-11-05

First release.

### Added
- **r6** init and supervision suite (Rust): PID 1, single-process supervisor with
  dependency-ordered parallel activation, readiness notification, restart
  policies with backoff and rate limiting, rotated logs, persistent
  enable/disable, reload, stale-instance recovery, CLI, build-time graph validation.
- **Raze Core** image: Linux 7.3.1 + 3.4 MB initramfs booting r6 as PID 1;
  r6-vs-s6 benchmark harness.
- **Rix** (Rust) with native NAR hashing, store verification, search index,
  generations/rollback, manifest and flake.lock readers; delegates evaluation,
  builds and installs to Nix.
- Raze NixOS modules: profiles (minimal, desktop, custom, development), desktop
  selection, kernel profiles (generic, performance, legacy, minimal), sudo-rs,
  r6 service definitions and an experimental r6 boot entry.
- Raze desktop session on niri with waybar, fuzzel, foot, mako; automatic
  software-rendering fallback (sway + pixman) without a 3D-capable GPU.
- Calamares installer with Raze branding and a flake-generating install module.
- VM tests (Raze Core boot, desktop integration), QEMU installer test helpers,
  GitHub Actions workflows.
