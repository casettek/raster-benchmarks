# Run Fixtures

## Raster Hello fixtures

Used by regression tests in `crates/shared/tests/run_fixtures.rs`:

- `raster-hello-honest.json` — replay matches claim (`divergence.detected=false`)
- `raster-hello-dishonest.json` — replay diverges and trace fetch is required

## L2 Kona POC fixture

- `l2-poc-synth-fixture.json` — canonical synthetic L2 fixture input (seeded
  checkpoint + 5 tracked txs + 5 supplemental block txs + seeded output-root
  witness). Consumed by workload `l2-kona-poc` via `--input`.
- `l2-poc-synth-chunk-plan-v1.json` — chunk-plan sidecar that partitions the
  10 execution txs into deterministic one-tx replay tiles.

Local witness artifacts live under `fixtures/l2-poc/`:

- `rollup-config-v1.json`
- `synthetic-fixture-seed-v1.json`
- `synthetic-witness-bundle-v1.json`
- `synthetic-witness-closure-manifest-v1.json`
- `synthetic-witness-kv-v1*` — repo-owned witness snapshots

## Commands

Regenerate the canonical synthetic package:

```bash
python3 scripts/generate_l2_poc_synthetic_fixture.py --force
```

Refresh the witness closure manifest:

```bash
python3 scripts/generate_l2_poc_witness_manifest.py
```

Refresh the chunk-plan sidecar:

```bash
scripts/generate_l2_poc_chunk_plan.sh
```

Run strict canonical acceptance (double-run determinism + 10 Raster-native
tile traces):

```bash
scripts/check_l2_kona_strict.sh
```
