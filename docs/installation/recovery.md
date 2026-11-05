# Recovery

## Boot an older generation

Every `rix system switch` / `nixos-rebuild` creates a new system generation. At the
boot menu (systemd-boot: press a key during the 2 s timeout; GRUB: menu) choose an
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

- systemd entry: add `systemd.unit=rescue.target` to the kernel command line.
- r6 entry: add `r6.emergency` to get a root shell before services start; `r6-init`
  also opens an emergency shell when the supervisor fails five times within a
  minute.

## Software-rendered session

If the graphical session is black or very slow, force a mode at the greeter by
changing the command (F2) to `env RAZE_SESSION=software raze-session` or
`env RAZE_SESSION=niri raze-session`.

## Installer logs

`/var/log/raze-install.log` on the installed disk, and
`/root/.cache/calamares/session.log` in the live session.
