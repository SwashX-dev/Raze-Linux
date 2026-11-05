# Kernel

Raze 1.0 uses **Linux 7.3.1**, the latest stable release on kernel.org at release
time (stable 7.3.1, 2026-10-30), as packaged by the pinned nixpkgs revision
`1d060086d60cb76a40cf37109f979d88a5bd6396` (`linux_latest`).

## Baseline before tuning

The nixpkgs build of 7.3.1 already uses the interactive-desktop options one would
otherwise patch in:

| Option | Value |
|---|---|
| `CONFIG_HZ` | 1000 |
| `CONFIG_PREEMPT_DYNAMIC` | y (model selectable with `preempt=` at boot) |
| `CONFIG_LRU_GEN` / `_ENABLED` | y / y (MGLRU on) |
| `CONFIG_SCHED_CLASS_EXT` | y (sched_ext) |
| `CONFIG_RCU_LAZY` | y |
| `CONFIG_ZSWAP_COMPRESSOR_DEFAULT` | zstd |
| `CONFIG_TRANSPARENT_HUGEPAGE_MADVISE` | y |

Rebuilding that kernel with the same options would cost binary-cache hits and add
nothing, so Raze does not do it. **Raze 1.0 carries no kernel patches.** The
profiles below are configuration choices and boot parameters, not patches.

## Profiles (`raze.kernel.profile`)

| Profile | Kernel | Differences | Status |
|---|---|---|---|
| `generic` (module default) | 7.3.1 (nixpkgs build) | `quiet loglevel=3` | Installed and boot-tested |
| `performance` (**installer default since 1.1**) | 7.3.1 (same binary, so installs never compile a kernel) | `preempt=full`, sched_ext `scx_bpfland` (an r6 service), autogroup, bounded dirty bytes | Boot-tested under r6 in the desktop VM test (sched_ext active); not benchmarked on desktop workloads yet |
| `legacy` | 6.18 LTS (6.18.60) | `transparent_hugepage=madvise` | Builds; no legacy hardware tested |
| `minimal` | 7.3.1, **custom config** | `x86_64_defconfig` + `kvm_guest.config` + [`config/minimal-kvm.config`](../../kernel/config/minimal-kvm.config); virtio built in, RAID6 benchmark/netconsole/DEBUG_WX off, ATA modular | QEMU/KVM guests only; boot-tested with Raze Core: 16 % faster kernel boot and 30 % less memory than generic (see benchmarks). Not tested as an installed desktop kernel. |

The minimal config build fails if any requested option does not survive Kconfig
dependency resolution (it caught `CGROUP_BPF` needing `BPF_SYSCALL`).

Measurements are in [benchmarks](../performance/benchmarks.md). Security features
(KASLR, stack protector, hardened usercopy, module signature enforcement where
configured by upstream) are not disabled by any profile.
