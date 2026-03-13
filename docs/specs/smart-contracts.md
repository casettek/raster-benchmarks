# Benchmark Smart Contracts Spec

This spec defines benchmark-local smart-contract ownership and the minimal
claimer/challenger interaction surface for `raster-benchmarks`.

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
- These apps are the harness-facing entrypoints that will call `IClaimVerifier`
  functions as scenario implementations mature.

## Runtime baseline

- Local chain: `anvil`
- Contract toolchain: `forge` (Foundry)
- Contract flows are expected to run in local deterministic scenarios before
  any testnet adaptation.

## Claimer/challenger interaction contract (Phase 2)

`contracts/src/interfaces/IClaimVerifier.sol` defines the baseline benchmark
interaction surface:

- `publishTrace(payload, codecId)` publishes trace payload bytes through a dedicated
  on-chain tx path and emits `TracePublished` with payload hash/size metadata.
- `submitClaim(workloadId, artifactRoot, resultRoot, traceTxHash, tracePayloadBytes, traceCodecId)`
  creates a pending claim and stores a trace pointer for challenger replay.
- `challengeClaim(claimId, observedArtifactRoot, observedResultRoot)` marks a
  claim slashed when divergence is observed.
- `settleClaim(claimId)` settles an uncontested pending claim.
- `getClaim(claimId)` exposes current claim state for harness assertions.

`contracts/src/ClaimVerifier.sol` is the benchmark reference implementation for
this baseline interface.

## Claim metadata pointer fields

`Claim` now includes DA pointer fields used by challenger retrieval in later phases:

- `traceTxHash` (`bytes32`) - hash of the tx that published the trace payload.
- `tracePayloadBytes` (`uint32`) - payload byte length for decode validation.
- `traceCodecId` (`uint8`) - trace codec discriminator (`1` = `trace.ndjson` v1).

For non-DA paths (for example `workload=stub`), these fields are zeroed.

## Determinism requirements

- Contract behavior must be deterministic for identical claim/challenge inputs
  under the same pinned toolchain versions.
- Scenario assertions in later phases must validate both honest settlement and
  dishonest slashing paths against this interaction surface.
