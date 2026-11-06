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
| Partitions | Nothing is preselected. **Erase disk** creates a 1 GiB EFI partition and an ext4 (or btrfs/xfs) root; no swap partition by default (Raze uses zram). **Install alongside** shrinks a partition of another system (drag the divider to choose the sizes) and installs Raze in the freed space; **Replace a partition** installs into an existing partition; **Manual partitioning** for everything else. See [Dual boot](#dual-boot). Encrypted automatic partitioning is not offered. |
| Desktop | Raze Desktop (Rift + Rift Shell) is the default and is shipped on the ISO; niri, Hyprland, dwl, Xfce, LXQt, KDE Plasma, GNOME and dwm are downloaded during installation. All of them run on r6 except GNOME, which requires systemd: choosing GNOME installs systemd as the init system. |
| Profile | Raze Desktop, Raze Development (adds compilers, debuggers, Podman, direnv), Raze Minimal (console only). |
| Users | Name, login, password. The user is in `wheel` and can use `sudo` (sudo-rs). |
| Summary | Review. Nothing is written before you confirm the final dialog. |
| Install | Partitions, writes `/etc/nixos`, runs `nixos-install`. The log is kept at `/var/log/raze-install.log` on the new system. |

## Dual boot

Raze can share a disk (and its EFI system partition) with Windows or another Linux
distribution.

- **Make room first if you can.** On Windows, shrink the Windows partition in Disk
  Management (or let the installer shrink it), and disable *Fast Startup*
  (Control Panel → Power Options) and BitLocker for that partition: a hibernated or
  encrypted NTFS partition cannot be resized safely. Back up important data.
- In the installer choose **Install alongside**, pick the partition to shrink and
  drag the divider. Raze reuses the existing EFI system partition (mounted at
  `/boot/efi`; only GRUB's EFI program is written there, kernels stay on the Raze
  root partition, so a 100 MB Windows ESP is enough).
- The installer looks for other systems (`os-prober`, and the boot programs on the
  EFI partitions) and adds them to the GRUB menu: *Windows Boot Manager*, other
  distributions' GRUB/shim, systemd-boot. The menu waits 5 seconds when other
  systems are present. The entries are written to `boot.loader.grub.extraEntries`
  in `/etc/nixos/configuration.nix` (UEFI) or come from
  `boot.loader.grub.useOSProber = true` (BIOS); edit them there.
- The firmware's boot order gets a **Raze** entry first. To start Windows directly
  instead, change the order in the firmware setup or with `efibootmgr`.
- After its partition was shrunk, Windows checks the disk the first time it starts
  (it may reboot once); that is expected.
- Windows keeps the hardware clock in local time, Raze in UTC. Either set Windows to
  UTC (registry value `RealTimeIsUniversal`) or add
  `time.hardwareClockInLocalTime = true;` to `configuration.nix`.

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
| UEFI dual boot: GPT disk with a 100 MB EFI partition holding *Windows Boot Manager* and an NTFS partition (a stand-in, not a real Windows installation), **Install alongside** | NTFS partition shrunk and still consistent; Windows' EFI partition reused; GRUB menu lists *Windows Boot Manager* and Raze Linux; Raze boots to the greeter; the Windows entry chainloads `bootmgfw.efi` |
| Generated installer configurations | `installer-configs` flake check evaluates the `configuration.nix` written for UEFI and BIOS dual boot, UEFI erase disk, GNOME and Xfce |
| Dual boot with a real Windows installation, BIOS/legacy boot, manual partitioning, other desktops, real hardware | **Not tested for 1.1** |

### 1.0.0

| Scenario | Result |
|---|---|
| UEFI (OVMF), empty 30 GB virtio disk, erase disk, ext4, Raze Desktop (niri), QEMU/KVM with virgl | Full graphical install of the 1.0.0 ISO; booted from disk without the ISO, logged in, network, `rix install`, sudo-rs, services, reboot, persistence, poweroff verified |
| Live ISO and installed system without 3D acceleration | Software fallback session (sway + pixman) with waybar and Calamares |
| BIOS/legacy boot, manual partitioning, dual boot, other desktops, real hardware | **Not tested for 1.0** |
