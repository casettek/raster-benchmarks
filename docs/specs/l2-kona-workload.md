# L2 Kona Workload Spec (Plan 008 / 008.5 / 008.6)

This spec defines the v1 native Kona adapter boundary and checkpoint contract for
the first L2 POC workload.

## Scope

- One fixed 5-transaction benchmark batch.
- One single-block execution tile shape.
- One explicit Raster transition that executes the 5 benchmark txs plus any supplemental txs required to close the canonical block.
- Native-only execution path (no `Risc0` guest/host flow in this phase).

Out of scope for this spec:

- Settlement contract implementation details.
- Runner/web UX details.
- Fraud-proof object generation and proof verification.

## Canonical tile boundary

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

## Plan 008 implementation snapshot

- Workload entrypoint: `apps/workloads/l2-kona-poc/src/main.rs`
- Workload id: `l2-kona-poc` (wired through `crates/shared/src/raster_workload.rs`)
- Input contract: the full fixture JSON (`runs/fixtures/l2-poc-synth-fixture.json`) is passed directly via `--input`
- Local witness artifacts: `fixtures/l2-poc/rollup-config-v1.json` + `fixtures/l2-poc/synthetic-witness-bundle-v1.json` + `fixtures/l2-poc/synthetic-witness-kv-v1*`
- Execution shape: exactly 1 trace record is emitted for the sealed block (`exec_index = 0`)
- Execution contract: the workload executes all 10 canonical block txs while still tracking the first 5 benchmark txs explicitly in the trace output
- Execution mode per run:
  - primary path: `kona-executor::StatelessL2Builder` single-block execution over the complete canonical tx list
  - strict canonical mode (`--execution-mode strict`, default): fail fast on missing execution witness preimages; canonical traces must report `output_root_status = fixture_output_root`, `tracked_tx_count = 5`, and `execution_tx_count = 10`
  - fallback dev mode (`--execution-mode fallback`): preserve deterministic fallback status `synthetic_incomplete_witness` for exploratory runs
  - runner override: set `L2_KONA_EXECUTION_MODE=fallback` to force exploratory mode when running through `apps/runner`
- Strict preflight now validates the witness closure manifest contract before execution: bundle ref, rollup-config ref, tracked tx hashes, supplemental tx hashes, block window, output-root witness, and all referenced witness KV-store paths must match the canonical package.

The workload consumes a complete single-block execution package so the tracked 5
txs are never skipped just because the output-root helper would otherwise need a
missing trie witness.

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

The legacy bootstrap fixture at `runs/fixtures/l2-poc-plan7-fixture.json`
remains reference-only and is no longer the canonical strict path.

## Acceptance gate command (strict canonical mode)

Run the strict acceptance check with:

```bash
scripts/check_l2_kona_strict.sh
```

Pass criteria:

- Exactly 1 trace record per run.
- The trace reports `tracked_tx_count = 5`, `execution_tx_count = 10`, and
  `output_root_status = fixture_output_root`.
- Two back-to-back runs produce identical `next_output_root` values.
