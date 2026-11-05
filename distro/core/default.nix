# Raze Core: a tiny, systemd-free image (kernel + initramfs) in which r6-init is PID 1.
#
# Used to verify r6 as a real init on real hardware/VMs and to benchmark it against
# s6 under identical conditions. Userspace is busybox; services: network (udhcpc),
# a serial console shell, a logging demo service, and an optional self-test.
{ lib, pkgs, kernel ? pkgs.linuxPackages_latest.kernel, init ? "r6" }:
let
  busybox = pkgs.pkgsStatic.busybox;
  r6 = pkgs.pkgsStatic.r6 or pkgs.r6;
  boottime = pkgs.writeCBin "boottime" ''
    #include <stdio.h>
    #include <time.h>
    int main(void) {
      struct timespec ts; clock_gettime(CLOCK_BOOTTIME, &ts);
      printf("%.3f\n", ts.tv_sec * 1000.0 + ts.tv_nsec / 1e6);
      return 0;
    }
  '';
  udhcpcScript = pkgs.writeScript "udhcpc.script" ''
    #!/bin/sh
    case "$1" in
      deconfig) ip addr flush dev "$interface"; ip link set "$interface" up ;;
      bound|renew)
        ip addr flush dev "$interface"
        ip addr add "$ip/''${mask:-24}" dev "$interface"
        [ -n "$router" ] && ip route add default via "''${router%% *}" dev "$interface"
        : > /etc/resolv.conf
        for d in $dns; do echo "nameserver $d" >> /etc/resolv.conf; done
        echo "network: $interface bound to $ip" > /dev/console
        ;;
    esac
    exit 0
  '';
  # Benchmark services: N identical long-running services that record when they start.
  benchServices = 20;

  r6Services = pkgs.runCommand "core-r6-services" { } ''
    mkdir -p $out
    cat > $out/network.toml <<EOT
    description = "DHCP on eth0"
    command = ["/bin/udhcpc", "-f", "-i", "eth0", "-s", "/etc/udhcpc.script", "-t", "10", "-T", "1"]
    restart = "always"
    enable = true
    EOT
    cat > $out/console.toml <<EOT
    description = "Root shell on the serial console"
    command = ["/bin/cttyhack", "/bin/sh", "-l"]
    stop_signal = "HUP"
    restart = "always"
    log = "console"
    enable = true
    EOT
    cat > $out/ticker.toml <<EOT
    description = "Logging demo: prints a line every 10 s"
    command = ["/bin/sh", "-c", "while :; do echo tick; sleep 10; done"]
    enable = true
    EOT
    for i in $(seq 1 ${toString benchServices}); do
      cat > $out/bench$i.toml <<EOT
    command = ["/bin/sh", "-c", "/bin/boottime > /run/stamps/bench$i; exec sleep 100000"]
    log = "null"
    enable = true
    EOT
    done
    cat > $out/selftest.toml <<EOT
    description = "Automated boot self-test (enabled with raze.autotest on the kernel command line)"
    type = "oneshot"
    command = ["/etc/selftest"]
    depends = [${lib.concatMapStringsSep ", " (i: "\"bench${toString i}\"") (lib.range 1 benchServices)}]
    wants = ["network"]
    log = "console"
    enable = true
    EOT
  '';

  # Equivalent s6 tree for the comparison benchmark.
  s6Tree = pkgs.runCommand "core-s6-tree" { } ''
    mkdir -p $out
    mk() { mkdir -p $out/$1; printf '#!/bin/sh\n%s\n' "$2" > $out/$1/run; chmod +x $out/$1/run; }
    mk network 'exec /bin/udhcpc -f -i eth0 -s /etc/udhcpc.script -t 10 -T 1'
    mk console 'exec /bin/setsid /bin/cttyhack /bin/sh -l'
    mk ticker 'while :; do echo tick; sleep 10; done'
    mkdir -p $out/ticker/log
    printf '#!/bin/sh\nmkdir -p /run/log/ticker\nexec /bin/s6-log -b n3 s524288 T /run/log/ticker\n' > $out/ticker/log/run
    chmod +x $out/ticker/log/run
    for i in $(seq 1 ${toString benchServices}); do
      mk bench$i "/bin/boottime > /run/stamps/bench$i; exec sleep 100000"
    done
    mk selftest '/etc/selftest; exec sleep 100000'
  '';

  selftest = pkgs.writeScript "selftest" ''
    #!/bin/sh
    grep -q raze.autotest /proc/cmdline || exit 0
    # Wait for all benchmark services to have started.
    i=0; while [ "$(ls /run/stamps | wc -l)" -lt ${toString benchServices} ] && [ $i -lt 500 ]; do sleep 0.01; i=$((i+1)); done
    last=$(cat /run/stamps/* | sort -n | tail -1)
    echo "RAZE-CORE: init=$(cat /proc/1/comm) services_started_ms=$last"
    if [ "$(cat /proc/1/comm)" = r6-init ]; then
      pid1=$(r6 --json info | sed -n 's/.*"init_start_ms": \([0-9.]*\).*/\1/p')
    else
      pid1=$(cat /run/pid1-start)
    fi
    echo "RAZE-CORE: pid1_start_ms=$pid1"
    # Supervision overhead: RSS of the init + supervision processes.
    total=0; count=0
    for p in /proc/[0-9]*; do
      c=$(cat $p/comm 2>/dev/null) || continue
      case "$c" in r6-init|r6-supervise|s6-svscan|s6-supervise|s6-log|init)
        kb=$(awk '/^VmRSS/ {print $2}' $p/status); total=$((total + ''${kb:-0})); count=$((count+1)) ;;
      esac
    done
    echo "RAZE-CORE: supervision_processes=$count supervision_rss_kib=$total"
    echo "RAZE-CORE: mem_used_kib=$(awk '/MemTotal/ {t=$2} /MemAvailable/ {a=$2} END {print t-a}' /proc/meminfo)"
    echo "RAZE-CORE: processes=$(ls -d /proc/[0-9]* | wc -l)"
    if [ "$(cat /proc/1/comm)" = r6-init ]; then
      r6 status
      # Supervision check: kill a service and verify it comes back.
      old=$(r6 --json status ticker | sed -n 's/.*"pid": \([0-9]*\).*/\1/p')
      kill -9 "$old"; sleep 1
      new=$(r6 --json status ticker | sed -n 's/.*"pid": \([0-9]*\).*/\1/p')
      [ -n "$new" ] && [ "$new" != "$old" ] && echo "RAZE-CORE: restart-check PASS ($old -> $new)" || echo "RAZE-CORE: restart-check FAIL"
      r6 logs ticker -n 3
      i=0; while ! grep -q nameserver /etc/resolv.conf 2>/dev/null && [ $i -lt 100 ]; do sleep 0.1; i=$((i+1)); done
      if ip -4 addr show eth0 2>/dev/null | grep -q inet; then
        echo "RAZE-CORE: network PASS ($(ip -4 -o addr show eth0 | awk '{print $4}'))"
      else
        echo "RAZE-CORE: network FAIL"; ip link; r6 logs network -n 20
      fi
    fi
    echo "RAZE-CORE: selftest done"
    if grep -q raze.poweroff /proc/cmdline; then
      echo "RAZE-CORE: poweroff_request_ms=$(/bin/boottime)"
      if [ "$(cat /proc/1/comm)" = r6-init ]; then r6 poweroff; fi
    fi
  '';

  s6Init = pkgs.writeScript "s6-init" ''
    #!/bin/sh
    # Minimal s6 PID 1 equivalent to r6-init's job here: mount API fs, exec s6-svscan.
    t0=$(/bin/boottime)
    export PATH=/bin:/sbin
    mount -t proc proc /proc; mount -t sysfs sys /sys
    mount -t devtmpfs dev /dev 2>/dev/null; mount -t tmpfs -o mode=0755 run /run
    mkdir -p /run/stamps /run/s6
    echo "$t0" > /run/pid1-start
    cp -r /etc/s6/sv /run/s6/sv
    /etc/r6/rc.init
    exec /bin/s6-svscan /run/s6/sv
  '';

  # Kernel modules for virtio devices (no-ops on kernels with virtio built in).
  modulesClosure = pkgs.makeModulesClosure {
    kernel = kernel.modules or kernel;
    firmware = pkgs.emptyDirectory;
    rootModules = [ "virtio_pci" "virtio_net" "virtio_blk" "virtio_rng" "af_packet" ];
    allowMissing = true;
  };
  # busybox modprobe loads plain .ko files: decompress and regenerate modules.dep.
  modules = pkgs.runCommand "raze-core-modules" { nativeBuildInputs = [ pkgs.xz pkgs.zstd pkgs.kmod ]; } ''
    mkdir -p $out
    if [ -d ${modulesClosure}/lib ]; then
      cp -r --no-preserve=mode ${modulesClosure}/lib $out/lib
      find $out/lib -name '*.ko.xz' -exec xz -d {} +
      find $out/lib -name '*.ko.zst' -exec zstd -d --rm -q {} +
      for v in $out/lib/modules/*; do depmod -b $out $(basename $v); done
    else
      mkdir -p $out/lib/modules
    fi
  '';

  rcInit = pkgs.writeScript "rc.init" ''
    #!/bin/sh
    for m in virtio_pci virtio_net virtio_blk virtio_rng af_packet; do modprobe -q $m 2>/dev/null; done
    mkdir -p /run/stamps /var/log /tmp
    ip link set lo up
    hostname raze-core
  '';

  # One root tree: busybox applets, r6, s6 and the image's configuration.
  root = pkgs.runCommand "raze-core-root" { } ''
    mkdir -p $out/bin $out/sbin $out/etc/r6
    ln -s ${modules}/lib $out/lib
    for a in $(${busybox}/bin/busybox --list); do ln -s ${busybox}/bin/busybox $out/bin/$a; done
    ln -sf ${boottime}/bin/boottime $out/bin/boottime
    ln -sf ${r6}/bin/r6 $out/bin/r6
    ln -sf ${r6}/bin/r6-log $out/bin/r6-log
    ln -s ${r6}/bin/r6-init $out/sbin/r6-init
    ln -s ${r6}/bin/r6-supervise $out/sbin/r6-supervise
    ln -s ${s6Init} $out/sbin/s6-init
    for b in ${pkgs.s6}/bin/*; do ln -sf $b $out/bin/$(basename $b); done
    cp ${udhcpcScript} $out/etc/udhcpc.script
    cp ${selftest} $out/etc/selftest
    cp ${rcInit} $out/etc/r6/rc.init
    cp -r ${r6Services} $out/etc/r6/services
    mkdir -p $out/etc/s6 && cp -r ${s6Tree} $out/etc/s6/sv
    echo "root:x:0:0:root:/root:/bin/sh" > $out/etc/passwd
    echo "root:x:0:" > $out/etc/group
    printf "export PATH=/bin:/sbin\nexport PS1='raze-core# '\n" > $out/etc/profile
  '';

  initrd = pkgs.makeInitrdNG {
    name = "raze-core-${init}";
    compressor = "zstd";
    contents = [
      { source = "${root}/bin"; target = "/bin"; }
      { source = "${root}/sbin"; target = "/sbin"; }
      { source = "${root}/etc"; target = "/etc"; }
      { source = "${root}/lib"; target = "/lib"; }
    ];
  };
in
pkgs.runCommand "raze-core-image" { passthru = { inherit initrd kernel r6Services s6Tree; }; } ''
  mkdir -p $out
  ln -s ${kernel}/bzImage $out/bzImage
  ln -s ${initrd}/initrd $out/initrd
  cat > $out/README <<EOT
  Raze Core image: boot with
    qemu-system-x86_64 -enable-kvm -m 256 -kernel bzImage -initrd initrd \
      -append "console=ttyS0 rdinit=/sbin/r6-init" -nographic -nic user,model=virtio-net-pci
  EOT
''
