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


def parse_output(path: Path):
    traces = []
    summary = None
    for line in path.read_text().splitlines():
        if line.startswith("[trace]"):
            traces.append(json.loads(line[len("[trace]"):]))
        elif line.startswith("[summary]"):
            summary = json.loads(line[len("[summary]"):])
    return traces, summary


run_one_traces, run_one_summary = parse_output(Path(sys.argv[1]))
run_two_traces, run_two_summary = parse_output(Path(sys.argv[2]))

# Validate trace count: 10 Raster-native tile traces per run
if len(run_one_traces) != 10 or len(run_two_traces) != 10:
    raise SystemExit(
        f"strict check failed: expected 10 trace records per run "
        f"(got {len(run_one_traces)} and {len(run_two_traces)})"
    )

# Validate Raster-native trace format: each trace has fn_name
for i, trace in enumerate(run_one_traces):
    if "fn_name" not in trace:
        raise SystemExit(
            f"strict check failed: run-1 trace {i} is missing fn_name "
            f"(not a Raster-native trace record)"
        )

for i, trace in enumerate(run_two_traces):
    if "fn_name" not in trace:
        raise SystemExit(
            f"strict check failed: run-2 trace {i} is missing fn_name "
            f"(not a Raster-native trace record)"
        )

# Validate tile function names: all 10 tiles use the same execute_chunk function
expected_fn_names = ["execute_chunk"] * 10
actual_fn_names_1 = [t["fn_name"] for t in run_one_traces]
actual_fn_names_2 = [t["fn_name"] for t in run_two_traces]

if actual_fn_names_1 != expected_fn_names:
    raise SystemExit(
        f"strict check failed: run-1 tile function names do not match expected "
        f"(got {actual_fn_names_1})"
    )

if actual_fn_names_2 != expected_fn_names:
    raise SystemExit(
        f"strict check failed: run-2 tile function names do not match expected "
        f"(got {actual_fn_names_2})"
    )

# Validate summary presence
if run_one_summary is None or run_two_summary is None:
    raise SystemExit("strict check failed: expected [summary] line in output")

# Validate summary fields
if run_one_summary.get("output_root_status") != "fixture_output_root":
    raise SystemExit(
        f"strict check failed: run-1 summary has status "
        f"{run_one_summary.get('output_root_status')!r}"
    )

if run_two_summary.get("output_root_status") != "fixture_output_root":
    raise SystemExit(
        f"strict check failed: run-2 summary has status "
        f"{run_two_summary.get('output_root_status')!r}"
    )

if run_one_summary.get("tracked_tx_count") != 5:
    raise SystemExit("strict check failed: canonical fixture must track exactly 5 target txs")

if run_one_summary.get("execution_tx_count") != 10:
    raise SystemExit("strict check failed: canonical fixture must execute all 10 block txs")

if run_one_summary.get("tile_count") != 10:
    raise SystemExit("strict check failed: canonical execution must produce 10 tile traces")

# Validate determinism: same output root across runs
if run_one_summary.get("next_output_root") != run_two_summary.get("next_output_root"):
    raise SystemExit(
        "strict check failed: next_output_root differs across runs "
        f"({run_one_summary.get('next_output_root')} vs "
        f"{run_two_summary.get('next_output_root')})"
    )

# Validate Raster trace determinism: serialized outputs should match
for i in range(10):
    if run_one_traces[i].get("output_data") != run_two_traces[i].get("output_data"):
        raise SystemExit(
            f"strict check failed: trace {i} output_data differs across runs"
        )

print("strict check passed")
print(f"final_next_output_root={run_one_summary['next_output_root']}")
print(f"tile_fn_name=execute_chunk (x10)")
PY
