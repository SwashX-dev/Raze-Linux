# Building

Requirements: Linux x86_64, Nix ≥ 2.18 with flakes, ~40 GB free disk for an ISO
build, KVM for VM tests.

```bash
git clone https://github.com/SwashX-dev/Raze-Linux && cd Raze-Linux
nix develop                         # Rust toolchain, QEMU, OVMF, s6, hyperfine
cargo test --workspace              # r6 + rix tests on the host

nix build .#r6 .#rix                # packages (tests run inside the Nix sandbox too)
nix build .#core-image              # Raze Core (kernel + r6 initramfs)
nix build .#kernel-minimal          # custom Raze Minimal kernel
nix build .#iso                     # live installer ISO -> result/iso/
```

Everything is pinned by `flake.lock` (nixpkgs `1d060086d60c…`). Bump deliberately:

```bash
nix flake update && nix build .#iso && nix flake check
```

Try the ISO in QEMU (UEFI; add `-device virtio-vga-gl -display gtk,gl=on` for 3D):

```bash
qemu-img create -f qcow2 disk.qcow2 30G
qemu-system-x86_64 -enable-kvm -m 4096 -smp 4 -machine q35 \
  -bios $(nix build --no-link --print-out-paths nixpkgs#OVMF.fd)/FV/OVMF.fd \
  -cdrom result/iso/raze-linux-v1.0.0-x86_64.iso -drive file=disk.qcow2,if=virtio \
  -device virtio-vga -nic user,model=virtio-net-pci
```
