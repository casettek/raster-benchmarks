# Local E2E Scenarios Spec

This spec defines the deterministic L2 fixture and scenario assertions for the
native Kona workload POC.

## Canonical fixture

Source of truth: `runs/fixtures/l2-poc-synth-fixture.json`

Canonical witness closure for strict Kona execution is pinned by:

- `fixtures/l2-poc/synthetic-witness-bundle-v1.json`
- `fixtures/l2-poc/synthetic-witness-closure-manifest-v1.json`

### Story identities

- `alice`, `bob`, and `carol` are the narrative labels for the 5 tracked token
  transfers in the canonical package.
- `token`: `0xb6def636914ae60173d9007e732684a9eedef26e`
- `feeRecipient`: `0x4200000000000000000000000000000000000011`

The canonical package is regenerated locally from repo-owned seed metadata plus
vendored witness snapshots. No live historical RPC access is required.

### Fixed execution target

- `startBlock = 26207960`
- `endBlock = 26207960`
- `startTimestamp = 1744218460`
- `timestampDeltaSeconds = 0`
- `gasLimit = 60000000`

### Canonical replay chunk plan

- Source of truth: `runs/fixtures/l2-poc-synth-chunk-plan-v1.json`
- Policy: fixed tx-count chunking with `chunk_size = 1`
- Tile count: `10`
- Tile ordering: tracked txs occupy tiles `0..=4`; supplemental txs occupy
  tiles `5..=9`
- Finalization: tile `9` is the only sealing tile (`seals_block = true`)

### Raster program shape

The canonical strict execution path is a real Raster program:

- Entry sequence: `l2_block_execution`
- Tile function: `execute_chunk` — one function, called 10 times with
  `tile_index` 0 through 9
- The tile uses `#[tile(kind = iter)]` for Raster runtime auto-tracing
- The sequence uses `#[sequence]` for CFS registration
- Traces are emitted by the Raster runtime's `emit_trace` subscriber, not by
  manual JSON construction
- A `[summary]` line after `raster::finish()` carries domain-specific
  validation fields

### Canonical tracked transaction order (5 total)

All tracked transactions are signed EIP-2718 txs against `chainId = 11155420`
and are the first five txs in the canonical block execution package.

1. `alice -> bob` (`nonce=0`, `amount=1.0` token)
2. `alice -> carol` (`nonce=1`, `amount=0.5` token)
3. `bob -> alice` (`nonce=0`, `amount=0.25` token)
4. `carol -> bob` (`nonce=0`, `amount=0.125` token)
5. `alice -> bob` (`nonce=2`, `amount=0.1` token)

Tracked hashes:

- `tx1 = 0x24b776bee9799bef3cc487401151d71c08241255882633870b442c673651ce60`
- `tx2 = 0xe06c6f8d15665606ecf92b19b6d7333d072c64a3e45a492470ecbd13cf116d97`
- `tx3 = 0xcb21cd3cbe16f3653b946d16253c7bbf1fef0a5c7cc7f6dc8daafb8c1239bfbb`
- `tx4 = 0x99776a682dfc28c0c3f173ee61f7177dee3221875918c7eb432663cea7c72019`
- `tx5 = 0x0686788003eda5de8529af5423b6b545e93827ee2756a8d8d53fc515dd996123`

Supplemental execution tx count:

- `supplementalTxCount = 5`
- `executionTxCount = 10`

Canonical batch commitment:

- `batchHash = keccak256(concat(tx1_raw, tx2_raw, tx3_raw, tx4_raw, tx5_raw))`
- `batchHash = 0xb9ef076572948183c38d75a6b8966236c1030c83c0e6ab50b813266de50be229`

## Program input contract

The Raster program input for the single-block run is deterministic and
blob-agnostic:

- `preCheckpoint` (seeded root/header/witness snapshot)
- `txBatch` (ordered array of 5 tracked tx byte strings)
- `supplementalTxBatch` (ordered array of 5 additional canonical block tx byte
  strings)
- `outputRootWitness.message_passer_storage_root`
- `blockContextSeed` (`startBlock`, `startTimestamp`, `timestampDeltaSeconds`,
  `gasLimit`, `feeRecipient`, `prevRandao`, `parentBeaconBlockRoot`)

DA/blob references are part of claim packaging, not part of execution input.
The canonical chunk plan is a sidecar replay contract derived from the same
fixture, not an additional user-facing runtime input field in this phase.

## Run lifecycle

L2 runs use the expanded lifecycle:

1. **Prepare Batch** — load the canonical synthetic fixture and derive batch identity.
2. **Execute Program** — run the Raster program (10 chunked tile invocations). Trace
   artifacts are folded into execution (no separate `trace` step).
3. **Publish to DA** — publish trace payload via blob tx.
4. **Submit Claim** — submit blob-carrying settlement claim with bond, binding:
   `prevOutputRoot`, `nextOutputRoot`, `startBlock`, `endBlock`, `batchHash`,
   `inputBlobVersionedHash`.
5. **Audit** — independent local replay comparison with conditional trace fetch.
6. **Await Finalization** — challenge-period countdown (120s default).
7. **Outcome** — terminal `Settled` (honest) or `Slashed` (dishonest).

Golden run fixtures for both paths are at `runs/fixtures/l2-kona-poc-honest.json`
and `runs/fixtures/l2-kona-poc-dishonest.json`.

## Scenario assertions

### Honest scenario

- Consumes the canonical fixture unchanged.
- Executes one canonical block as 10 invocations of the `execute_chunk` tile.
- Each invocation is auto-traced by the Raster runtime (`[trace]` records with
  `fn_name = "execute_chunk"`, `input_data`, `output_data`).
- The sequence `l2_block_execution` orchestrates all 10 tile calls.
- Uses strict canonical execution mode (no fallback statuses permitted).
- Produces deterministic `nextOutputRoot` for block `26207960`.
- Emits a `[summary]` record with `output_root_status = fixture_output_root`.
- Submits L2 settlement claim with claimer bond, binding:
  `prevOutputRoot`, `nextOutputRoot`, `startBlock`, `endBlock`, `batchHash`.
- Contract records `challengeDeadline = createdAt + challengePeriod`.
- Audit step confirms no divergence; trace fetch is `skipped`.
- Await-finalization step waits for the challenge deadline to pass.
- After challenge deadline, claim settles and bond is returned to claimer.
- Canonical deterministic `nextOutputRoot` for the synthetic fixture:
  `0xe13f82b2b6e02d94a7b1a2a5a8ca21da71c7d14c1e3e35d97687e7bf86425b17`

### Dishonest scenario

- Uses same canonical fixture input bytes and block target.
- Replay/audit flow computes a deliberately wrong `nextOutputRoot` (byte-flipped).
- Audit step detects divergence and fetches trace payload from DA pointer.
- Challenger calls `challengeClaim` with the divergent root before the deadline.
- Await-finalization step shows `Challenged before deadline`.
- Contract transitions to `Slashed`, bond transferred to challenger.
- Replay/audit flow classifies divergence deterministically and emits structured
  divergence report.

## Fixture completeness rule

Mark fixture as invalid (input failure) before execution if any of these are
missing:

- any tx raw bytes or tx hash mismatch
- checkpoint root/header anchors
- block context seed fields
- witness bundle reference
- seeded output-root witness

Only after fixture completeness passes can failures be classified as execution
or output-root derivation failures.

In strict canonical mode, missing execution witness preimages are treated as
hard failures with step-local diagnostics (`trie-node` vs `bytecode` witness
class) and must not be silently converted into fallback output roots.
