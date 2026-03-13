# Golden Run Fixtures

These fixtures are stable `RunOutput` samples used by regression tests in
`crates/shared/tests/run_fixtures.rs`.

- `raster-hello-honest.json` - replay matches claim (`divergence.detected=false`)
- `raster-hello-dishonest.json` - replay diverges and trace fetch is required

The values are representative snapshots from local Anvil runs and are intended
for schema/contract validation, not gas benchmarking.
