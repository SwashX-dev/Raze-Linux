# System-wide performance defaults. Each item is justified in
# docs/performance/benchmarks.md, with measurements where they were taken.
{ config, lib, pkgs, ... }:
{
  # dbus-broker: lower latency and memory than the reference dbus-daemon.
  services.dbus.implementation = lib.mkDefault "broker";

  # Compressed RAM swap instead of a disk swap partition: keeps the system responsive
  # under memory pressure on low-RAM machines.
  zramSwap = {
    enable = lib.mkDefault true;
    algorithm = "zstd";
    memoryPercent = 50;
  };
  boot.kernel.sysctl = {
    # With zram, swapping is cheap compared to dropping page cache.
    "vm.swappiness" = lib.mkDefault 100;
    "vm.page-cluster" = lib.mkDefault 0;
    "vm.watermark_boost_factor" = lib.mkDefault 0;
  };

  # Keep logs bounded and in RAM-friendly sizes.
  services.journald.settings.Journal = {
    SystemMaxUse = "200M";
    RuntimeMaxUse = "50M";
  };

  # Userspace OOM handling before the kernel thrashes.
  systemd.oomd.enable = lib.mkDefault true;

  # Faster boot: don't wait for the network to be "online" before reaching the desktop.
  systemd.services.NetworkManager-wait-online.enable = lib.mkDefault false;
  systemd.network.wait-online.enable = lib.mkDefault false;

  # Scheduler-agnostic I/O defaults: none for NVMe, mq-deadline for SATA SSD, BFQ for HDD.
  services.udev.extraRules = ''
    ACTION=="add|change", KERNEL=="nvme[0-9]*n[0-9]*", ATTR{queue/scheduler}="none"
    ACTION=="add|change", KERNEL=="sd[a-z]*|mmcblk[0-9]*", ATTR{queue/rotational}=="0", ATTR{queue/scheduler}="mq-deadline"
    ACTION=="add|change", KERNEL=="sd[a-z]*", ATTR{queue/rotational}=="1", ATTR{queue/scheduler}="bfq"
  '';

  # fstrim weekly instead of continuous discard.
  services.fstrim.enable = lib.mkDefault true;
}
