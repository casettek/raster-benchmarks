# L2 Kona Workload Spec (Plan 008 / 008.5 / 008.6 / 008.7 phases 1-3)

This spec defines the v1 native Kona adapter boundary and checkpoint contract for
the first L2 POC workload.

## Scope

- One fixed 5-transaction benchmark batch.
- One canonical Raster program that executes the batch as 10 deterministic tile calls.
- One explicit Raster transition that executes the 5 benchmark txs plus any supplemental txs required to close the canonical block.
- One canonical chunk-plan sidecar that partitions the same 10 execution txs into deterministic replay slices (default `chunk_size = 1`).
- Native-only execution path (no `Risc0` guest/host flow in this phase).

Out of scope for this spec:

- Settlement contract implementation details.
- Runner/web UX details.
- Fraud-proof object generation and proof verification.

## Current execution boundary

The execution boundary is explicit and serializable:

`preCheckpoint + trackedTxBytes + supplementalTxBytes + outputRootWitness + blockContext -> postCheckpoint + executionArtifacts`

### `preCheckpoint` (required)

- `prev_output_root` (`bytes32`): prior agreed OP `outputRoot`.
- `parent_header_hash` (`bytes32`): expected `parentHash` value in the witness-bundle parent header.
- `parent_block_number` (`u64`): parent L2 block number.
- `rollup_config_ref` (`string`): rollup config handle/version used by adapter.
- `chain_id` (`u64`): chain id for typed tx decode/validation.
- `witness_bundle_ref` (`string`): deterministic witness bundle path/handle.

### `trackedTxBytes` (required)

- Canonical signed EIP-2718 tx bytes for the 5 benchmark txs we care about.

### `supplementalTxBytes` (required)

- Additional canonical signed tx bytes required to execute the full block without
  missing witness data.
- For the current fixture, 5 supplemental txs are appended to the tracked batch,
  producing 10 executed txs total.

### `outputRootWitness` (required)

- `message_passer_storage_root` (`bytes32`): pinned storage root used to hash the
  OP output root after block execution.

### `blockContext` (required)

- `block_number` (`u64`)
- `timestamp` (`u64`)
- `gas_limit` (`u64`)
- `fee_recipient` (`address`)
- `prev_randao` (`bytes32`)
- `parent_beacon_block_root` (`bytes32`)

### `postCheckpoint` (required)

- `next_output_root` (`bytes32`)
- `new_parent_header_hash` (`bytes32`)
- `new_parent_block_number` (`u64`)
- `witness_bundle_ref` (`string`)

### `executionArtifacts` (required)

- `tx_hashes` (`bytes32[]`)
- `tracked_tx_count` (`u64`)
- `execution_tx_count` (`u64`)
- `gas_used` (`u64`)
- `receipt_root` (`bytes32`, optional until executor output is wired)
- `logs_bloom` (`bytes`, optional until executor output is wired)

## Plan 008.7 phases 1-2 chunk plan and driver contract

The canonical replay partition is now defined by:

- `runs/fixtures/l2-poc-synth-chunk-plan-v1.json`
- `scripts/generate_l2_poc_chunk_plan.sh`

### Progression model

- Uniform `execute_chunk` tiles.
- Tile 0 starts from `preCheckpoint`.
- Each non-final tile emits a resumable `chunk_checkpoint`.
- The final tile seals the canonical block and emits the final block checkpoint.

### Initial deterministic chunking rule

- Policy kind: `fixed-tx-count`
- `chunk_size = 1`
- `tracked_tx_count = 5`
- `supplemental_tx_count = 5`
- `execution_tx_count = 10`

This yields 10 deterministic replay tiles for the canonical package: the first 5
tiles each contain one tracked tx, and the next 5 tiles each contain one
supplemental tx.

### Block-global vs tile-local fields

Block-global fields stay constant across all tiles:

- `start_block`, `end_block`, `start_timestamp`, `gas_limit`, `fee_recipient`
- `batch_hash`, `prev_output_root`
- `rollup_config_ref`, `witness_bundle_ref`
- `message_passer_storage_root`

