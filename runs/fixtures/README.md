# Golden Run Fixtures

These fixtures are stable `RunOutput` samples used by regression tests in
`crates/shared/tests/run_fixtures.rs`.

- `raster-hello-honest.json` - replay matches claim (`divergence.detected=false`)
- `raster-hello-dishonest.json` - replay diverges and trace fetch is required
- `l2-poc-synth-fixture.json` - canonical repo-owned synthetic L2 fixture input contract (seeded checkpoint + 5 tracked txs + 5 supplemental block txs + seeded output-root witness)
- `l2-poc-synth-fixture.json` is consumed directly by workload id `l2-kona-poc` via `--input` for the plan-008.6 canonical strict path
- `l2-poc-plan7-fixture.json` remains as the legacy bootstrap/reference package used to generate the synthetic canonical fixture
- Local witness artifacts for the L2 fixture live under `fixtures/l2-poc/`:
  - `rollup-config-v1.json`
  - `synthetic-fixture-seed-v1.json`
  - `synthetic-witness-bundle-v1.json`
  - `synthetic-witness-closure-manifest-v1.json` (deterministic fixture/witness commitment)
  - `synthetic-witness-kv-v1*` repo-owned witness snapshots used by the canonical synthetic package

Regenerate the canonical synthetic package with:

```bash
python3 scripts/generate_l2_poc_synthetic_fixture.py --force
```

Refresh the witness closure manifest alone with:

```bash
python3 scripts/generate_l2_poc_witness_manifest.py
```

Run strict canonical acceptance (double-run determinism + strict Kona status):

```bash
scripts/check_l2_kona_strict.sh
```

The canonical fixture pins a complete execution package for the benchmarked 5
txs: the five tracked txs, the five additional canonical block txs needed to
close the block without skipping execution, and a seeded
`message_passer_storage_root` used for deterministic output-root hashing inside
the workload.
