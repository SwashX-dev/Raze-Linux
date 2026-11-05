# Contributing to Raze Linux

Thanks for helping. Raze values correctness, reproducibility and measured
performance over feature count.

1. Open an issue first for anything larger than a small fix.
2. Work on a branch; keep commits focused and use conventional prefixes
   (`feat(r6):`, `fix(rix):`, `perf(kernel):`, `docs:`, `test(iso):`).
3. Before opening a pull request:
   ```bash
   nix develop -c cargo fmt --all --check
   nix develop -c cargo clippy --workspace --all-targets -- -D warnings
   nix develop -c cargo test --workspace
   nix build .#checks.x86_64-linux.core-boot .#checks.x86_64-linux.desktop
   ```
4. Performance claims need numbers: describe the method, the hardware or VM, and
   include before/after measurements (see `docs/performance/methodology.md`).
5. Do not describe delegated or upstream functionality as native Raze work.

See [docs/development](docs/development) for building and testing details.
