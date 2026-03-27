# L2 Kona Workload Spec

This spec defines the native Kona adapter boundary and checkpoint contract for
the L2 POC workload.

## Scope

- One fixed 5-transaction benchmark batch.
- One canonical Raster program that executes the batch as 10 deterministic tile
  invocations of a single `execute_chunk` tile function.
- One explicit Raster transition that executes the 5 benchmark txs plus any
  supplemental txs required to close the canonical block.
- One canonical chunk-plan sidecar that partitions the same 10 execution txs
  into deterministic replay slices (default `chunk_size = 1`).
- Native-only execution path (no Risc0 guest/host flow in this phase).

Out of scope for this spec:

- Settlement contract implementation details.
- Runner/web UX details.
- Fraud-proof object generation and proof verification.

## Execution boundary

The execution boundary is explicit and serializable:

`preCheckpoint + trackedTxBytes + supplementalTxBytes + outputRootWitness + blockContext -> postCheckpoint + executionArtifacts`

### `preCheckpoint` (required)

- `prev_output_root` (`bytes32`): prior agreed OP `outputRoot`.
- `parent_header_hash` (`bytes32`): expected `parentHash` value in the
  witness-bundle parent header.
- `parent_block_number` (`u64`): parent L2 block number.
- `rollup_config_ref` (`string`): rollup config handle/version used by adapter.
- `chain_id` (`u64`): chain id for typed tx decode/validation.
- `witness_bundle_ref` (`string`): deterministic witness bundle path/handle.

### `trackedTxBytes` (required)

- Canonical signed EIP-2718 tx bytes for the 5 benchmark txs we care about.

### `supplementalTxBytes` (required)

- Additional canonical signed tx bytes required to execute the full block
  without missing witness data.
- For the current fixture, 5 supplemental txs are appended to the tracked batch,
  producing 10 executed txs total.

### `outputRootWitness` (required)

- `message_passer_storage_root` (`bytes32`): pinned storage root used to hash
  the OP output root after block execution.

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

## Chunk plan and driver contract

The canonical replay partition is defined by:

- `runs/fixtures/l2-poc-synth-chunk-plan-v1.json`
- `scripts/generate_l2_poc_chunk_plan.sh`

### Progression model

- Uniform `execute_chunk` tiles — one tile function, invoked N times.
- Tile 0 starts from `preCheckpoint`.
- Each non-final tile advances the execution cursor.
- The final tile seals the canonical block and emits the output root.

### Deterministic chunking rule

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

## Raster program contract

The workload is a canonical Raster program where tiles and sequences are
authored using the `raster` crate's `#[tile]` and `#[sequence]` attributes,
and trace records are emitted by the Raster runtime — not by manual JSON
construction.

### Raster program shape

```
#[sequence]
fn main(fixture: FixtureInput)
    call!(execute_chunk, fixture, 0)
    call!(execute_chunk, fixture, 1)
    ...
    call!(execute_chunk, fixture, 9)

#[tile(kind = iter)]
fn execute_chunk(fixture, tile_index) -> TileOutput
```

One sequence, one tile function, ten invocations. The `main` sequence is the
Raster program entry point. The sequence body contains only `call!` invocations.
The tile index parameter selects which transaction slice to execute. In a zkVM
context, this compiles to one ELF binary parameterized by `tile_index`.

### Trace output format

The program emits 10 Raster-native `[trace]` records (one per tile invocation),
each containing:

- `fn_name` — always `execute_chunk`
- `sequence_id` — `main`
- `input_data` — postcard-serialized tile inputs (fixture + tile_index)
- `output_data` — postcard-serialized `TileOutput`
- `inputs` — parameter metadata (name and type for each parameter)
- `output_type` — `TileOutput`
- `sequence_coordinates` — `[0]`

### Shared execution state

In native execution mode, a single `ChunkDriver` persists across all tile
invocations via thread-local storage with lazy initialization on the first
tile call. The TrieDB and cumulative EVM state are carried forward in-process
so later tiles can resume from where prior tiles left off without replaying
from the parent checkpoint.

