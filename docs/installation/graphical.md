# Graphical installation

## Requirements

| | Minimum | Recommended |
|---|---|---|
| CPU | x86_64 | x86_64 with 2+ cores |
| RAM | 2 GB | 4 GB |
| Disk | 12 GB | 30 GB+ |
| Firmware | UEFI or BIOS | UEFI |
| Network | Required during installation | Wired or Wi-Fi |
| GPU | Any (Rift renders with pixman without 3D acceleration) | OpenGL ES 3 capable GPU |

## 1. Write the ISO

Download `raze-linux-v1.1.0-x86_64.iso` and `SHA256SUMS` from the
[release page](https://github.com/SwashX-dev/Raze-Linux/releases), then verify:

```bash
sha256sum -c SHA256SUMS
```

Write to a USB stick (this erases the stick; replace `/dev/sdX`):

```bash
sudo dd if=raze-linux-v1.1.0-x86_64.iso of=/dev/sdX bs=4M status=progress oflag=sync
```

## 2. Boot the live system

Boot from the USB stick. The live session logs in automatically and opens the
installer. On machines without a 3D-capable GPU (including most VMs without 3D
acceleration) the live session uses the software-rendered fallback; everything works,
just with CPU rendering.

Keys in the live session: `Super+Enter` terminal, `Super+D` launcher,
`Super+Escape` power menu, `Super+Shift+/` all shortcuts. The installer can be
reopened from the red **Install Raze** button in the top bar.

## 3. Installer steps

| Step | Notes |
|---|---|
| Welcome | Language. Checks RAM, disk space and the internet connection. |
| Location | Time zone, system language, number/date formats. |
| Keyboard | Layout and variant. |
| Partitions | **Erase disk** (tested) creates a 1 GiB EFI partition and an ext4 (or btrfs/xfs) root; no swap partition by default (Raze uses zram). **Manual partitioning** is available but not validated for 1.0. Encrypted automatic partitioning is not offered in 1.0. |
| Desktop | Raze Desktop (Rift + Rift Shell) is the default and is shipped on the ISO; niri, Hyprland, dwl, Xfce, LXQt, KDE Plasma, GNOME and dwm are downloaded during installation. |
| Profile | Raze Desktop, Raze Development (adds compilers, debuggers, Podman, direnv), Raze Minimal (console only). |
| Users | Name, login, password. The user is in `wheel` and can use `sudo` (sudo-rs). |
| Summary | Review. Nothing is written before you confirm the final dialog. |
| Install | Partitions, writes `/etc/nixos`, runs `nixos-install`. The log is kept at `/var/log/raze-install.log` on the new system. |

## 4. First boot

Remove the installation medium and reboot. GRUB (UEFI and BIOS)
shows **Raze Linux**; log in at the greeter. Then:

```bash
rix search firefox        # first run builds the package index (about a minute)
rix install firefox
sudo rix system switch    # apply edits to /etc/nixos/configuration.nix
```

## What the installer writes

```
/etc/nixos/flake.nix                 # inputs.raze = path:./raze
/etc/nixos/configuration.nix         # raze.profile, raze.desktop, users, locale, bootloader
/etc/nixos/hardware-configuration.nix
/etc/nixos/raze/                     # pinned copy of the Raze tree the ISO was built from
```

## Tested scenarios

### 1.1.0

| Scenario | Result |
|---|---|
| UEFI (OVMF), empty 30 GB virtio disk, erase disk, ext4, Raze Desktop (Rift), QEMU/KVM without 3D | Full graphical install of the 1.1.0 ISO under r6; "Restart now"; GRUB from disk; login; Rift + Rift Shell; fastfetch; terminal |
| Live ISO without 3D acceleration | r6 PID 1, Rift (pixman) + Rift Shell, installer autostart |
| Upgrade of a 1.0.0 installation | See [upgrade.md](upgrade.md): upgrade, live switch to Rift, rollback to 1.0 |
| BIOS/legacy boot, manual partitioning, dual boot, other desktops, real hardware | **Not tested for 1.1** |

### 1.0.0

| Scenario | Result |
|---|---|
| UEFI (OVMF), empty 30 GB virtio disk, erase disk, ext4, Raze Desktop (niri), QEMU/KVM with virgl | Full graphical install of the 1.0.0 ISO; booted from disk without the ISO, logged in, network, `rix install`, sudo-rs, services, reboot, persistence, poweroff verified |
| Live ISO and installed system without 3D acceleration | Software fallback session (sway + pixman) with waybar and Calamares |
| BIOS/legacy boot, manual partitioning, dual boot, other desktops, real hardware | **Not tested for 1.0** |
