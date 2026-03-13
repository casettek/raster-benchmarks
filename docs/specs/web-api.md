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
| `workload` | string | `"stub"`    | Workload name (`"raster-hello"` enables real exec + trace + DA publication) |
| `scenario` | string | `"honest"`  | `"honest"` or `"dishonest"` |

**Response:** `text/event-stream`

**Concurrency:** Only one run at a time. Concurrent requests return `409 Conflict`.

#### SSE event types

**`step`** — Emitted for each pipeline step as it progresses.

```
event: step
data: {"key":"<step_key>","label":"<label>","status":"<status>","metrics":{...}}
```

Step keys are emitted in order: `exec`, `trace`, `da`, `claim`, `replay`, `outcome`.

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

#### Step emission sequence

1. `exec`, `trace`, `da` are emitted first as `status: "pending"` placeholders.
   - For `workload=raster-hello`, each step is promoted to `"running"` when active, then re-emitted as `status: "done"` with real metrics.
   - For `workload=stub`, they remain `"pending"`.
2. `claim` emitted as `"running"`, then re-emitted as `"done"` with metrics after `submitClaim` tx.
3. `replay` emitted as `"running"`, then re-emitted as `"done"` after rerun-first challenger audit.
4. `outcome` emitted as `"settled"` or `"slashed"` with final metrics.
5. `done` event with full `RunOutput`.

`claim` metrics include trace pointer fields (`Trace tx hash`, `Trace payload bytes`, `Trace codec id`).
For `stub` paths these values are zeroed (`0x00..00`, `0`, `0`).

Replay behavior is rerun-first:

- Challenger fetches claim metadata, replays locally from claim workload inputs, and compares local roots against claim roots.
- If replay matches, no trace payload fetch is attempted and outcome resolves via settlement.
- If replay diverges, challenger conditionally fetches and decodes the trace payload from the trace publication tx pointer, then resolves via challenge/slash in dishonest simulation mode.

Replay and outcome metrics now include divergence context (`Reason`, `Trace fetch`, optional `First divergence index`) and proof status (`Proof status = not-generated` until fraud-proof generation is implemented).

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

`:id` matches the run filename stem (e.g., `2026-03-06T12-00-00-stub-honest`).

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

The scenario runner UI at `/scenario-runner/` auto-detects whether it's served over HTTP (live mode with SSE) or opened from the filesystem (stub mode with `setTimeout` animation).
