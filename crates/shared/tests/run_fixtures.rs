use std::path::PathBuf;

use shared::run::RunOutput;

// --- L2 lifecycle step keys ---
const L2_STEP_KEYS: &[&str] = &[
    "prepare",
    "exec",
    "da",
    "claim",
    "audit",
    "await-finalization",
    "outcome",
];

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

// --- L2 Kona POC golden fixture tests ---

#[test]
fn l2_honest_fixture_has_correct_lifecycle_steps() {
    let run = load_fixture("l2-kona-poc-honest.json");
    assert_eq!(run.workload, "l2-kona-poc");
    assert_eq!(run.scenario, "honest");

    let step_keys: Vec<&str> = run.steps.iter().map(|s| s.key.as_str()).collect();
    assert_eq!(step_keys, L2_STEP_KEYS);

    // All steps should be done (outcome is "settled")
    for step in &run.steps {
        match step.key.as_str() {
            "outcome" => assert_eq!(step.status, "settled"),
            _ => assert_eq!(step.status, "done", "step {} should be done", step.key),
        }
    }
}

#[test]
fn l2_honest_fixture_has_l2_summary_metadata() {
    let run = load_fixture("l2-kona-poc-honest.json");

    // L2 summary metadata must be present
    assert!(
        run.summary.prev_output_root.is_some(),
        "missing prev_output_root"
    );
    assert!(
        run.summary.next_output_root.is_some(),
        "missing next_output_root"
    );
    assert!(run.summary.start_block.is_some(), "missing start_block");
    assert!(run.summary.end_block.is_some(), "missing end_block");
    assert!(run.summary.batch_hash.is_some(), "missing batch_hash");
    assert!(run.summary.bond_amount.is_some(), "missing bond_amount");
    assert!(
        run.summary.challenge_deadline.is_some(),
        "missing challenge_deadline"
    );
    assert!(
        run.summary.challenge_period_seconds.is_some(),
        "missing challenge_period_seconds"
    );

    // Canonical values
    assert_eq!(run.summary.start_block, Some(26207960));
    assert_eq!(run.summary.end_block, Some(26207960));
    assert_eq!(
        run.summary.batch_hash.as_deref(),
        Some("0xb9ef076572948183c38d75a6b8966236c1030c83c0e6ab50b813266de50be229")
    );
    assert_eq!(run.summary.challenge_period_seconds, Some(120));

    // Honest = settled with no divergence
    let divergence = run.summary.divergence.expect("missing divergence");
    assert!(!divergence.detected);
    assert_eq!(divergence.trace_fetch_status, "skipped");
    assert_eq!(run.summary.outcome, "settled");
}

#[test]
fn l2_dishonest_fixture_has_correct_lifecycle_steps() {
    let run = load_fixture("l2-kona-poc-dishonest.json");
    assert_eq!(run.workload, "l2-kona-poc");
    assert_eq!(run.scenario, "dishonest");

    let step_keys: Vec<&str> = run.steps.iter().map(|s| s.key.as_str()).collect();
    assert_eq!(step_keys, L2_STEP_KEYS);

    let outcome = run.steps.iter().find(|s| s.key == "outcome").unwrap();
    assert_eq!(outcome.status, "slashed");
}

#[test]
fn l2_dishonest_fixture_has_divergence_and_trace_fetch() {
    let run = load_fixture("l2-kona-poc-dishonest.json");

    let divergence = run.summary.divergence.expect("missing divergence");
    assert!(divergence.detected);
    assert_eq!(divergence.trace_fetch_status, "fetched");
    assert!(divergence.first_divergence_index.is_some());
    assert_eq!(run.summary.outcome, "slashed");

    // Audit step should show divergence detected
    let audit = run.steps.iter().find(|s| s.key == "audit").unwrap();
    assert_eq!(
        audit.metrics.get("Divergence").map(String::as_str),
        Some("Detected")
    );
    assert_eq!(
        audit.metrics.get("Trace fetch").map(String::as_str),
        Some("fetched")
    );

    // Await-finalization should show challenged before deadline
    let await_fin = run
        .steps
        .iter()
        .find(|s| s.key == "await-finalization")
        .unwrap();
    assert_eq!(
        await_fin.metrics.get("Status").map(String::as_str),
        Some("Challenged before deadline")
    );
}

#[test]
fn l2_honest_fixture_prepare_step_has_batch_metadata() {
    let run = load_fixture("l2-kona-poc-honest.json");

    let prepare = run.steps.iter().find(|s| s.key == "prepare").unwrap();
    assert_eq!(prepare.status, "done");
    assert!(
        prepare.metrics.contains_key("Fixture"),
        "missing Fixture metric"
    );
    assert!(
        prepare.metrics.contains_key("Batch hash"),
        "missing Batch hash metric"
    );
    assert!(
        prepare.metrics.contains_key("Block range"),
        "missing Block range metric"
    );
}

#[test]
fn legacy_fixtures_have_no_l2_summary_metadata() {
    let run = load_fixture("raster-hello-honest.json");
    assert!(run.summary.prev_output_root.is_none());
    assert!(run.summary.next_output_root.is_none());
    assert!(run.summary.start_block.is_none());
    assert!(run.summary.end_block.is_none());
    assert!(run.summary.batch_hash.is_none());
    assert!(run.summary.challenge_deadline.is_none());
    assert!(run.summary.challenge_period_seconds.is_none());
}
