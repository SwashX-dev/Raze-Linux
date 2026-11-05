# Rix — package management

Rix is a Rust front-end for Nix-compatible package management. **It is not a Nix
reimplementation.** Evaluation, building, substitution from binary caches, profile
changes and garbage collection are delegated to Nix, and every delegated command is
printed (`rix: → nix profile add nixpkgs#hello`) so nothing is hidden. The parts
that are implemented natively in Rust are verified against Nix itself.

## Native (Rust)

| Feature | Command | Verified by |
|---|---|---|
| Nix base-32 encoding/decoding | — | unit tests with vectors produced by `nix hash file` |
| NAR serialisation + SHA-256 (`narHash`) | `rix hash path` | integration test vs `nix hash path`; VM test on store paths |
| Flat file hashing | `rix hash file [--sri]` | vectors from `nix hash file` |
| Store path validation, name/version split | `rix verify`, `rix list` | unit tests |
| Store verification (re-hash vs database) | `rix verify PATH…` | VM test; matches on real store paths |
| Profile generations, atomic rollback/switch | `rix generations`, `rix rollback [--to N]`, `rix system generations`, `rix system rollback` | unit tests on synthetic profiles; installed-system run |
| `nix profile` manifest (v1–v3) reader | `rix list` | unit tests; installed-system run |
| flake.lock (v5–v7) reader | `rix flake info` | unit tests |
| Package search over a cached index | `rix search`, `rix info` | unit tests (ranking); see benchmarks |
| Setup diagnostics | `rix doctor` | — |

Rollback performs the same on-disk operation as `nix-env --rollback` (a new
symlink renamed over the profile link), so Nix and Rix can be used interchangeably.
`rix system rollback` switches the system profile and then runs the target
generation's own `switch-to-configuration switch`, exactly like `nixos-rebuild
--rollback`.

## Delegated to Nix

| Command | Runs |
|---|---|
| `rix install PKG…` | `nix profile add nixpkgs#PKG…` (unknown names are rejected with suggestions from the index before Nix is invoked) |
| `rix remove PKG…` | `nix profile remove PKG…` |
| `rix upgrade` | `nix profile upgrade --all` |
| `rix update` | builds the search index (`nix search --json nixpkgs ^`) for the pinned nixpkgs |
| `rix system switch\|boot\|test` | `nixos-rebuild … --flake /etc/nixos#raze` |
| `rix system update` / `rix update --system` | `nix flake update --flake /etc/nixos` |
| `rix build\|develop\|shell\|run` | `nix build\|develop\|shell\|run` |
| `rix gc [--older-than 30d]` | `nix-collect-garbage [--delete-older-than 30d]` |

## Pinned package source

Raze systems pin the `nixpkgs` flake registry entry and `NIX_PATH` to the exact
nixpkgs the system was built from, so `rix install` resolves against the same
revision (reproducible, and mostly served from the local store and
cache.nixos.org binaries).

## Search index

`nix search nixpkgs foo` evaluates nixpkgs on each query. Rix evaluates once per
nixpkgs revision (keyed by its narHash), stores a compact tab-separated index in
`~/.cache/rix/`, and answers queries by scanning it natively, ranking exact names
before prefixes, substrings and description matches. Results come from the same
evaluation as `nix search`; only the query path differs. See
[benchmarks](../performance/benchmarks.md) for measured numbers.

## Roadmap

Native evaluation (a Nix language evaluator), native substitution (narinfo +
parallel NAR downloads) and a native daemon protocol client are long-term goals.
None of them is claimed for 1.0.
