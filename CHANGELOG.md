# Changelog

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
