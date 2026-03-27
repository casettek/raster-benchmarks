# Raster Benchmarks Specs

This directory contains canonical behavioral and operational specs for `raster-benchmarks`.

## Scope and ownership boundaries

- `raster-benchmarks` owns end-to-end benchmark harness behavior, scenario contracts, workload bundle shape, and benchmark result formats.
- `raster-benchmarks` also owns benchmark-local smart contracts used by claimer/challenger scenarios.
- Core Raster protocol/library contracts remain owned by the `raster` repo; this repo references pinned Raster revisions when running benchmarks.
- Infra-heavy local scenario execution belongs here; small deterministic library/unit testing remains in `raster`.

## Spec map

- `docs/specs/README.md` (this file): spec index and ownership boundaries.
- `docs/specs/run-schema.md`: canonical JSON schema for `runs/*.json` run output files — data contract between runner and UI. Covers both legacy (`raster-hello`) and L2 (`l2-kona-poc`) step lifecycles.
- `docs/specs/l2-kona-workload.md`: the L2 Kona POC workload — Raster program shape (`#[sequence]` + `#[tile]`), execution boundary, chunk plan, trace format, fixture contract, witness closure manifest, and acceptance gate.
- `docs/specs/local-e2e-scenarios.md`: canonical 5-transaction benchmark fixture, Raster program shape, replay chunk plan, and honest/dishonest scenario assertions.
- `docs/specs/smart-contracts.md`: L2 settlement contract — claim object (outputRoot transition, bond, challenge deadline), state machine, and explicit input/trace blob versioned-hash bindings.

## Repository structure

- `apps/` — runnable benchmark app entrypoints (claimer/challenger) that interact with local contracts.
- `apps/workloads/` — Raster-backed benchmark workloads executed by the runner.
  - `l2-kona-poc/` — L2 POC workload: one Raster program (`l2_block_execution` sequence calling `execute_chunk` tile 10 times) backed by a Kona EVM chunk driver.
  - `raster-hello/` — minimal Raster workload for integration testing.
- `contracts/` — benchmark-local smart contracts plus Foundry build/test/deploy scaffolding.
- `crates/` — shared Rust libraries (workload adapter, runner integration).
- `docs/` — canonical specs and setup contracts.
- `fixtures/` — witness data and seed metadata for deterministic fixture regeneration.
- `runs/` — run output landing area and canonical fixture files.
- `scripts/` — generation and validation scripts (fixture, chunk plan, strict check).
- `web/` — zero-dependency static HTML tools (open directly in a browser, no build step required).

## Operational contracts

### Local execution baseline

- Local EVM dependency: `anvil` is the default chain runtime for benchmark scenarios.
- All flows are local-only and must be reproducible without remote services.
- Required local setup is defined in `docs/local-setup.md`.

### Run metadata schema

Each benchmark run MUST emit metadata that includes exact Raster pin information:

```json
{
  "run_id": "string",
  "timestamp_utc": "RFC3339 string",
  "workload_id": "string",
  "scenario_id": "string",
  "local_chain": {
    "engine": "anvil",
    "version": "string"
  },
  "raster_pin": {
    "repository": "string",
    "revision": "full git commit sha",
    "workspace_lock_hash": "sha256 hex string",
    "toolchain": "string"
  },
  "runner": {
    "host_os": "string",
    "rustc_version": "string"
  }
}
```

`raster_pin.revision` is required and must be a full commit SHA (not a branch name or floating tag).
