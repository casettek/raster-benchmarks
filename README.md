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

Open `http://localhost:8010/scenario-runner/` in a browser. Select a workload (`l2-kona-poc` or `raster-hello`), a scenario (honest/dishonest), and click **Run**. Steps stream in real time via SSE as the on-chain pipeline executes.

Completed runs appear in the **Past Runs** tab. Select two runs and click **Compare** for side-by-side metrics.

### 4. Run from the CLI (alternative)

```bash
cargo run -p runner -- --scenario honest --workload raster-hello
cargo run -p runner -- --scenario dishonest --workload raster-hello
```

Each run writes a JSON file to `runs/` with the full step-by-step results.
Raster workload runs also persist raw trace artifacts plus a compact
`trace.commitment.json` sidecar under `runs/artifacts/<run-id>/`.

### 5. Run the L2 Kona POC demo

The `l2-kona-poc` workload demonstrates the full L2 optimistic settlement lifecycle:
canonical batch preparation, chunked tile execution (10 tiles), blob-carrying claim
submission, audit replay, challenge-period countdown, and terminal finalization or rejection.

```bash
# Honest path — claim settles after challenge period
cargo run -p runner -- --scenario honest --workload l2-kona-poc

# Dishonest path — claim is challenged and slashed before deadline
cargo run -p runner -- --scenario dishonest --workload l2-kona-poc
```

The L2 lifecycle uses an expanded step sequence:

1. **Prepare Batch** — loads the canonical synthetic fixture and identifies the batch
2. **Execute Program** — runs the Raster program (10 chunked tile invocations)
3. **Publish to DA** — publishes the compact trace-commitment payload for audit
4. **Submit Claim** — submits the blob-carrying settlement claim with bond
5. **Audit** — independent local replay comparison
6. **Await Finalization** — challenge-period countdown (120s default on Anvil)
7. **Outcome** — terminal `Settled` or `Slashed` state

The web UI (`/scenario-runner/`) shows a live countdown timer during the `Await Finalization` step and displays full L2 claim metadata (block range, output roots, batch hash, bond amount) in the summary panel.

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

- `GET /api/run?workload=l2-kona-poc&scenario=honest` — SSE stream of L2 run progress
- `GET /api/run?workload=raster-hello&scenario=honest` — SSE stream of run progress
- `GET /api/runs` — JSON array of all past runs (newest first)
- `GET /api/runs/:id` — single run by ID (or 404)

See `docs/specs/web-api.md` for the full specification.
