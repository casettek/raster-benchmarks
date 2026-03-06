# Local Setup (Deterministic Baseline)

This repo is local-first. Phase 0 requires deterministic local prerequisites before benchmark development.

## Required tools

- Rust stable toolchain (`rustup`, `cargo`, `rustc`)
- Foundry toolchain (`forge`, `anvil`)
- `git`

## Version pinning expectations

- Record `rustc --version` used for benchmark runs.
- Record `forge --version` used for contract build/test runs.
- Record `anvil --version` used for benchmark runs.
- Record exact Raster revision SHA and lockfile hash in run metadata (`raster_pin` block).

## Baseline startup check

1. Verify Rust toolchain:
   - `rustc --version`
   - `cargo --version`
2. Verify local chain runtime:
   - `anvil --version`
3. Verify contract toolchain:
   - `forge --version`
4. Build local contracts:
   - `cd contracts && forge build`
5. Start local chain:
   - `anvil`
6. In another shell, run starter app entrypoints:
    - `cargo run --manifest-path apps/claimer/Cargo.toml`
    - `cargo run --manifest-path apps/challenger/Cargo.toml`
7. To serve the web interfaces locally:
    - `cargo run --manifest-path apps/web-server/Cargo.toml`
    - Then open: http://localhost:8010

## Reproducibility requirement

Given the same:

- Raster pin (`repository`, `revision`, `workspace_lock_hash`)
- workload/scenario identifiers
- local chain engine/version (`anvil`)

the run must produce stable structural outputs suitable for deterministic regression comparison.
