# Web API Specification

The web server (`apps/web-server`) manages an Anvil lifecycle and exposes a REST + SSE API for executing runs and querying run history.

## Server lifecycle

On startup the web server:

1. Warms known Raster workloads once (build-if-missing) to avoid first-run compile stalls.
2. Spawns a local Anvil instance (or connects to an external one via `ANVIL_URL`).
3. Verifies the `ClaimVerifier` contract can be deployed from Foundry artifacts.
4. Serves static files from `web/` and the API routes below.

Each run redeploys `ClaimVerifier` for clean chain state (no claim ID conflicts between runs).

### Environment variables

| Variable    | Default | Description |
|-------------|---------|-------------|
| `PORT`      | `8010`  | HTTP listen port |
| `ANVIL_URL` | (none)  | If set, connect to an external Anvil instance instead of spawning one |

### Prerequisites

- `forge build` must have been run in `contracts/` to produce `contracts/out/ClaimVerifier.sol/ClaimVerifier.json`.
- `anvil` must be on `PATH` (or `ANVIL_URL` must point to a running instance).

## Endpoints

### `GET /api/run` — Execute a run (SSE)

Executes the full claim/challenge pipeline and streams progress via Server-Sent Events.

Uses GET with query parameters so the browser `EventSource` API can connect directly.

**Query parameters:**

| Param      | Type   | Default     | Description |
|------------|--------|-------------|-------------|
| `workload` | string | `"l2-kona-poc"` | Workload name (`"raster-hello"` or `"l2-kona-poc"`) |
| `scenario` | string | `"honest"`  | `"honest"` or `"dishonest"` |

**Response:** `text/event-stream`

**Concurrency:** Only one run at a time. Concurrent requests return `409 Conflict`.

#### SSE event types

**`step`** — Emitted for each pipeline step as it progresses.

```
event: step
data: {"key":"<step_key>","label":"<label>","status":"<status>","metrics":{...}}
```

Step keys depend on the workload (see `docs/specs/run-schema.md` for full lifecycle definitions):

- **Legacy** (`raster-hello`): `exec`, `trace`, `da`, `claim`, `replay`, `outcome`
- **L2** (`l2-kona-poc`): `prepare`, `exec`, `da`, `claim`, `audit`, `await-finalization`, `outcome`

Status values: `"pending"`, `"running"`, `"done"`, `"settled"`, `"slashed"`.

A step may be emitted multiple times (e.g., `claim` emitted first as `"running"`, then as `"done"` with metrics).

**`done`** — Emitted once after all steps complete.

```
event: done
data: {"run_id":"<id>","file":"runs/<filename>.json","run":<RunOutput>}
```

The `run` field contains the full `RunOutput` object (see `docs/specs/run-schema.md`).

**`error`** — Emitted if the pipeline fails at any point.

```
event: error
data: {"message":"<error text>"}
```

#### Step emission sequence — legacy lifecycle (`raster-hello`)

1. `exec`, `trace`, `da` are emitted first as `status: "pending"` placeholders.
   - Each step is promoted to `"running"` when active, then re-emitted as `status: "done"` with real metrics.
2. `claim` emitted as `"running"`, then re-emitted as `"done"` with metrics after `submitClaim` tx.
3. `replay` emitted as `"running"`, then re-emitted as `"done"` after rerun-first challenger audit.
4. `outcome` emitted as `"settled"` or `"slashed"` with final metrics.
5. `done` event with full `RunOutput`.

#### Step emission sequence — L2 lifecycle (`l2-kona-poc`)

1. `prepare`, `exec`, `da` emitted first as `status: "pending"` placeholders.
2. `prepare` promoted to `"running"`, then `"done"` with batch metadata (fixture name, batch hash, block range, input blob tx/versioned-hash metadata, and input manifest registration block/timestamp).
3. `exec` promoted to `"running"`, then `"done"` with execution metrics. No separate `trace` step — trace artifacts and trace-commitment metadata are folded into exec.
4. `da` promoted to `"running"`, then `"done"` with DA publication metrics for both the input-package artifact and `trace.commitment.json`, including the manifest registration block/timestamp for each claim-bound manifest blob.
5. `claim` emitted as `"running"`, then `"done"` with full L2 claim metadata (prevOutputRoot, nextOutputRoot, startBlock, endBlock, batchHash, bond amount, challenge deadline, input blob hash, trace blob hash, and manifest registration metadata).
6. `audit` emitted as `"running"`, then `"done"` with local replay results (replay time, divergence status, input fetch status, trace fetch status).
7. `await-finalization` emitted as `"running"` with challenge deadline and period metrics, then `"done"` with terminal status text.
8. `outcome` emitted as `"settled"` or `"slashed"` with final metrics.
9. `done` event with full `RunOutput` including L2 summary metadata.

#### Claim and replay behavior

`claim` metrics include blob publication identifiers (`Input blob tx hash`, `Input blob versioned hash`, `Trace blob tx hash`, `Trace blob versioned hash`), manifest registration metadata (`... registered block`, `... registered at`), and full L2 claim metadata for `l2-kona-poc` runs.

Replay/audit behavior is rerun-first:

- Challenger fetches claim metadata, fetches the input-package artifact and trace-commitment artifact from Anvil blob storage, materializes the input package, reruns the workload locally, and compares the fresh trace commitment against the published commitment artifact.
- Challenger also compares the locally replayed `nextOutputRoot` against the claimed root.
- The settlement contract independently rejects claims that reference unregistered or stale manifest blob hashes before they ever reach challenger audit.
- If replay and commitment match, outcome resolves via settlement.
- If divergence is detected, challenger resolves via `challengeClaim` when the locally observed `nextOutputRoot` differs from the claim.

Replay, audit, and outcome metrics include divergence context (`Reason`, `Input fetch`, `Trace fetch`, optional `First divergence index`) and proof status (`Proof status = not-generated` until fraud-proof generation is implemented).

### `GET /api/runs` — List all runs

Returns a JSON array of all `RunOutput` objects from `runs/*.json`, sorted by filename descending (newest first).

**Response:** `200 OK` with `application/json`

```json
[
  { "id": "...", "workload": "...", "scenario": "...", ... },
  ...
]
```

Returns `[]` if no run files exist.

### `GET /api/runs/:id` — Get a single run

`:id` matches the run filename stem (e.g., `2026-03-06T12-00-00-l2-kona-poc-honest`).

**Response:**
- `200 OK` with the `RunOutput` JSON if found.
- `404 Not Found` if no matching run file exists.

## Run file naming

Files are written to `runs/<id>.json` where `id` follows the format:

```
<YYYY-MM-DDTHH-MM-SS>-<workload>-<scenario>
```

Same convention as the CLI runner (`apps/runner`).

## Static file serving

All files under `web/` are served at the root path. API routes take precedence over static files.

The scenario runner UI at `/scenario-runner/` requires the web server to be running (live mode with SSE). When opened from the filesystem, it displays past runs from localStorage but cannot execute new runs.

For L2 runs (`l2-kona-poc`), the UI renders the expanded lifecycle with `Prepare Batch`, `Audit`, and `Await Finalization` steps. The `Await Finalization` step shows a live countdown timer derived from the challenge deadline and challenge period metrics emitted during the `await-finalization` running state.