Per-tile boundary fields are explicit in the chunk-plan sidecar:

- `start_tx_index`, `end_tx_index_exclusive`
- `tx_ids`, `tx_hashes`
- `tracked_tx_ids`, `supplemental_tx_ids`
- `seals_block`

Carry-forward checkpoint fields required after each chunk:

- `tx_cursor`
- `pending_header_hash`
- `pending_state_root`
- `gas_used_so_far`
- `last_executed_tx_hash`
- `witness_bundle_ref`

Finalization-only fields on the sealing tile:

- `next_output_root`
- `sealed_block_hash`
- `total_gas_used`

## Plan 008.7 phase 3 Raster program contract

Phase 3 restructures the workload as a canonical Raster program where tiles and
sequences are authored using the `raster` crate's `#[tile]` and `#[sequence]`
attributes, and trace records are emitted by the Raster runtime — not by manual
JSON construction.

### Raster program shape

The program entry is the `l2_block_execution` sequence which orchestrates 10
explicit tile calls:

- 9 non-final tiles: `execute_chunk_0` through `execute_chunk_8`
- 1 final (block-sealing) tile: `finalize_block`

Each tile is annotated with `#[tile(kind = iter)]` which auto-instruments the
function to serialize inputs/outputs via postcard and emit a `[trace]` record
through the Raster runtime's `emit_trace` subscriber.

### Trace output format

Strict mode now emits:

- 10 Raster-native `[trace]` records (one per tile), each containing:
  - `fn_name` — the tile function name (e.g. `execute_chunk_0`, `finalize_block`)
  - `input_data` — postcard-serialized tile inputs
  - `output_data` — postcard-serialized tile outputs
  - `inputs` — parameter metadata (name and type for each parameter)
  - `output_type` — return type as a string
- 1 `[summary]` record (after `raster::finish()`) containing domain-specific
  validation fields:
  - `next_output_root`, `output_root_status`, `state_root`, `gas_used`
  - `block_hash`, `tile_count`, `tracked_tx_count`, `execution_tx_count`

Fallback mode preserves the legacy whole-block path and emits a single
`[trace]` record with the original JSON schema.

### Shared execution state

In native execution mode, a single `ChunkDriver` persists across all tiles
within one `l2_block_execution` sequence call via thread-local storage. The
TrieDB and cumulative EVM state are carried forward in-process so later tiles
can resume from where prior tiles left off without replaying from the parent
checkpoint.

This shared state is an optimization for the native execution path. In a real
zkVM execution, each tile would be an independent proving unit with its own
witness data.

## Canonical fixture contract

The first L2 POC uses canonical synthetic fixture inputs from
`runs/fixtures/l2-poc-synth-fixture.json`.

The current plan-008.6 slice re-homes that package under repo control with a
deterministic local regeneration path. The underlying witness snapshot lineage
is still bootstrap-derived from vendored Kona test fixtures, but canonical runs
no longer depend on external historical RPC reads.

### Fixed block target

- `startBlock = 26207960`
- `endBlock = 26207960`
- Exactly 1 executed block.
- `trackedTxCount = 5`
- `supplementalTxCount = 5`
- `executionTxCount = 10`
- `timestampDeltaSeconds = 0`
- `fixtureId = l2-poc-synth-v1`

## Claim-facing mapping contract

Plan 008.6 keeps the claim-facing mapping stable for later settlement phases:

- `prevOutputRoot = preCheckpoint.prev_output_root`
- `nextOutputRoot = postCheckpoint.next_output_root` from the sealed block
- `startBlock = 26207960`
- `endBlock = 26207960`
- `batchHash = keccak256(concat(tx1_raw, tx2_raw, tx3_raw, tx4_raw, tx5_raw))`

The claim object may additionally carry a canonical input blob/versioned-hash
pointer, but that pointer is outside the execution-program boundary.

## Failure model

Failures are categorized for deterministic reporting:

