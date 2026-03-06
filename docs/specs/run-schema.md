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
| `workload` | `string` | no | Workload name (e.g., `"stub"`) |
| `scenario` | `string` | no | Scenario name: `"honest"` or `"dishonest"` |
| `timestamp` | `string` | no | RFC 3339 timestamp of when the run started |
| `raster_pin` | `object` | no | Raster version pin (see below) |
| `steps` | `array` | no | Ordered list of `StepOutput` objects |
| `summary` | `object` | no | Aggregate metrics (see below) |

## `raster_pin` object

| Field | Type | Nullable | Description |
|---|---|---|---|
| `revision` | `string` | no | Git commit SHA or `"stub"` when no real Raster version is pinned |

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
| `exec` | Execute | `pending` | Raster program execution (not applicable in stub phase) |
| `trace` | Trace | `pending` | Trace generation (not applicable in stub phase) |
| `da` | DA Submission | `pending` | Data availability submission (not applicable in stub phase) |
| `claim` | Submit Claim | `done` | On-chain claim submission via `submitClaim()` |
| `replay` | Replay | `done` | Replay verification step |
| `outcome` | Outcome | `settled`, `slashed` | Final on-chain settlement or slashing outcome |

### Step metrics by key

**`claim` metrics:**
- `Claim ID` — on-chain claim identifier
- `Tx hash` — transaction hash
- `Gas used` — gas consumed by `submitClaim()`
- `Artifact root` — submitted artifact root (hex)
- `Result root` — submitted result root (hex)

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

Nullable fields are serialized as JSON `null` when not applicable (e.g., Raster-only fields in stub workloads).

## File naming convention

Run files are written to `runs/` with the naming pattern:

```
runs/<ISO-timestamp>-<workload>-<scenario>.json
```

Example: `runs/2026-03-06T12-00-00-stub-honest.json`

## Compatibility notes

- This schema is the contract between `apps/runner` (producer) and `web/scenario-runner/index.html` (consumer).
- Phase 3 will add `apps/web-server` as an intermediary that serves run files over HTTP/SSE. The schema must remain stable across that transition.
- When Raster core integration is added, the `exec`, `trace`, and `da` steps will transition from `"pending"` to `"done"` with populated metrics. The nullable summary fields will be populated with real values.
