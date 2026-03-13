use std::path::PathBuf;

use shared::run::RunOutput;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("runs")
        .join("fixtures")
        .join(name)
}

fn load_fixture(name: &str) -> RunOutput {
    let path = fixture_path(name);
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read fixture {}: {e}", path.display()));
    serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("invalid RunOutput fixture {}: {e}", path.display()))
}

#[test]
fn honest_fixture_matches_expected_replay_contract() {
    let run = load_fixture("raster-hello-honest.json");
    assert_eq!(run.workload, "raster-hello");
    assert_eq!(run.scenario, "honest");

    let exec = run.steps.iter().find(|step| step.key == "exec").unwrap();
    let trace = run.steps.iter().find(|step| step.key == "trace").unwrap();
    let da = run.steps.iter().find(|step| step.key == "da").unwrap();
    assert_eq!(exec.status, "done");
    assert_eq!(trace.status, "done");
    assert_eq!(da.status, "done");

    let divergence = run.summary.divergence.expect("missing divergence summary");
    assert!(!divergence.detected);
    assert_eq!(divergence.trace_fetch_status, "skipped");
    assert_eq!(run.summary.outcome, "settled");
}

#[test]
fn dishonest_fixture_requires_trace_fetch() {
    let run = load_fixture("raster-hello-dishonest.json");
    assert_eq!(run.workload, "raster-hello");
    assert_eq!(run.scenario, "dishonest");

    let replay = run.steps.iter().find(|step| step.key == "replay").unwrap();
    let outcome = run.steps.iter().find(|step| step.key == "outcome").unwrap();
    assert_eq!(replay.status, "done");
    assert_eq!(outcome.status, "slashed");

    let divergence = run.summary.divergence.expect("missing divergence summary");
    assert!(divergence.detected);
    assert_eq!(divergence.trace_fetch_status, "fetched");
    assert_eq!(run.summary.outcome, "slashed");
}
