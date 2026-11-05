# Security policy

## Supported versions

| Version | Supported |
|---|---|
| 1.0.x | Yes |

## Reporting a vulnerability

Please report privately via GitHub: **Security → Report a vulnerability** on
https://github.com/SwashX-dev/Raze-Linux. Do not open public issues for security
problems. You will get an acknowledgement within a week.

## Security-relevant design

- Privilege escalation uses **sudo-rs** (memory-safe); classic sudo is disabled.
  Only `wheel` members may execute it (`execWheelOnly`), `use_pty` and a 5-minute
  timestamp timeout are set.
- The r6 control socket is created mode 0600; only root controls services.
  Service definitions are validated (absolute commands, known dependencies, no
  cycles) before anything starts. All `unsafe` in r6 is confined to `r6/src/sys.rs`.
- Rix verifies store paths by re-hashing them (`rix verify`); downloads and
  signature checks are performed by Nix against cache.nixos.org's public key.
- The installer never erases a disk without the final confirmation dialog.
  The live ISO user has passwordless sudo (installation media only); installed
  systems require the user's password.
- Release ISOs are published with SHA-256 checksums. They are **not** signed in
  1.0.0 because no signing key and secure signing process exist yet.
