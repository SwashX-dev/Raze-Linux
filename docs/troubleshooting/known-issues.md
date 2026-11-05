# Known issues and limitations (1.1.0)

## Init system (r6)

- **systemd still runs in the initramfs.** This nixpkgs has no scripted stage 1, so
  NixOS's systemd-based stage 1 mounts the root filesystem and runs activation; PID 1
  is handed to r6 at switch-root and no systemd process survives it.
- **systemd libraries remain on the system.** Several nixpkgs packages link
  `libsystemd`/`libudev` from systemd's libraries (for example linux-pam). Raze's own
  components (r6, Rift) link none.
- **No socket activation yet.** nix-daemon starts at boot (systemd starts it on first
  use), which is most of r6's higher userspace memory at the login screen
  ([benchmarks](../performance/benchmarks.md)).
- **NixOS services without an r6 equivalent do not run.** The system build prints a
  warning naming them; declare them in `raze.r6.services` or use
  `raze.init = "systemd"`. Covered: everything the Raze profiles enable, plus sshd.
- **No timer units.** fstrim and logrotate run from one periodic r6 service; other
  NixOS timers you enable do not run under r6 (the build warning does not cover
  timers yet).
- **User services change at the next login** after `nixos-rebuild switch`; dbus,
  elogind, greetd and udevd changes apply at the next boot.
- **Upgrading from 1.0 needs a reboot** (`rix system boot`, not `switch`): PID 1
  changes from systemd to r6.

## Rift and Rift Shell

- **One GPU**: rendering and scan-out on the primary GPU only.
- **Screencopy is shm only**: recorders that need dmabuf capture (wl-screenrec) do
  not work; screen recording uses wf-recorder (CPU encoding).
- **No screen sharing through xdg-desktop-portal** (no portal backend for Rift's
  screencopy is configured), so browser screen sharing does not work yet.
- **No touch gestures or tablet input** yet; no mouse move/resize of tiled windows.
- **Night light** needs DRM gamma tables: it has no effect in nested (winit) or
  headless Rift, and on virtual GPUs without gamma support.
- **Rift Shell memory**: about 100 MB PSS for the Quickshell process with Qt's
  software renderer (the default); about 220 MB with `RIFT_SHELL_RENDERER=gl`.
- Rift draws no title bars (server-side decorations mode); clients that insist on
  client-side decorations draw their own.

## Installation and hardware

- **Installation needs a network connection** (small derivations are built and a few
  build tools are downloaded).
- **Untested in 1.1:** BIOS/legacy installs, manual partitioning, dual boot, real
  hardware, desktops other than Rift and niri, the `legacy` and `minimal` kernel
  profiles on installed systems.
- **Live boot is slower than it could be**: the ISO uses xz to stay under GitHub's
  2 GiB asset limit.
- The installer adds a UEFI boot entry; to boot the ISO again on the same machine,
  choose it in the firmware boot menu.
- `rix search` builds its index on first use; this evaluates nixpkgs once and needs
  about 2 GB of RAM and a minute or two.
- CJK fonts are not installed by default (`fonts.packages = [ pkgs.noto-fonts-cjk-sans ];`).
- **The ISO is not signed** (SHA-256 checksums only).
