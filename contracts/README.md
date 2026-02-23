# Smart Contracts

This directory owns benchmark-local smart contracts used by claimer/challenger
scenario tests.

## Why contracts live here

- Benchmark iteration speed: contract changes should ship with scenario updates.
- Local determinism: contracts are exercised against local `anvil` runs.
- Clear ownership: benchmark harness behavior and benchmark-specific contracts
  are versioned together in this repo.

## Layout

- `src/`: Solidity sources used by benchmark scenarios.
- `script/`: deploy/setup scripts for local runs.
- `test/`: Solidity-level contract tests.
- `foundry.toml`: Foundry project config for this contracts package.

## Local commands

- Build: `forge build` (run from `contracts/`)
- Test: `forge test` (run from `contracts/`)

These contracts are benchmark harness contracts and are not intended to define
the canonical Raster protocol interface for production.