This shared state is an optimization for the native execution path. In a real
zkVM execution, each tile would be an independent proving unit with its own
witness data.

## Raster compiler pipeline

The l2-kona-poc is a Raster program. Build and run it through the Raster
toolchain from the `apps/workloads/l2-kona-poc` directory.

### CFS extraction

```bash
cargo raster cfs
```

Produces `target/raster/cfs.json` with:
- 1 tile: `execute_chunk` (kind: `iter`, 2 inputs, 1 output)
- 1 sequence: `main` (10 items: `execute_chunk` x10)

### Tile discovery

```bash
cargo raster list
```

Lists exactly one tile: `execute_chunk(fixture: FixtureInput, tile_index: usize) -> TileOutput`

### Execution

```bash
cargo raster run --input "$(cat ../../../runs/fixtures/l2-poc-synth-fixture.json)"
```

Builds the project, runs the binary, and captures `[trace]` lines from stdout.
The Raster runtime emits 10 trace records (one per `execute_chunk` invocation)
with full postcard-serialized input/output data.

## Canonical fixture contract

The L2 POC uses canonical synthetic fixture inputs from
`runs/fixtures/l2-poc-synth-fixture.json`.

The fixture package is repo-owned with a deterministic local regeneration path.
The underlying witness snapshot lineage is bootstrap-derived from vendored Kona
test fixtures, but canonical runs do not depend on external historical RPC reads.

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

- `prevOutputRoot = preCheckpoint.prev_output_root`
- `nextOutputRoot = postCheckpoint.next_output_root` from the sealed block
- `startBlock = 26207960`
- `endBlock = 26207960`
- `batchHash = keccak256(concat(tx1_raw, tx2_raw, tx3_raw, tx4_raw, tx5_raw))`

The claim object may additionally carry a canonical input blob/versioned-hash
pointer, but that pointer is outside the execution-program boundary.

## Blob-backed input package contract

- The harness publishes a canonical input-package tarball before claim
  submission.
- That package contains the transaction batch input, fixture JSON, rollup
  config, rewritten witness metadata, and only the minimum extra state/witness
  data currently required to execute the canonical single-block transition for
  that batch (rather than the full vendored closure set).
- The claim stores the input-package manifest blob versioned hash; challenger
  audit fetches that manifest, fetches its referenced chunks from Anvil, and
  materializes the package before replay.
- The workload remains blob-agnostic: it still receives ordinary fixture JSON as
  `--input`, with file refs resolved by the harness-provided materialization
  root.

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
- Source modules:
  - `main.rs` — Raster program (`#[sequence] fn main` + `#[tile] fn execute_chunk`),
    Kona reference execution, TrieDB provider
  - `chunk_plan.rs` — deterministic transaction partitioning into tiles
  - `chunk_driver.rs` — per-tile EVM execution engine using Kona/alloy-op-evm
- Input contract: the full fixture JSON is passed via `--input` (parsed
  automatically by the `#[sequence]` macro on `fn main`)
- Local witness artifacts: `fixtures/l2-poc/rollup-config-v1.json` +
  `fixtures/l2-poc/synthetic-witness-bundle-v1.json` +
  `fixtures/l2-poc/synthetic-witness-kv-v1*`

## Witness closure manifest contract

Canonical witness closure is pinned by:

- `fixtures/l2-poc/synthetic-witness-bundle-v1.json`
- `fixtures/l2-poc/synthetic-witness-closure-manifest-v1.json`

The manifest binds fixture identity (`fixture_id`, `batch_hash`, tracked tx
hashes, supplemental tx hashes, block target, parent checkpoint anchor, seeded
output-root witness) to deterministic content hashes for every referenced
witness KV store.

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

## Acceptance gate

Pass criteria:

- `cargo raster cfs` produces 1 sequence (`main`), 1 tile (`execute_chunk`), 10 calls.
- `cargo raster list` discovers exactly 1 tile.
- `cargo raster run` emits exactly 10 `[trace]` records with `fn_name = "execute_chunk"` and `sequence_id = "main"`.
- Two back-to-back runs produce identical trace record bytes.
- `cargo test -p workload-l2-kona-poc` passes.
