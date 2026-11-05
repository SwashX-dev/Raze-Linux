# Upgrading from Raze Linux 1.0 to 1.1

An installed Raze system is built from the Raze tree copied to `/etc/nixos/raze` at
installation (`inputs.raze.url = "path:./raze"` in `/etc/nixos/flake.nix`). Upgrading
means replacing that tree with the 1.1 release and rebuilding. Your
`configuration.nix`, users and data are kept; the previous system stays in the boot
menu, so the upgrade can be rolled back.

## What changes

| | 1.0 | 1.1 |
|---|---|---|
| PID 1 / services | systemd | **r6** (`raze.init = "r6"`, the new default) |
| Default desktop | niri | **Rift + Rift Shell** (only for new installs; see below) |
| Logging | journald | per-service logs: `r6 logs <service>`, `r6 logs syslog` |
| Service control | `systemctl` | `r6 status`, `r6 start/stop/restart`, `r6 logs` |

Your configuration keeps its `raze.desktop` (e.g. `"niri"`): the upgrade does not
switch desktops for you. niri still works under r6.

## Steps

```bash
# 1. Keep the 1.0 tree (for rollback and reference).
sudo mv /etc/nixos/raze /etc/nixos/raze-1.0.0

# 2. Fetch the 1.1.0 tree.
curl -L https://github.com/SwashX-dev/Raze-Linux/archive/refs/tags/v1.1.0.tar.gz | sudo tar -xz -C /etc/nixos
sudo mv /etc/nixos/Raze-Linux-1.1.0 /etc/nixos/raze

# 3. Re-lock the system flake to the new tree.
cd /etc/nixos && sudo nix flake update raze

# 4. Optional: edit /etc/nixos/configuration.nix
#    - remove `raze.r6.bootEntry.enable = ...;` (no effect any more, prints a warning)
#    - raze.desktop = "rift";   to switch to Rift + Rift Shell
#    - raze.init = "systemd";   to stay on systemd for now

# 5. Build the new system and make it the boot default, then reboot into it.
sudo rix system boot
sudo reboot
```

Use `boot` rather than `switch` for this upgrade: the running system's PID 1
(systemd) cannot be replaced by r6 without a reboot. After the reboot,
`cat /proc/1/comm` prints `r6-init` and `fastfetch` shows *Raze Linux 1.1.0*.

EFI systems installed with 1.0 keep systemd-boot as their boot loader (only new
installations use GRUB); nothing to do.

## Rolling back

- At the boot menu, choose the previous generation (1.0, systemd).
- Or from the running 1.1 system: `sudo rix system rollback`. The previous generation
  uses systemd as PID 1, so rix makes it the boot default instead of activating it in
  place, and asks you to reboot.
- To return to the 1.0 tree permanently:
  `sudo rm -rf /etc/nixos/raze && sudo mv /etc/nixos/raze-1.0.0 /etc/nixos/raze &&
  cd /etc/nixos && sudo nix flake update raze && sudo rix system boot`.

## Recovery

If the 1.1 system does not reach the login screen:

1. Pick the 1.0 generation in the boot menu: it is unchanged.
2. Or add `r6.emergency` to the kernel command line for a root shell before services
   start; `r6 status` and `r6 logs <service>` show what failed.
3. Report the problem with `/var/log/r6/` attached.

See also [recovery.md](recovery.md).
