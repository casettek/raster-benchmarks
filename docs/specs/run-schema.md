# Run Output Schema

Canonical JSON schema for `runs/*.json` run output files produced by `apps/runner`
and `apps/web-server`. This is the data contract for CLI runs, SSE `done` payloads,
and the scenario runner UI (`web/scenario-runner/index.html`).

## Ownership

- Defined by: `crates/shared/src/run.rs` (Rust source of truth)
- Produced by: `apps/runner`, `apps/web-server`
- Consumed by: `apps/web-server` API, `web/scenario-runner/index.html`

## Top-level structure

| Field | Type | Nullable | Description |
|---|---|---|---|
| `id` | `string` | no | Run identifier, format: `<ISO-timestamp>-<workload>-<scenario>` |
| `workload` | `string` | no | Workload name (e.g., `"raster-hello"`, `"l2-kona-poc"`) |
| `scenario` | `string` | no | Scenario name: `"honest"` or `"dishonest"` |
| `timestamp` | `string` | no | RFC 3339 timestamp of when the run started |
| `raster_pin` | `object` | no | Raster version pin (see below) |
| `steps` | `array` | no | Ordered list of `StepOutput` objects |
| `summary` | `object` | no | Aggregate metrics (see below) |

## `raster_pin` object

| Field | Type | Nullable | Description |
|---|---|---|---|
| `revision` | `string` | no | Pinned Raster dependency revision (full git SHA for local `../raster` integration, or `"unknown"` when unavailable) |

## `StepOutput` object

Each step in the `steps` array has:

| Field | Type | Nullable | Description |
|---|---|---|---|
| `key` | `string` | no | Step identifier (see lifecycles below) |
| `label` | `string` | no | Human-readable step label |
| `status` | `string` | no | Step status (see below) |
| `metrics` | `object` | no | Key-value string pairs of step-specific metrics (may be empty) |

### Step lifecycles

The step sequence depends on the workload type.

**Legacy lifecycle** (workload: `raster-hello`):

| Key | Label | Status values | Description |
|---|---|---|---|
| `exec` | Execute | `pending`, `done` | Raster program execution |
| `trace` | Trace | `pending`, `done` | Trace generation |
| `da` | DA Submission | `pending`, `done` | Trace-commitment artifact publication |
| `claim` | Submit Claim | `done` | On-chain claim submission via `submitClaim()` |
| `replay` | Replay | `done` | Replay verification step |
| `outcome` | Outcome | `settled`, `slashed` | Final on-chain settlement or slashing outcome |

**L2 lifecycle** (workload: `l2-kona-poc`):

| Key | Label | Status values | Description |
|---|---|---|---|
| `prepare` | Prepare Batch | `done` | Canonical batch preparation from synthetic fixture |
| `exec` | Execute Program | `pending`, `done` | Raster program execution (10-tile chunked replay) |
| `da` | Publish to DA | `pending`, `done` | Trace-commitment artifact publication to DA layer |
| `claim` | Submit Claim | `done` | Blob-carrying claim submission binding canonical input + claimed output roots |
| `audit` | Audit | `done` | Independent local replay audit with conditional trace fetch |
| `await-finalization` | Await Finalization | `done` | Challenge-period countdown and terminal settlement |
| `outcome` | Outcome | `settled`, `slashed` | Final on-chain finalization or rejection |

Note: the L2 lifecycle does not include a separate `trace` step — trace artifacts are folded into the `exec` step. The `replay` step is replaced by the two-phase `audit` + `await-finalization` sequence.

### Step metrics by key

**`prepare` metrics (L2 only, `status = done`):**
- `Fixture` — canonical fixture filename (`l2-poc-synth-fixture.json`)
- `Batch hash` — keccak256 commitment over tracked transaction bytes (hex)
- `Block range` — `startBlock → endBlock` range covered by the claim

**`exec` metrics (`status = done`):**
- `Workload` — executed workload identifier
- `Exec time (ms)` — native workload runtime in milliseconds (measured from workload binary start until trace emission completes; excludes Cargo build/check and host-side trace artifact persistence)
- `Trace steps` — number of trace step records captured from workload execution
- `Trace commitment` — aggregate trace commitment over postcard-encoded Raster trace records (hex)
- `Trace commitment size (bytes)` — serialized `trace.commitment.json` artifact size
- `Trace commitment file` — relative path to persisted trace-commitment artifact JSON

**`trace` metrics (`status = done`):**
- `Trace size (bytes)` — serialized trace payload size (NDJSON bytes)
- `Trace file` — relative path to persisted trace artifact JSON
- `Trace commitment` — aggregate trace commitment over postcard-encoded Raster trace records (hex)
- `Trace commitment size (bytes)` — serialized `trace.commitment.json` artifact size
- `Trace commitment file` — relative path to persisted trace-commitment artifact JSON
- `Raster revision` — pinned Raster dependency revision used for the run

**`da` metrics (`status = done`):**
- `Blob tx hash` - publication transaction hash used as claim trace pointer
- `Payload bytes` - published trace payload size in bytes
- `Codec id` - trace codec discriminator (`2` = `trace.commitment.json` v1)
- `Gas used` - gas consumed by the DA publication tx
- `Payload hash` - keccak256 hash emitted by `TracePublished`

**`claim` metrics:**
- `Claim ID` — on-chain claim identifier
- `Tx hash` — transaction hash
- `Gas used` — gas consumed by `submitClaim()`
- `prevOutputRoot` — prior agreed output root (hex)
- `nextOutputRoot` — claimed output root after execution (hex)
- `startBlock` — first L2 block in the claimed range
- `endBlock` — last L2 block in the claimed range
- `batchHash` — keccak256 commitment over tracked tx bytes (hex)
- `Bond amount` — claimer bond in wei (decimal string)
- `Challenge deadline` — unix timestamp after which the claim can settle
- `Trace tx hash` — pointer to the DA publication tx hash
- `Trace payload bytes` — pointer payload byte size
- `Trace codec id` — pointer codec id

