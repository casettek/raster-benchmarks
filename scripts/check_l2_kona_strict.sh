#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FIXTURE_PATH="$ROOT_DIR/runs/fixtures/l2-poc-synth-fixture.json"
TMP_ONE="$(mktemp)"
TMP_TWO="$(mktemp)"

cleanup() {
  rm -f "$TMP_ONE" "$TMP_TWO"
}
trap cleanup EXIT

run_once() {
  local output_file="$1"
  cargo run -q -p workload-l2-kona-poc -- --execution-mode strict --input "$(python3 -c 'import json, pathlib, sys; print(json.dumps(json.loads(pathlib.Path(sys.argv[1]).read_text())))' "$FIXTURE_PATH")" >"$output_file"
}

run_once "$TMP_ONE"
run_once "$TMP_TWO"

python3 - "$TMP_ONE" "$TMP_TWO" <<'PY'
import json
import sys
from pathlib import Path


def parse_traces(path: Path):
    traces = []
    for line in path.read_text().splitlines():
        if not line.startswith("[trace]"):
            continue
        traces.append(json.loads(line[len("[trace]"):]))
    return traces


def roots(traces):
    return [entry["next_output_root"] for entry in traces]


run_one = parse_traces(Path(sys.argv[1]))
run_two = parse_traces(Path(sys.argv[2]))

if len(run_one) != 1 or len(run_two) != 1:
    raise SystemExit("strict check failed: expected exactly 1 trace per run")

trace_one = run_one[0]
trace_two = run_two[0]

if trace_one.get("output_root_status") != "fixture_output_root":
    raise SystemExit(
        f"strict check failed: run-1 trace has status {trace_one.get('output_root_status')!r}"
    )

if trace_two.get("output_root_status") != "fixture_output_root":
    raise SystemExit(
        f"strict check failed: run-2 trace has status {trace_two.get('output_root_status')!r}"
    )

if trace_one.get("tracked_tx_count") != 5 or trace_two.get("tracked_tx_count") != 5:
    raise SystemExit("strict check failed: canonical fixture must track exactly 5 target txs")

if trace_one.get("execution_tx_count") != 10 or trace_two.get("execution_tx_count") != 10:
    raise SystemExit(
        "strict check failed: canonical fixture must execute all 10 block txs"
    )

if roots(run_one) != roots(run_two):
    raise SystemExit("strict check failed: ordered next_output_root values differ across runs")

print("strict check passed")
print(f"final_next_output_root={run_one[-1]['next_output_root']}")
PY
