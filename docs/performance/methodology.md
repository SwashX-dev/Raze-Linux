# Performance methodology

Rules: measure a baseline, change one thing, measure again with repeated runs,
keep or revert, document. Numbers are reported with the environment that produced
them. Claims are limited to what was measured.

## Reference environment

| | |
|---|---|
| Host | AMD Ryzen 5 7235HS (4 cores / 8 threads), 31 GiB RAM, Samsung NVMe SSD, NVIDIA RTX 4050 Laptop GPU (virgl host), NixOS 26.05, Linux 6.18.57 |
| Hypervisor | QEMU 11.1.2 with KVM, `-cpu host` |
| Guest (Raze Core) | 2 vCPU, 256 MiB, virtio-net, no disk |
| Guest (desktop) | 4 vCPU, 4 GiB, q35, OVMF, virtio disk/net, virtio-vga-gl (virgl) or virtio-vga |

| Guest (1.1 boot benchmark) | 4 vCPU, 3 GiB, NixOS VM runner (virtiofs store), virtio, `-vga std` (no 3D) |
| Guest (1.1 compositor benchmark) | 4 vCPU, 4 GiB, `virtio-vga-gl` on `egl-headless` (virgl, host NVIDIA GPU) |

The host was otherwise idle except where noted. VM numbers do not transfer
directly to bare metal; they are used to compare configurations under identical
conditions.

## Clocks

- `CLOCK_BOOTTIME` (milliseconds since the kernel started timekeeping) for
  in-guest boot milestones: r6 records it at `r6-init` entry, supervisor start and
  boot completion; benchmark services record it when they start; the s6 PID 1
  records it as its first action.
- `systemd-analyze` for systemd-based boots (kernel, initrd, userspace).
- `hyperfine` for command latency (warm-up runs discarded).

## r6 vs s6 (benchmarks/core-init.py)

Same kernel, same initramfs, same 23 services (20 identical benchmark services,
DHCP, a console shell, a logging ticker). Only PID 1 and the supervision tree
differ: `r6-init` + `r6-supervise`, or a 6-line shell PID 1 that mounts the API
filesystems and execs `s6-svscan` (+ one `s6-supervise` per service, `s6-log` for
the ticker). Both statically linked against musl. Runs are interleaved r6/s6 so
host drift affects both equally. Reported: median ± stdev of 10 boots.

Memory: summed RSS overstates processes that share pages, so PSS (from
`/proc/<pid>/smaps_rollup`) is reported as well.

## r6 vs systemd (benchmarks/desktop-init.py)

The same Raze Desktop configuration built twice, differing only in `raze.init`
(plus the guest agent as a systemd unit for the systemd build). One warm-up boot per
variant (creates the disk and machine-id), then interleaved boots. Milestones are
process start times (`/proc/<pid>/stat` field 22) of greetd and NetworkManager,
read through the QEMU guest agent, so neither system carries extra instrumentation.
Memory and processes are sampled 20 s after greetd started. VMs are stopped with
SIGKILL (which is how the activation-flush bug in r6-init was found).

## Rift vs niri (benchmarks/compositor.py)

Both desktops on the same r6 system with automatic login, virgl GPU (both
compositors render with GLES). After the session is up and one terminal is open,
20 s settle, then: compositor CPU time (utime + stime) over 60 s, compositor RSS/PSS,
PSS of every process of the session user, MemTotal − MemAvailable; then a terminal
that prints a line every 50 ms for 10 s (the harness checks that its window opened)
and CPU time of the compositor and the session during those 10 s. One warm-up run
per variant, then interleaved runs.
