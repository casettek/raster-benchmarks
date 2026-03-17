# L2 Kona Workload Spec (Plan 007)

This spec defines the v1 native Kona adapter boundary and checkpoint contract for
the first L2 POC workload.

## Scope

- One fixed 5-transaction batch.
- One execution tile shape.
- Five explicit tile invocations from one Raster `main` sequence.
- Native-only execution path (no `Risc0` guest/host flow in this phase).

Out of scope for this spec:

- Settlement contract implementation details.
- Runner/web UX details.
- Fraud-proof object generation and proof verification.

## Canonical tile boundary

The execution boundary is explicit and serializable:

`preCheckpoint + txBytes + blockContext -> postCheckpoint + executionArtifacts`

### `preCheckpoint` (required)

- `prev_output_root` (`bytes32`): prior agreed OP `outputRoot`.
- `parent_header_hash` (`bytes32`): hash of the parent L2 header.
- `parent_block_number` (`u64`): parent L2 block number.
- `rollup_config_ref` (`string`): rollup config handle/version used by adapter.
- `chain_id` (`u64`): chain id for typed tx decode/validation.
- `witness_bundle_ref` (`string`): deterministic witness bundle path/handle.

### `txBytes` (required)

- Canonical signed EIP-2718 tx bytes for exactly one synthetic block.
- The program input carries bytes directly; no DA/blob pointer is part of tile
  execution input.

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

- `tx_hash` (`bytes32`)
- `gas_used` (`u64`)
- `receipt_root` (`bytes32`, optional until executor output is wired)
- `logs_bloom` (`bytes`, optional until executor output is wired)

## Canonical fixture contract

The first L2 POC uses fixture inputs from
`runs/fixtures/l2-poc-plan7-fixture.json`.

### Fixed block window

- `startBlock = 900001`
- `endBlock = 900005`
- Exactly 5 synthetic one-transaction blocks.

Per-tile progression is deterministic:

- `block_number(i) = startBlock + i`
- `timestamp(i) = startTimestamp + (i * timestampDeltaSeconds)`
- `gas_limit` stays constant for all 5 tiles.
- `fee_recipient` stays constant for all 5 tiles.

## Claim-facing mapping contract

Plan 007 freezes the mapping used by later settlement phases:

- `prevOutputRoot = preCheckpoint.prev_output_root`
- `nextOutputRoot = postCheckpoint.next_output_root` from tile 5
- `startBlock = 900001`
- `endBlock = 900005`
- `batchHash = keccak256(concat(tx1_raw, tx2_raw, tx3_raw, tx4_raw, tx5_raw))`

The claim object may additionally carry a canonical input blob/versioned-hash
pointer, but that pointer is outside the execution-program boundary.

## Failure model

Failures are categorized for deterministic reporting:

- `invalid-fixture-input`: missing/malformed checkpoint, tx bytes, or context.
- `incomplete-witness`: checkpoint exists but required witness material is absent.
- `execution-failed`: Kona execution returns failure for a valid input package.
- `output-root-failed`: execution succeeded but `outputRoot` derivation failed.

This separation is required so runner/UI can distinguish fixture errors from
executor failures.