- `invalid-fixture-input`: missing/malformed checkpoint, tx bytes, or context.
- `incomplete-witness`: checkpoint exists but required witness material is absent.
- `execution-failed`: Kona execution returns failure for a valid input package.
- `output-root-witness-missing`: execution succeeded but the required seeded
  output-root witness is absent.

This separation is required so runner/UI can distinguish fixture errors from
executor failures.

## Implementation snapshot

- Workload entrypoint: `apps/workloads/l2-kona-poc/src/main.rs`
- Workload id: `l2-kona-poc` (wired through `crates/shared/src/raster_workload.rs`)
- Input contract: the full fixture JSON (`runs/fixtures/l2-poc-synth-fixture.json`) is passed directly via `--input`
- Chunk-plan artifact: `runs/fixtures/l2-poc-synth-chunk-plan-v1.json` is generated from the canonical fixture via `scripts/generate_l2_poc_chunk_plan.sh`
- Local witness artifacts: `fixtures/l2-poc/rollup-config-v1.json` + `fixtures/l2-poc/synthetic-witness-bundle-v1.json` + `fixtures/l2-poc/synthetic-witness-kv-v1*`
- Runtime execution shape: 10 Raster-native `[trace]` records plus 1 `[summary]` record
- Raster program shape: `#[sequence] fn l2_block_execution` orchestrates 10 `#[tile(kind = iter)]` calls
- Strict runtime path: the `raster::init()` / `raster::finish()` lifecycle manages trace emission; a shared `ChunkDriver` persists across tiles via thread-local storage
- Fallback runtime path: exploratory fallback mode still uses the existing whole-block path and emits a single legacy `[trace]` record
- Planning flags: `--emit-chunk-plan` prints the canonical chunk-plan JSON and `--chunk-size` selects the deterministic fixed-tx partition size used for planning output
- Execution mode per run:
  - strict canonical mode (`--execution-mode strict`, default): runs the Raster program with tile-level tracing
  - fallback dev mode (`--execution-mode fallback`): preserves the legacy whole-block path for exploratory runs
  - runner override: set `L2_KONA_EXECUTION_MODE=fallback` to force exploratory mode when running through `apps/runner`
- Strict preflight now validates the witness closure manifest contract before execution: bundle ref, rollup-config ref, tracked tx hashes, supplemental tx hashes, block window, output-root witness, and all referenced witness KV-store paths must match the canonical package.

## Witness closure manifest contract (Plan 008.5 / 008.6)

Canonical witness closure is pinned by:

- `fixtures/l2-poc/synthetic-witness-bundle-v1.json`
- `fixtures/l2-poc/synthetic-witness-closure-manifest-v1.json`

The manifest binds fixture identity (`fixture_id`, `batch_hash`, tracked tx
hashes, supplemental tx hashes, block target, parent checkpoint anchor, seeded
output-root witness) to deterministic content hashes for
every referenced witness KV store.

Regenerate the canonical synthetic package deterministically with:

```bash
python3 scripts/generate_l2_poc_synthetic_fixture.py --force
```

Refresh the manifest alone with:

```bash
python3 scripts/generate_l2_poc_witness_manifest.py
```

Refresh the canonical chunk-plan sidecar with:

```bash
scripts/generate_l2_poc_chunk_plan.sh
```

## Acceptance gate command (strict canonical mode)

Run the strict acceptance check with:

```bash
scripts/check_l2_kona_strict.sh
```

Pass criteria:

- Exactly 10 Raster-native trace records per run (one per tile).
- Each trace contains `fn_name` matching the expected tile function names.
- The `[summary]` record reports `output_root_status = fixture_output_root`,
  `tracked_tx_count = 5`, `execution_tx_count = 10`, and `tile_count = 10`.
- Two back-to-back runs produce identical `next_output_root` values.
- Raster trace `output_data` bytes match across repeated runs.
- The canonical chunk plan contains 10 deterministic one-tx tiles and the final
  tile reports `seals_block = true`.
