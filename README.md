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

### 3. Run live from the browser

Start the web server (spawns Anvil, deploys the contract, serves the API + UI):

```bash
cargo run -p web-server
```

Note: first startup may take longer because the server warms Raster workload binaries once.

Open `http://localhost:8010/scenario-runner/` in a browser. Select a scenario (honest/dishonest) and click **Run**. Steps stream in real time via SSE as the on-chain pipeline executes.

Completed runs appear in the **Past Runs** tab. Select two runs and click **Compare** for side-by-side metrics.

### 4. Run from the CLI (alternative)

```bash
cargo run -p runner -- --scenario honest --workload raster-hello
cargo run -p runner -- --scenario dishonest --workload raster-hello
```

Each run writes a JSON file to `runs/` with the full step-by-step results.

### 5. Environment variables

| Variable    | Default | Description |
|-------------|---------|-------------|
| `PORT`      | `8010`  | Web server listen port |
| `ANVIL_URL` | (none)  | Connect to an external Anvil instead of spawning one |

## Project layout

```
apps/
  claimer/       Standalone claim submission binary
  challenger/    Standalone settle/challenge binary
  runner/        Orchestrator — chains claimer + challenger, writes run JSON
  web-server/    API server — Anvil lifecycle, SSE run streaming, run history
contracts/       Solidity contracts (ClaimVerifier) + Foundry config
crates/shared/   Shared library — EVM bindings, Anvil helpers, run types
docs/            Specs and setup docs
runs/            Run output JSON files and golden fixtures (`runs/fixtures/`)
web/             Static HTML tools (scenario runner, settlement estimator)
```

## API

The web server exposes:

- `GET /api/run?workload=raster-hello&scenario=honest` — SSE stream of run progress
- `GET /api/runs` — JSON array of all past runs (newest first)
- `GET /api/runs/:id` — single run by ID (or 404)

See `docs/specs/web-api.md` for the full specification.