**`replay` metrics (legacy lifecycle):**
- `Replay time (ms)` — replay duration in milliseconds
- `Divergence` — `"None"` (honest) or `"Detected"` (dishonest)
- `Reason` — deterministic replay/audit reason string
- `Trace fetch` — trace-commitment audit status (`fetched`)
- `First divergence index` — optional first mismatching commitment item index when available

**`audit` metrics (L2 lifecycle):**
- `Replay time (ms)` — local replay duration in milliseconds
- `Divergence` — `"None"` (honest) or `"Detected"` (dishonest)
- `Reason` — deterministic replay/audit reason string
- `Trace fetch` — trace-commitment audit status (`fetched`)
- `First divergence index` — optional first mismatching commitment item index when available

**`await-finalization` metrics (L2 lifecycle):**
- `Challenge deadline` — unix timestamp of the challenge deadline
- `Challenge period (s)` — challenge period duration in seconds
- `Status` — `"Deadline passed — settling"` (honest) or `"Challenged before deadline"` (dishonest)

**`outcome` metrics (honest):**
- `Tx hash` — settlement transaction hash
- `Gas used` — gas consumed by `settleClaim()`
- `Final state` — `"Settled"`
- `Challenge deadline` — the unix timestamp that was waited for

**`outcome` metrics (dishonest):**
- `Tx hash` — challenge transaction hash
- `Gas used` — gas consumed by `challengeClaim()`
- `Final state` — `"Slashed"`
- `Proof status` — currently `"not-generated"`
- `Claimer nextOutputRoot` — original claimer's claimed output root (hex)
- `Observed nextOutputRoot` — challenger's replayed output root (hex)
- `Trace fetch` — whether challenge path fetched the trace payload
- `Trace tx hash` / `Trace payload bytes` — included when trace fetch occurred
- `Challenge deadline` — the deadline that was preempted by the challenge

## `SummaryOutput` object

| Field | Type | Nullable | Description |
|---|---|---|---|
| `exec_time_ms` | `u64` | yes | Raster execution time in milliseconds |
| `trace_size_bytes` | `u64` | yes | Generated trace size in bytes |
| `trace_commitment_size_bytes` | `u64` | yes | Published trace-commitment artifact size in bytes |
| `da_gas` | `u64` | yes | Gas consumed for DA submission |
| `claim_gas` | `u64` | no | Gas consumed by `submitClaim()` transaction |
| `replay_time_ms` | `u64` | yes | Replay verification time in milliseconds |
| `fraud_proof_time_ms` | `u64` | yes | Fraud proof generation time in milliseconds |
| `fraud_proof_gas` | `u64` | yes | Gas consumed by fraud proof verification |
| `proof_status` | `string` | no | Proof pipeline status (`"not-generated"` in current phase) |
| `divergence` | `object` | yes | Structured divergence report from replay audit (see below) |
| `total_time_ms` | `u64` | yes | Total end-to-end run time in milliseconds |
| `outcome` | `string` | no | Final outcome: `"settled"` or `"slashed"` |
| `prev_output_root` | `string` | yes | Prior agreed OP output root (hex). L2 only. |
| `next_output_root` | `string` | yes | Claimed OP output root after execution (hex). L2 only. |
| `start_block` | `u64` | yes | First L2 block in the claimed range. L2 only. |
| `end_block` | `u64` | yes | Last L2 block in the claimed range. L2 only. |
| `batch_hash` | `string` | yes | keccak256 over tracked tx bytes (hex). L2 only. |
| `input_blob_tx_hash` | `string` | yes | Input-package manifest publication tx hash (hex). L2 only. |
| `input_blob_versioned_hash` | `string` | yes | Input-package manifest blob versioned hash (hex). L2 only. |
| `trace_blob_tx_hash` | `string` | yes | Trace-commitment manifest publication tx hash (hex). |
| `trace_blob_versioned_hash` | `string` | yes | Trace-commitment manifest blob versioned hash (hex). |
| `bond_amount` | `string` | yes | Claimer bond amount in wei (decimal string). L2 only. |
| `challenge_deadline` | `u64` | yes | Challenge deadline as unix timestamp. L2 only. |
| `challenge_period_seconds` | `u64` | yes | Challenge period duration in seconds. L2 only. |

Nullable fields are serialized as JSON `null` when not applicable. L2 claim metadata fields are omitted entirely (via `skip_serializing_if`) for non-L2 workloads.

## `divergence` object

| Field | Type | Nullable | Description |
|---|---|---|---|
| `detected` | `bool` | no | Whether replay output diverged from claimed output |
| `reason` | `string` | no | Human-readable replay/audit decision reason |
| `first_divergence_index` | `u64` | yes | First divergence index when trace localization is available |
| `trace_fetch_status` | `string` | no | Trace-blob fetch status (`"fetched"` in the current blob-backed flow) |
| `input_fetch_status` | `string` | yes | Input-package fetch status for L2 audit |
| `input_blob_versioned_hash` | `string` | yes | Input-package manifest blob versioned hash used for audit |
| `trace_blob_versioned_hash` | `string` | yes | Trace-commitment manifest blob versioned hash used for audit |

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
runs/artifacts/<run-id>/trace.commitment.json
```

## Compatibility notes

- This schema must remain stable across both CLI and API/SSE producers.
- All workloads (`raster-hello`, `l2-kona-poc`) produce real execution results. Step statuses are `"done"` with metrics after workload execution completes.
