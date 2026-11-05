# Performance methodology

Rules: measure a baseline, change one thing, measure again with repeated runs,
keep or revert, document. Numbers are reported with the environment that produced
them. Claims are limited to what was measured.

## Reference environment (1.0.0 measurements)

| | |
|---|---|
| Host | AMD Ryzen 5 7235HS (4 cores / 8 threads), 31 GiB RAM, Samsung NVMe SSD, NVIDIA RTX 4050 Laptop GPU (virgl host), NixOS 26.05, Linux 6.18.57 |
| Hypervisor | QEMU 11.1.2 with KVM, `-cpu host` |
| Guest (Raze Core) | 2 vCPU, 256 MiB, virtio-net, no disk |
| Guest (desktop) | 4 vCPU, 4 GiB, q35, OVMF, virtio disk/net, virtio-vga-gl (virgl) or virtio-vga |

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
