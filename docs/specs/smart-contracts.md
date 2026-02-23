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

## Claimer/challenger interaction contract (MVP baseline)

`contracts/src/interfaces/IClaimVerifier.sol` defines the baseline benchmark
interaction surface:

- `submitClaim(workloadId, artifactRoot, resultRoot)` creates a pending claim.
- `challengeClaim(claimId, observedArtifactRoot, observedResultRoot)` marks a
  claim slashed when divergence is observed.
- `settleClaim(claimId)` settles an uncontested pending claim.
- `getClaim(claimId)` exposes current claim state for harness assertions.

`contracts/src/ClaimVerifier.sol` is the benchmark reference implementation for
this baseline interface.

## Determinism requirements

- Contract behavior must be deterministic for identical claim/challenge inputs
  under the same pinned toolchain versions.
- Scenario assertions in later phases must validate both honest settlement and
  dishonest slashing paths against this interaction surface.
