# Local E2E Scenarios Spec (L2 POC Plan 007 / 008.6)

This spec defines the deterministic L2 fixture and scenario assertions for the
first native Kona workload POC.

## Canonical fixture

Source of truth: `runs/fixtures/l2-poc-synth-fixture.json`

Canonical witness closure for strict Kona execution is pinned by:

- `fixtures/l2-poc/synthetic-witness-bundle-v1.json`
- `fixtures/l2-poc/synthetic-witness-closure-manifest-v1.json`

### Story identities

- `alice`, `bob`, and `carol` are the narrative labels for the 5 tracked token transfers in the canonical package.
- `token`: `0xb6def636914ae60173d9007e732684a9eedef26e`
- `feeRecipient`: `0x4200000000000000000000000000000000000011`

The plan-008.6 canonical package is regenerated locally from repo-owned seed
metadata plus vendored witness snapshots. The current bootstrap slice preserves
the parent-header / block-number lineage of the cached witness snapshot while
removing any need for live historical RPC access during normal runs.

### Fixed execution target

- `startBlock = 26207960`
- `endBlock = 26207960`
- `startTimestamp = 1744218460`
- `timestampDeltaSeconds = 0`
- `gasLimit = 60000000`

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

The Raster program input for the single-block run is deterministic and blob-agnostic:

- `preCheckpoint` (seeded root/header/witness snapshot)
- `txBatch` (ordered array of 5 tracked tx byte strings)
- `supplementalTxBatch` (ordered array of 5 additional canonical block tx byte strings)
- `outputRootWitness.message_passer_storage_root`
- `blockContextSeed` (`startBlock`, `startTimestamp`, `timestampDeltaSeconds`, `gasLimit`, `feeRecipient`, `prevRandao`, `parentBeaconBlockRoot`)

DA/blob references are part of claim packaging, not part of execution input.

## Scenario assertions

### Honest scenario

- Consumes the canonical fixture unchanged.
- Executes one canonical block in order with all 10 execution txs.
- Uses strict canonical execution mode (no fallback statuses permitted).
- Produces deterministic `nextOutputRoot` for block `26207960`.
- Submits claim metadata bound to
  `prevOutputRoot`, `nextOutputRoot`, `startBlock`, `endBlock`, `batchHash`.

### Dishonest scenario

- Uses same canonical fixture input bytes and block target.
- Claim contains an intentionally incorrect `nextOutputRoot`.
- Replay/audit flow must classify divergence deterministically and reject/slash.

## Fixture completeness rule

Mark fixture as invalid (input failure) before execution if any of these are
missing:

- any tx raw bytes or tx hash mismatch
- checkpoint root/header anchors
- block context seed fields
- witness bundle reference
- seeded output-root witness

Only after fixture completeness passes can failures be classified as execution or
output-root derivation failures.

In strict canonical mode, missing execution witness preimages are treated as
hard failures with step-local diagnostics (`trie-node` vs `bytecode` witness
class) and must not be silently converted into fallback output roots.
