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
- Blob-backed local dev is pinned against Foundry/Anvil `1.5.1-stable`
  (`b0a9dd9ceda36f63e2326ce530c10e6916f4b8a2`) with Anvil started using
  `--hardfork cancun`.
- On Linux environments without a full clang header setup, rebuilding
  `workload-l2-kona-poc` may require:
  `BINDGEN_EXTRA_CLANG_ARGS='-I/usr/lib/gcc/x86_64-linux-gnu/13/include -I/usr/include/x86_64-linux-gnu -I/usr/include'`
  so `librocksdb-sys` bindgen can find builtin C headers like `stdbool.h`.

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
5. Build all Rust crates (from repo root):
   - `cargo build --workspace`
6. Run the claimer (spawns its own Anvil, deploys contract, submits a claim):
   - `cargo run -p claimer-app`
7. Or with an external Anvil instance:
   - Start Anvil: `anvil`
   - In another shell: `ANVIL_URL=http://127.0.0.1:8545 cargo run -p claimer-app`
8. To serve the web interfaces locally:
   - `cargo run -p web-server`
   - Then open: http://localhost:8010

## Cargo workspace

The repo uses a Cargo workspace rooted at `Cargo.toml`. All apps and shared crates are members:

- `apps/claimer` — submits claims to ClaimVerifier on a local Anvil chain
- `apps/challenger` — (stub) will challenge fraudulent claims
- `apps/web-server` — static file server for web tools
- `crates/shared` — alloy contract bindings, Anvil helpers, deploy logic, run output types

Build everything: `cargo build --workspace`
Check everything: `cargo check --workspace`
Lint everything: `cargo clippy --workspace --all-targets`

## Reproducibility requirement

Given the same:

- Raster pin (`repository`, `revision`, `workspace_lock_hash`)
- workload/scenario identifiers
- local chain engine/version (`anvil`)

the run must produce stable structural outputs suitable for deterministic regression comparison.
