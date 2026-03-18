#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FIXTURE_PATH="${ROOT}/runs/fixtures/l2-poc-synth-fixture.json"
OUTPUT_PATH="${ROOT}/runs/fixtures/l2-poc-synth-chunk-plan-v1.json"

FIXTURE_JSON="$(python3 - "$FIXTURE_PATH" <<'PY'
import json
import pathlib
import sys

fixture_path = pathlib.Path(sys.argv[1])
print(json.dumps(json.loads(fixture_path.read_text())))
PY
)"

cargo run -q -p workload-l2-kona-poc --manifest-path "${ROOT}/Cargo.toml" -- \
  --emit-chunk-plan \
  --chunk-size 1 \
  --input "${FIXTURE_JSON}" > "${OUTPUT_PATH}"

echo "wrote ${OUTPUT_PATH}"
