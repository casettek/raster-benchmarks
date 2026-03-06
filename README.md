# Raster Benchmarks

End-to-end benchmark harness for the Raster optimistic coprocessing protocol. Runs on-chain claim/challenge scenarios against a local Anvil instance with real gas numbers and tx receipts.

## Prerequisites

- [Rust](https://rustup.rs/) (stable)
- [Foundry](https://book.getfoundry.sh/getting-started/installation) (`forge`, `anvil`)

## Quick start

### 1. Build the contracts (once, or after Solidity changes)

```bash
cd contracts && forge build && cd ..
```

### 2. Build everything

```bash
cargo build --workspace
```

### 3. Run a scenario

```bash
cargo run -p runner -- --scenario honest
cargo run -p runner -- --scenario dishonest
```

Each run writes a JSON file to `runs/` with the full step-by-step results.

### 4. Check the output

Open the JSON file in `runs/` — it contains 6 steps (`exec`, `trace`, `da`, `claim`, `replay`, `outcome`) with statuses and metrics. The Raster-only steps (`exec`, `trace`, `da`) are `"pending"` placeholders until Raster core integration lands.

### 5. Visual check (optional)

In a second terminal, start the web server:

```bash
cargo run -p web-server
```

Open `http://localhost:8010/scenario-runner/` in a browser. Scroll down to **Import Run JSON**, paste the contents of a run JSON file from `runs/`, and click **Import**. The run will load into the lifecycle view and get saved to your past runs.

## Project layout

```
apps/
  claimer/       Standalone claim submission binary
  challenger/    Standalone settle/challenge binary
  runner/        Orchestrator — chains claimer + challenger, writes run JSON
  web-server/    Static file server for web/ directory
contracts/       Solidity contracts (ClaimVerifier) + Foundry config
crates/shared/   Shared library — EVM bindings, Anvil helpers, run types
docs/            Specs and setup docs
runs/            Run output JSON files (gitignored contents)
web/             Static HTML tools (scenario runner, settlement estimator)
```

## What's next

Phase 3 will add a `POST /api/run` endpoint to the web server so you can trigger scenarios from the UI and see results stream in live via SSE — no more manual copy-paste.
