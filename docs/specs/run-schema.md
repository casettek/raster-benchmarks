# Run Output Schema

Canonical JSON schema for `runs/*.json` run output files produced by `apps/runner`. This is the data contract between the runner and the scenario runner UI (`web/scenario-runner/index.html`).

## Ownership

- Defined by: `crates/shared/src/run.rs` (Rust source of truth)
- Produced by: `apps/runner`
- Consumed by: `web/scenario-runner/index.html`, future `apps/web-server` API

## Top-level structure

| Field | Type | Nullable | Description |
|---|---|---|---|
| `id` | `string` | no | Run identifier, format: `<ISO-timestamp>-<workload>-<scenario>` |
| `workload` | `string` | no | Workload name (e.g., `"stub"`, `"raster-hello"`) |
| `scenario` | `string` | no | Scenario name: `"honest"` or `"dishonest"` |
| `timestamp` | `string` | no | RFC 3339 timestamp of when the run started |
| `raster_pin` | `object` | no | Raster version pin (see below) |
| `steps` | `array` | no | Ordered list of `StepOutput` objects |
| `summary` | `object` | no | Aggregate metrics (see below) |

## `raster_pin` object

| Field | Type | Nullable | Description |
|---|---|---|---|
| `revision` | `string` | no | Pinned Raster dependency revision (full git SHA for local `../raster` integration, or `"stub"` for placeholder workloads) |

## `StepOutput` object

Each step in the `steps` array has:

| Field | Type | Nullable | Description |
|---|---|---|---|
| `key` | `string` | no | Step identifier, one of: `exec`, `trace`, `da`, `claim`, `replay`, `outcome` |
| `label` | `string` | no | Human-readable step label |
| `status` | `string` | no | Step status (see below) |
| `metrics` | `object` | no | Key-value string pairs of step-specific metrics (may be empty) |

### Step keys and expected statuses

| Key | Label | Status values | Description |
|---|---|---|---|
| `exec` | Execute | `pending`, `done` | Raster program execution (`done` for real Raster workloads) |
| `trace` | Trace | `pending`, `done` | Trace generation (`done` when trace artifacts are emitted) |
| `da` | DA Submission | `pending`, `done` | Trace data publication step (`done` for workloads that publish DA payload) |
| `claim` | Submit Claim | `done` | On-chain claim submission via `submitClaim()` |
| `replay` | Replay | `done` | Replay verification step |
| `outcome` | Outcome | `settled`, `slashed` | Final on-chain settlement or slashing outcome |

### Step metrics by key

**`exec` metrics (`status = done`):**
- `Workload` — executed workload identifier
- `Exec time (ms)` — workload execution duration in milliseconds
- `Trace steps` — number of trace step records captured from workload execution

**`trace` metrics (`status = done`):**
- `Trace size (bytes)` — serialized trace payload size (NDJSON bytes)
- `Trace file` — relative path to persisted trace artifact JSON
- `Raster revision` — pinned Raster dependency revision used for the run

**`da` metrics (`status = done`):**
- `Blob tx hash` - publication transaction hash used as claim trace pointer
- `Payload bytes` - published trace payload size in bytes
- `Codec id` - trace codec discriminator (`1` = `trace.ndjson` v1)
- `Gas used` - gas consumed by the DA publication tx
- `Payload hash` - keccak256 hash emitted by `TracePublished`

**`claim` metrics:**
- `Claim ID` — on-chain claim identifier
- `Tx hash` — transaction hash
- `Gas used` — gas consumed by `submitClaim()`
- `Artifact root` — submitted artifact root (hex)
- `Result root` — submitted result root (hex)
- `Trace tx hash` — pointer to the DA publication tx hash (`0x00..00` when unset)
- `Trace payload bytes` — pointer payload byte size (`0` when unset)
- `Trace codec id` — pointer codec id (`0` when unset)

**`replay` metrics:**
- `Replay time` — replay duration or `"n/a"` for stub
- `Divergence` — `"None"` (honest) or `"Detected"` (dishonest)

**`outcome` metrics (honest):**
- `Tx hash` — settlement transaction hash
- `Gas used` — gas consumed by `settleClaim()`
- `Final state` — `"Settled"`

**`outcome` metrics (dishonest):**
- `Tx hash` — challenge transaction hash
- `Gas used` — gas consumed by `challengeClaim()`
- `Final state` — `"Slashed"`
- `Claimer artifact root` — original claimer's artifact root (hex)
- `Claimer result root` — original claimer's result root (hex)
- `Observed artifact root` — challenger's divergent artifact root (hex)
- `Observed result root` — challenger's divergent result root (hex)

## `SummaryOutput` object

| Field | Type | Nullable | Description |
|---|---|---|---|
| `exec_time_ms` | `u64` | yes | Raster execution time in milliseconds |
| `trace_size_bytes` | `u64` | yes | Generated trace size in bytes |
| `da_gas` | `u64` | yes | Gas consumed for DA submission |
| `claim_gas` | `u64` | no | Gas consumed by `submitClaim()` transaction |
| `replay_time_ms` | `u64` | yes | Replay verification time in milliseconds |
| `fraud_proof_time_ms` | `u64` | yes | Fraud proof generation time in milliseconds |
| `fraud_proof_gas` | `u64` | yes | Gas consumed by fraud proof verification |
| `total_time_ms` | `u64` | yes | Total end-to-end run time in milliseconds |
| `outcome` | `string` | no | Final outcome: `"settled"` or `"slashed"` |

Nullable fields are serialized as JSON `null` when not applicable (for example, `stub` workload runs).

## File naming convention

Run files are written to `runs/` with the naming pattern:

```
runs/<ISO-timestamp>-<workload>-<scenario>.json
```

Example: `runs/2026-03-06T12-00-00-raster-hello-honest.json`

Real Raster workload runs also persist trace artifacts under:

```
runs/artifacts/<run-id>/trace.json
runs/artifacts/<run-id>/trace.ndjson
```

## Compatibility notes

- This schema is the contract between `apps/runner` (producer) and `web/scenario-runner/index.html` (consumer).
- Phase 3 will add `apps/web-server` as an intermediary that serves run files over HTTP/SSE. The schema must remain stable across that transition.
- When Raster core integration is added, the `exec`, `trace`, and `da` steps will transition from `"pending"` to `"done"` with populated metrics. The nullable summary fields will be populated with real values.
