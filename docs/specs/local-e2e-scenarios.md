# Local E2E Scenarios Spec (L2 POC Plan 007)

This spec defines the deterministic L2 fixture and scenario assertions for the
first native Kona workload POC.

## Canonical fixture

Source of truth: `runs/fixtures/l2-poc-plan7-fixture.json`

### Seeded identities

- `alice`: `0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266`
- `bob`: `0x70997970C51812dc3A010C7d01b50e0d17dc79C8`
- `carol`: `0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC`
- `token`: `0x1000000000000000000000000000000000000001`
- `feeRecipient`: `0x2000000000000000000000000000000000000001`

### Fixed execution window

- `startBlock = 900001`
- `endBlock = 900005`
- `startTimestamp = 1700000000`
- `timestampDeltaSeconds = 2`
- `gasLimit = 30000000`

### Canonical transaction order (5 total)

All transactions are signed EIP-2718 type-2 txs against `chainId = 42069` and
call `transfer(address,uint256)` on the fixed token contract.

1. `alice -> bob` (`nonce=0`, `amount=1.0` token)
2. `alice -> carol` (`nonce=1`, `amount=0.5` token)
3. `bob -> alice` (`nonce=0`, `amount=0.25` token)
4. `carol -> bob` (`nonce=0`, `amount=0.125` token)
5. `alice -> bob` (`nonce=2`, `amount=0.1` token)

Canonical hashes:

- `tx1 = 0x854df7d49647b8a7cbd1aa3842771bb59ade9f767fc93a01c7c7ca4f064d55b1`
- `tx2 = 0xdcf82dab307b3efe5c678ea1e8faaf7b62b8b3bdb059bb1c0fd8769bcc2557a3`
- `tx3 = 0xd598a956891c5fdaffe217e6dd50ba74e639db6b6f4a6c9f6ba3793c525d930c`
- `tx4 = 0x7db9fcab301662069f118767601e89d9be74d084dff642021948641b23d84c4a`
- `tx5 = 0xa7ca5f8f7a1bcf92b4bce38241d55e808385b21645df25ceb8543612a284516e`

Canonical batch commitment:

- `batchHash = keccak256(concat(tx1_raw, tx2_raw, tx3_raw, tx4_raw, tx5_raw))`
- `batchHash = 0x9091820ee7372c1090ec433dcbfdb8a9205d9037818d469bc425b66d67cbe87d`

## Program input contract

The Raster program input for the 5-tile run is deterministic and blob-agnostic:

- `preCheckpoint` (seeded root/header/witness snapshot)
- `txBatch` (ordered array of 5 raw signed tx byte strings)
- `blockContextSeed` (`startBlock`, `startTimestamp`, `timestampDeltaSeconds`,
  `gasLimit`, `feeRecipient`, `prevRandao`, `parentBeaconBlockRoot`)

DA/blob references are part of claim packaging, not part of execution input.

## Scenario assertions

### Honest scenario

- Consumes the canonical fixture unchanged.
- Executes five one-transaction synthetic blocks in order.
- Produces deterministic final `nextOutputRoot` for block `900005`.
- Submits claim metadata bound to
  `prevOutputRoot`, `nextOutputRoot`, `startBlock`, `endBlock`, `batchHash`.

### Dishonest scenario

- Uses same canonical fixture input bytes and block window.
- Claim contains an intentionally incorrect `nextOutputRoot`.
- Replay/audit flow must classify divergence deterministically and reject/slash.

## Fixture completeness rule

Mark fixture as invalid (input failure) before execution if any of these are
missing:

- any tx raw bytes or tx hash mismatch
- checkpoint root/header anchors
- block context seed fields
- witness bundle reference

Only after fixture completeness passes can failures be classified as execution or
output-root derivation failures.
