# Kernel profiles.
#
# Measured starting point (see docs/architecture/kernel.md): the nixpkgs build of
# Linux 7.3.1 already uses HZ=1000, PREEMPT_DYNAMIC, MGLRU (enabled), sched_ext,
# RCU_LAZY and zstd zswap. Rebuilding it with those same options would only cost
# reproducibility and binary-cache hits, so the generic/performance profiles share
# the upstream binary and differ in boot-time tuning. Only `minimal` is a custom
# configuration build.
{ config, lib, pkgs, ... }:
let
  cfg = config.raze.kernel;
in
{
  options.raze.kernel = {
    profile = lib.mkOption {
      type = lib.types.enum [ "generic" "performance" "legacy" "minimal" ];
      default = "generic";
      description = ''
        generic:     latest stable kernel (7.3.x), broad hardware support.
        performance: same kernel; full preemption, sched_ext (scx_bpfland) and
                     responsiveness-oriented sysctls for desktops.
        legacy:      6.18 LTS kernel with conservative defaults for older machines.
        minimal:     custom reduced configuration for QEMU/KVM guests (experimental).
      '';
    };
  };

  config = lib.mkMerge [
    {
      boot.kernelPackages = {
        generic = pkgs.linuxPackages_latest;
        performance = pkgs.linuxPackages_latest;
        legacy = pkgs.linuxPackages_6_18;
        minimal = pkgs.linuxPackagesFor (pkgs.callPackage ../../kernel/minimal.nix { });
      }.${cfg.profile};
      boot.kernelParams = [ "quiet" "loglevel=3" "udev.log_level=3" ];
      boot.consoleLogLevel = lib.mkDefault 3;
    }

    (lib.mkIf (cfg.profile == "performance") {
      # Full preemption lowers worst-case input/audio latency under load.
      # PREEMPT_DYNAMIC makes this a boot parameter rather than a rebuild.
      boot.kernelParams = [ "preempt=full" ];
      # sched_ext scheduler tuned for interactive desktops; falls back to EEVDF
      # automatically if it exits.
      services.scx = {
        enable = true;
        scheduler = "scx_bpfland";
      };
      boot.kernel.sysctl = {
        "kernel.sched_autogroup_enabled" = 1;
        "vm.dirty_bytes" = 268435456;
        "vm.dirty_background_bytes" = 67108864;
      };
    })

    (lib.mkIf (cfg.profile == "legacy") {
      # Older machines: no zswap pressure on slow CPUs, keep THP opt-in.
      boot.kernelParams = [ "transparent_hugepage=madvise" ];
    })
  ];
}
