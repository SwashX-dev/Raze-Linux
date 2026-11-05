# Benchmarks (1.0.0)

Environment and method: [methodology.md](methodology.md). Raw data:
[`benchmarks/results/`](../../benchmarks/results).

## r6 vs s6 — Raze Core, 10 interleaved boots each

QEMU/KVM, 2 vCPU, 256 MiB, Linux 7.3.1 (generic), same initramfs and services.

| Metric (median ± stdev) | r6 | s6 |
|---|---|---|
| PID 1 start → last of 20 services started | 32.31 ± 2.67 ms | 32.37 ± 1.19 ms |
| Kernel start → last service started | 234.7 ± 4.33 ms | 232.39 ± 4.25 ms |
| Init + supervision processes | 2 | 27 |
| Supervision RSS (sum) | 1778 ± 31.74 KiB | 3172 ± 8.04 KiB |
| Supervision PSS (sum) | 1766 ± 31.74 KiB | 1223 ± 8.04 KiB |
| System memory used (MemTotal − MemAvailable) | 51176 ± 1831.98 KiB | 53270 ± 1144.93 KiB |

**Reading:** activation latency is the same within noise. r6 runs 25 fewer
processes and about half the summed RSS. s6 has lower PSS: its many small
processes share text pages, while r6-supervise is one larger binary
(serde/TOML parsing included). The system-memory difference is within one
standard deviation. r6 is not faster than s6; it is as fast with a different
process model.

### r6 optimisation history (same benchmark)

| Change | PID 1 → services (median) |
|---|---|
| Initial (3 boots: 46.3, 43.4, 45.5 ms) | ≈ 45 ms |
| Event-driven `rc.init` wait (signalfd instead of 10 ms polling) + per-service console messages behind `r6.verbose` | 32.3 ms |

| Change | Poweroff request → power down |
|---|---|
| Initial (`kill(-1, 0)` counted kernel threads, so the wait always ran 3 s) | ≈ 3.2 s |
| `/proc` scan ignoring `PF_KTHREAD` | ≈ 0.2 s |

## Installed Raze Desktop (QEMU/KVM, 4 vCPU, 4 GiB, OVMF, virgl)

| Metric | Value |
|---|---|
| Boot to `graphical.target` (`systemd-analyze`) | 4.70 s = 0.48 s kernel + 2.15 s initrd + 2.06 s userspace |
| Memory used after login (niri, waybar, mako, foot open; `free -m`) | 681 MiB |
| Live ISO boot to `graphical.target`, zstd squashfs (pre-release build) | 6.64 s = 0.50 s kernel + 3.17 s initrd + 2.98 s userspace |
| Live ISO boot to `graphical.target`, **1.0.0 ISO (xz)**, no 3D | 12.47 s = 0.50 s kernel + 4.60 s initrd + 7.38 s userspace |
| Memory used, installed desktop after reboot (niri, waybar, foot) | 525 MiB |
| Full graphical installation (Calamares, erase disk, default desktop) | 448 s, host concurrently compiling a kernel |
| Software-rendered session idle (no 3D; waybar reading) | 0.4 GiB |

## Kernel profiles — Raze Core, r6 as PID 1, 10 boots each (median ± stdev)

QEMU/KVM, 2 vCPU, 256 MiB. "Kernel start → PID 1" is `CLOCK_BOOTTIME` at r6-init entry.

| Metric | Generic 7.3.1 | Minimal v1 | **Minimal v2 (1.0.0)** |
|---|---|---|---|
| Kernel start → PID 1 | 201.4 ± 3.2 ms | 360.9 ± 4.7 ms | **169.6 ± 3.5 ms** |
| Kernel start → all services started | 235.9 ± 4.9 ms | 377.7 ± 5.5 ms | **188.5 ± 4.3 ms** |
| PID 1 → all services started | 33.2 ± 3.5 ms | 16.9 ± 1.6 ms | 18.8 ± 1.6 ms |
| System memory used | 50.2 ± 3.7 MB | 35.8 ± 1.2 MB | **34.9 ± 1.9 MB** |
| bzImage | 14.6 MB | 15.8 MB | 15.6 MB |

v1 (defconfig + kvm_guest + virtio built in) used 30 % less memory but booted
**160 ms slower**: built-in drivers probe synchronously. Boot-log gap analysis found
the RAID6 algorithm benchmark (pulled in by built-in btrfs, ~160 ms), libata probing
the QEMU ATA port (~90 ms), netconsole (~48 ms) and the `DEBUG_WX` page-table scan
(~45 ms). v2 disables the benchmark, makes ATA modular, drops netconsole and
`DEBUG_WX` (a boot-time warning, not a protection): **16 % faster kernel boot, 20 %
faster to services, 30 % less memory than generic.** PID 1 → services is faster on
both minimal kernels because no modules need loading in `rc.init`.

The minimal profile targets QEMU/KVM guests only and has been boot-tested with
Raze Core, not as an installed desktop kernel.

## Rix search — pinned nixpkgs 1d060086 (113 419 packages)

Host: Ryzen 5 7235HS, idle. `hyperfine -N --warmup 3 --runs 20`.

| Operation | Time | Peak memory |
|---|---|---|
| `nix search --no-eval-cache nixpkgs firefox` (cold) | 26.9 s | 4.5 GB |
| `rix update` — one-time index build (same evaluation) | 27.9 s | 4.6 GB |
| `nix search nixpkgs firefox` (warm eval cache) | 1734 ± 18 ms | 44.6 MiB |
| **`rix search firefox`** (index) | **63.7 ± 1.2 ms** (27× faster) | 41.4 MiB |

The index is an 11 MB text file per nixpkgs revision. Rix pays the evaluation cost
once per revision instead of on every query; results are identical because they
come from the same `nix search` evaluation.

## ISO size

| Build | Size | Change |
|---|---|---|
| First build (zstd-15, reference closure, NixOS defaults) | 3.08 GB | — |
| No speech-dispatcher voices, no default font set, gvfs without samba | 2.40 GB | −0.68 GB |
| C GPU probe instead of mesa-demos, wiremix instead of pavucontrol, xz | 2.18 GB | −0.22 GB |
| xz + x86 BCJ, no separate reference closure, locale list instead of the all-locales archive, no gparted | **2.11 GB (1.961 GiB)** | −0.07 GB |

The remaining size is dominated by linux-firmware (825 MiB, already compressed),
Mesa/LLVM (needed for GPU drivers and software rendering) and the Qt stack used by
Calamares. Trade-off: xz makes the live session boot slower than zstd did; it does
not affect installed systems.
