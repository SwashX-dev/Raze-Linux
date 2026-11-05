# Known issues and limitations (1.0.0)

- **Default PID 1 is systemd.** r6 runs as PID 1 in Raze Core and in the optional,
  experimental r6 boot entry (console/server use; no graphical session there).
- **Rift is not included.** The default compositor is niri.
- **Rix delegates** evaluation, builds, substitution and profile changes to Nix.
- **Installation needs a network connection** (small derivations are built and a
  few build tools are downloaded even for the default desktop).
- **niri needs a 3D-capable GPU.** Without one, `raze-session` starts the
  software-rendered sway session. In that session `xdg-desktop-portal` does not
  start (it is bound to systemd's graphical-session target), so portal-based file
  pickers and screen sharing are unavailable.
- **Untested in 1.0:** BIOS/legacy installs, manual partitioning, dual boot,
  desktops other than the default, real hardware, the `performance`, `legacy` and
  `minimal` kernel profiles on installed systems, the r6 boot entry.
- **Live boot is slower than it could be**: the ISO uses xz to stay under GitHub's
  2 GiB asset limit (12.5 s to the desktop in a VM without 3D, vs 6.6 s with zstd).
  Installed systems are unaffected.
- The installer adds a UEFI boot entry; to boot the ISO again on the same machine
  choose it in the firmware boot menu.
- **ISO size** is dominated by Mesa/LLVM, firmware and the Qt stack needed by the
  installer.
- `rix search` builds its index on first use; this evaluates nixpkgs once and
  needs about 2 GB of RAM and a minute or two.
- CJK fonts are not installed by default (`fonts.packages = [ pkgs.noto-fonts-cjk-sans ];`).
