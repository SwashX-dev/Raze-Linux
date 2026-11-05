# Recovery

## Boot an older generation

Every `rix system switch` / `nixos-rebuild` creates a new system generation. At the
boot menu (GRUB; systemd-boot on EFI systems installed with 1.0: press a key during the 2 s timeout) choose an
older entry. To make it the default again:

```bash
sudo rix system rollback          # previous generation, activated immediately
sudo rix system rollback --to 12  # a specific generation
rix system generations            # list them
```

## Undo a package change

```bash
rix generations       # user profile generations
rix rollback          # back to the previous one (atomic)
```

## Broken configuration

Edit `/etc/nixos/configuration.nix` from an older generation (or the live ISO,
after mounting the disk at `/mnt`) and rebuild:

```bash
sudo rix system switch
# from the live ISO:
sudo nixos-install --root /mnt --flake /mnt/etc/nixos#raze --no-channel-copy
```

## Emergency shell

- Add `r6.emergency` to the kernel command line (`e` in the GRUB menu) for a root
  shell before services start; `r6-init` also opens an emergency shell when the
  supervisor fails five times within a minute. `r6 status` and `r6 logs <service>`
  show what failed.
- Systems with `raze.init = "systemd"`: `systemd.unit=rescue.target`.

## Graphical session problems

- Rift picks its renderer automatically (GLES with a 3D-capable GPU, pixman
  otherwise). To force software rendering, change the greeter command (F2) to
  `env RIFT_RENDERER=pixman raze-session`. Rift's log is
  `~/.local/state/rift/rift.log` (`rift.log.old`: the previous session);
  `rift msg config-error` reports configuration problems.
- Rift Shell missing (no panel): `r6 --user status rift-shell`, `r6 --user restart rift-shell`.
- niri sessions: `env RAZE_SESSION=software raze-session` forces the sway/pixman
  fallback.

## Installer logs

`/var/log/raze-install.log` on the installed disk, and
`/root/.cache/calamares/session.log` in the live session.
