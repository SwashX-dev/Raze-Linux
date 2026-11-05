# Testing

| Layer | Command | What it covers |
|---|---|---|
| r6 unit | `cargo test -p r6 --lib` (and `--bins`) | config parsing/validation, capabilities, dependency graph (topo order, waves, cycles), log rotation, kthread detection, tmpfiles.d parsing (verbatim NixOS lines), syslog message parsing |
| r6 end-to-end | `cargo test -p r6 --test supervisor` | 21 tests with the real binaries: ordering, readiness, readiness timeout, crash loops, restart limit, dependency failure, oneshots, dependents-first stop, SIGKILL escalation, restart, invalid config, reload, enable persistence, logging, shutdown order, 50-service parallel activation, PID 1 stages + supervisor crash recovery, setenv, umask/rlimit, user supervisor |
| rix | `cargo test -p rix` | nix32, NAR hashing vs `nix hash path`, store paths, profiles/rollback, manifests, flake.lock, search ranking |
| Rift | `cargo test --manifest-path rift/Cargo.toml` | layout engine (columns, scrolling, workspaces, monitors/hotplug, rules, animation), configuration (parsing, merging over defaults, validation, binds), night-light ramps |
| rift-shell | `cargo test --manifest-path rift-shell/Cargo.toml` | builds the helper (no unit tests yet; behaviour is covered by the desktop VM test) |
| Raze Core boot | `nix build .#checks.x86_64-linux.core-boot` | r6 as PID 1 in QEMU: services, restart after SIGKILL, DHCP, logs, poweroff |
| Desktop VM (r6) | `nix build .#checks.x86_64-linux.desktop` | r6 PID 1 with no systemd process, every enabled service up, firewall rules, sched_ext (performance profile), syslog, sudo-rs, os-release/fastfetch, NetworkManager, ntpd-rs, Rift (pixman, no 3D) + Rift Shell + clipboard history, windows over IPC, grim (ext-image-copy-capture), wf-recorder (wlr-screencopy), notifications, session lock, switch-to-configuration, rix hashing |
| Desktop VM (systemd) | `nix build .#checks.x86_64-linux.desktop-systemd` | the same desktop with `raze.init = "systemd"` |
| Installer, live ISO, upgrade | manual, `tests/qemu/` helpers (QMP + guest agent) | live boot under r6, Calamares GUI install in a UEFI VM, boot from disk, 1.0 → 1.1 upgrade |
| Benchmarks | `benchmarks/*.py` | r6 vs s6 (Raze Core), r6 vs systemd (desktop boot), Rift vs niri |

The desktop VM tests run under r6 although the NixOS test driver expects systemd:
the driver's root shell (`backdoor`) is an r6 service in test VMs
(`distro/modules/r6.nix`), and the test script uses r6 commands instead of
`wait_for_unit`.

Headless Rift for development: `rift --backend headless --size 1280x720` renders
with pixman in memory; Wayland clients, `rift msg`, grim and wf-recorder work
against it.
