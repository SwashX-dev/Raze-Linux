# Testing

| Layer | Command | What it covers |
|---|---|---|
| r6 unit | `cargo test -p r6 --lib` | config parsing/validation, dependency graph (topo order, waves, cycles), log rotation, kthread detection |
| r6 end-to-end | `cargo test -p r6 --test supervisor` | 18 tests with the real binaries: ordering, readiness, readiness timeout, crash loops, restart limit, dependency failure, oneshots, dependents-first stop, SIGKILL escalation, restart, invalid config, reload, enable persistence, logging, shutdown order, 50-service parallel activation, PID 1 stages + supervisor crash recovery |
| rix unit/integration | `cargo test -p rix` | nix32, NAR hashing vs `nix hash path`, store paths, profiles/rollback, manifests, flake.lock, search ranking |
| Raze Core boot | `nix build .#checks.x86_64-linux.core-boot` | r6 as PID 1 in QEMU: services, restart after SIGKILL, DHCP, logs, poweroff |
| Desktop VM | `nix build .#checks.x86_64-linux.desktop` | sudo-rs auth, branding, dbus-broker, zram, NetworkManager, greetd autologin, software fallback session, waybar, rix hash/verify vs Nix |
| Installer | manual, `tests/qemu/` helpers | Calamares GUI driven over QMP in a UEFI VM, then boot from disk |
| Benchmarks | `benchmarks/core-init.py` | r6 vs s6 boot latency and memory |

`tests/report.json` records the results of the release validation run.
