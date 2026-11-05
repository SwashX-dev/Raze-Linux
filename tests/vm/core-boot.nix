{ runCommand, qemu_kvm, image }:
runCommand "raze-core-boot-test" { nativeBuildInputs = [ qemu_kvm ]; requiredSystemFeatures = [ "kvm" ]; } ''
  timeout 120 qemu-system-x86_64 -enable-kvm -m 256 -smp 2 -nographic -no-reboot \
    -kernel ${image}/bzImage -initrd ${image}/initrd \
    -append "console=ttyS0 rdinit=/sbin/r6-init raze.autotest raze.poweroff panic=-1" \
    -nic user,model=virtio-net-pci > log.txt 2>&1 || true
  cat log.txt
  grep -q "RAZE-CORE: RESULT init=r6-init" log.txt
  grep -q "RAZE-CORE: restart-check PASS" log.txt
  grep -q "RAZE-CORE: network PASS" log.txt
  grep -q "r6-init: shutdown requested (Poweroff)" log.txt
  grep -q "reboot: Power down" log.txt
  mkdir -p $out && cp log.txt $out/
''
