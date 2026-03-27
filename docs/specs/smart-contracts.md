# Benchmark Smart Contracts Spec

This spec defines benchmark-local smart-contract ownership and the L2
settlement contract surface for `raster-benchmarks`.

## Ownership and scope

- Contracts used by benchmark claim/challenge scenarios are owned by this repo.
- These contracts are benchmark harness contracts and may evolve quickly for
  feasibility/performance iteration.
- They do not define final production Raster protocol interfaces.

## Package layout

- Foundry package root: `contracts/`
- Sources: `contracts/src/`
- Tests: `contracts/test/`
- Deployment/setup scripts: `contracts/script/`
- Build config: `contracts/foundry.toml`

## App interaction layout

- Claimer executable: `apps/claimer`
- Challenger executable: `apps/challenger`
- Runner orchestrator: `apps/runner`
- Web server: `apps/web-server`
- Shared contract bindings: `crates/shared`

## Runtime baseline

- Local chain: `anvil`
- Contract toolchain: `forge` (Foundry)
- Contract flows are expected to run in local deterministic scenarios before
  any testnet adaptation.

## L2 settlement contract (plan-014)

`contracts/src/interfaces/IClaimVerifier.sol` defines the L2 settlement
interaction surface. The contract centers on an optimistic-rollup-style
`outputRoot` transition claim plus a contract-managed blob registry for the
claim-bound manifest blob hashes.

### Blob registry

The contract keeps first-seen inclusion metadata for claim-bound manifest blob
hashes:

| Field | Type | Description |
|---|---|---|
| `blockNumber` | `uint64` | Block number where the contract first observed the blob hash in a blob-carrying registration tx |
| `timestamp` | `uint64` | Block timestamp for that first observation |

- Registration happens in a dedicated blob-carrying transaction via
  `registerManifestBlobs()`.
- The function loops over `blobhash(0..5)`, stores each nonzero hash once, and
  preserves the first-seen metadata on duplicates.
- The contract does not prune old entries. Claim freshness is enforced when the
  claim is submitted.

### Claim freshness invariant

The contract's job is to prove existence and challenge-window freshness, not
payload correctness.

- `inputBlobVersionedHash` and `traceBlobVersionedHash` are caller-supplied as
  claim references, but they must already exist in the registry.
- `traceBlobVersionedHash` is required.
- `inputBlobVersionedHash` remains optional at the contract level for shared
  demo compatibility, but if present it must also be registered and fresh.
- Freshness rule:

```text
registeredAtTimestamp + blobRetentionWindow >= block.timestamp + challengePeriod
```

This ensures the claim cannot reference a blob hash that would expire before
the challenge window closes.

### Claim object

The `Claim` struct stores the full canonical L2 transition claim:

| Field | Type | Description |
|---|---|---|
| `claimer` | `address` | Address that submitted and bonded the claim |
| `prevOutputRoot` | `bytes32` | Prior agreed OP output root |
| `nextOutputRoot` | `bytes32` | Claimed OP output root after execution |
| `startBlock` | `uint64` | First L2 block in the claimed range |
| `endBlock` | `uint64` | Last L2 block in the claimed range |
| `batchHash` | `bytes32` | `keccak256(concat(tx1_raw..txN_raw))` of the canonical batch |
| `inputBlobVersionedHash` | `bytes32` | Canonical input-package manifest blob versioned hash supplied at claim submission |
| `traceBlobVersionedHash` | `bytes32` | Trace-commitment manifest blob versioned hash supplied at claim submission |
| `bondAmount` | `uint256` | ETH bond locked by the claimer |
| `createdAt` | `uint64` | Block timestamp when claim was created |
| `challengeDeadline` | `uint64` | Timestamp after which settlement is allowed |
| `state` | `ClaimState` | Enum: None(0), Pending(1), Settled(2), Slashed(3) |

### Claim state machine

```
             submit (+ bond)
                  │
                  ▼
              ┌────────┐
              │Pending │
              └───┬────┘
             ┌────┴────┐
    challenge │         │ settle (after deadline)
    (+ proof) │         │
              ▼         ▼
          ┌───────┐ ┌────────┐
          │Slashed│ │Settled │
          └───────┘ └────────┘
```

- **Submit**: claimer posts bond (>= `minBond`), contract records claim with
  `challengeDeadline = block.timestamp + challengePeriod`. Claims without a
  trace blob hash are rejected.
- **Challenge**: anyone can challenge before `challengeDeadline` by providing
  a divergent `observedNextOutputRoot`. Claim transitions to `Slashed`, bond
  transferred to challenger. No challenger stake required in v1.
- **Settle**: after `challengeDeadline`, anyone can finalize. Claim transitions
  to `Settled`, bond returned to claimer.

### Constructor parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `_challengePeriod` | `uint64` | 120 | Challenge window in seconds |
| `_minBond` | `uint256` | 0.01 ether | Minimum ETH bond for claim submission |
| `_blobRetentionWindow` | `uint64` | `18 days` | Minimum DA lifetime the contract enforces for registered claim-bound blobs |

### Functions

- `registerManifestBlobs()` records every nonzero `blobhash(i)` attached to the
  registration tx, preserving the first-seen block/timestamp for each hash.
- `submitClaim(prevOutputRoot, nextOutputRoot, startBlock, endBlock, batchHash,
  inputBlobVersionedHash, traceBlobVersionedHash)` creates a pending claim with
  claimer bond and records the two canonical DA references explicitly. Requires
  `msg.value >= minBond`, requires registered blob hashes, and rejects hashes
  that are too old to remain available for the full challenge window.
- `challengeClaim(claimId, observedNextOutputRoot)` marks a claim slashed when
  the challenger observes a different `nextOutputRoot`. Must be called before
  `challengeDeadline`. Transfers bond to challenger.
- `settleClaim(claimId)` settles an uncontested pending claim after the
  challenge deadline. Returns bond to claimer.
- `getClaim(claimId)` exposes current claim state for harness assertions.
- `getBlobRegistration(blobVersionedHash)` exposes the first-seen block and
  timestamp for a registered manifest blob hash.
- `challengePeriod()` returns the configured challenge window (seconds).
- `minBond()` returns the configured minimum bond amount.
- `blobRetentionWindow()` returns the configured blob freshness window
  enforced at claim submission.

### DA pointer fields

`inputBlobVersionedHash` is the settlement-critical DA binding for the claimed
execution package. `traceBlobVersionedHash` is the audit-critical DA binding for
the published trace commitment. The primary settlement anchor is still the
`outputRoot` transition, but the contract now also proves that these manifest
blob hashes were previously observed onchain and were still within the required
DA lifetime when the claim was submitted.

Payload validity remains offchain:

- The registry only proves that a blob hash exists and is fresh enough.
- Challenger audit still fetches the referenced manifests from Anvil, fetches
  the underlying payload chunks, reconstructs the payload bytes, and validates
  replay/trace correctness separately.

## Determinism requirements

- Contract behavior must be deterministic for identical claim/challenge inputs
  under the same pinned toolchain versions.
- Scenario assertions validate both honest settlement (after deadline) and
  dishonest slashing (before deadline) paths.
- Challenge period timing is readable from contract state (`challengeDeadline`
  field and `challengePeriod()` view) so UI does not hardcode timers.
