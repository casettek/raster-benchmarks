# Raster Benchmarks Specs

This directory contains canonical behavioral and operational specs for `raster-benchmarks`.

## Scope and ownership boundaries

- `raster-benchmarks` owns end-to-end benchmark harness behavior, scenario contracts, workload bundle shape, and benchmark result formats.
- `raster-benchmarks` also owns benchmark-local smart contracts used by claimer/challenger scenarios.
- Core Raster protocol/library contracts remain owned by the `raster` repo; this repo references pinned Raster revisions when running benchmarks.
- Infra-heavy local scenario execution belongs here; small deterministic library/unit testing remains in `raster`.

## Spec map (current)

- `docs/specs/README.md` (this file): spec index, ownership boundaries, and baseline operational contracts for the MVP foundation phase.
- `docs/specs/program-bundle.md` (planned, Phase 1): bundle contract, identity fields, reproducibility rules.
- `docs/specs/run-schema.md` (implemented): canonical JSON schema for `runs/*.json` run output files — data contract between runner and UI.
- `docs/specs/local-e2e-scenarios.md` (planned, Phase 2): honest/dishonest flow assertions and expected outcomes.
- `docs/specs/metrics-schema.md` (planned, Phase 3): result/artifact schema and baseline comparison contract.
- `docs/specs/smart-contracts.md` (implemented): benchmark-local contract ownership, Foundry layout, and MVP claimer/challenger interaction surface.

## Phase 0 operational contracts

### Local execution baseline

- Local EVM dependency: `anvil` is the default chain runtime for benchmark scenarios.
- All phase-0 flows are local-only and must be reproducible without remote services.
- Required local setup is defined in `docs/local-setup.md`.

### Repository structure contract

Current active top-level directories in this lean baseline:

- `apps/`: runnable benchmark app entrypoints (claimer/challenger) that interact with local contracts.
- `apps/workloads/`: Raster-backed benchmark workloads executed by the runner.
- `contracts/`: benchmark-local smart contracts plus Foundry build/test/deploy scaffolding.
- `docs/`: canonical specs and setup contracts.
- `runs/`: local run-output landing area.
- `web/`: zero-dependency static HTML tools (open directly in a browser, no build step required).

`runs/artifacts/<run-id>/` is reserved for persisted Raster trace artifacts produced during real workload execution.

### Run metadata schema (phase-0 baseline)

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
