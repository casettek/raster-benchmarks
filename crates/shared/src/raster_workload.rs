use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use eyre::{Context, Result, eyre};
use raster_core::postcard;
use raster_core::trace::{FnCallRecord, StepRecord};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

const TRACE_COMMITMENT_SCHEME: &str = "raster.trace_record.sha256.postcard.v1";
const TRACE_AGGREGATE_DOMAIN_SEPARATOR: &[u8] = b"raster.trace_commitment.v1\0";

pub struct RasterWorkloadResult {
    pub raster_revision: String,
    pub exec_time_ms: u64,
    pub trace_size_bytes: u64,
    pub trace_step_count: usize,
    pub trace_json_path: String,
    pub trace_ndjson_path: String,
    pub trace_commitment_size_bytes: u64,
    pub trace_commitment_path: String,
    pub trace_aggregate_commitment: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceCommitmentArtifact {
    pub scheme: String,
    pub item_count: usize,
    pub aggregate_commitment: String,
    pub item_commitments: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceCommitmentComparison {
    pub matches: bool,
    pub reason: String,
    pub first_divergence_index: Option<u64>,
}

pub fn warmup_known_workloads() -> Result<()> {
    for workload in ["raster-hello", "l2-kona-poc"] {
        let spec = workload_spec(workload)?;
        ensure_workload_binary(&spec)?;
    }
    Ok(())
}

pub fn run(workload: &str, run_id: &str) -> Result<Option<RasterWorkloadResult>> {
    let spec = workload_spec(workload)?;

    ensure_workload_binary(&spec)?;
    let input_json = workload_input_json(&spec)?;

    let start = Instant::now();
    let mut command = Command::new(spec.bin_path);
    command
        .current_dir(spec.run_dir)
        .arg("--input")
        .arg(input_json);

    if workload == "l2-kona-poc"
        && let Ok(mode) = std::env::var("L2_KONA_EXECUTION_MODE")
    {
        let trimmed = mode.trim();
        if !trimmed.is_empty() {
            command.arg("--execution-mode").arg(trimmed);
        }
    }

    let output = command
        .output()
        .wrap_err("failed to execute Raster workload")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(eyre!(
            "Raster workload process failed with status {}: {}",
            output.status,
            stderr.trim()
        ));
    }

    let stdout =
        String::from_utf8(output.stdout).wrap_err("workload stdout was not valid UTF-8")?;
    let trace_records: Vec<Value> = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("[trace]"))
        .map(serde_json::from_str::<Value>)
        .collect::<serde_json::Result<_>>()
        .wrap_err("failed to parse workload trace records")?;

    if trace_records.is_empty() {
        return Err(eyre!(
            "Raster workload completed without emitting trace records"
        ));
    }

    let artifact_dir = PathBuf::from("runs").join("artifacts").join(run_id);
    std::fs::create_dir_all(&artifact_dir).wrap_err("failed to create trace artifact directory")?;

    let trace_json_path = artifact_dir.join("trace.json");
    let trace_ndjson_path = artifact_dir.join("trace.ndjson");
    let trace_commitment_path = artifact_dir.join("trace.commitment.json");

    let trace_json = serde_json::to_string_pretty(&trace_records)?;
    std::fs::write(&trace_json_path, trace_json.as_bytes())
        .wrap_err("failed to write trace.json artifact")?;

    let mut ndjson_buf = String::new();
    for record in &trace_records {
        let line = serde_json::to_string(record)?;
        ndjson_buf.push_str(&line);
        ndjson_buf.push('\n');
    }
    std::fs::write(&trace_ndjson_path, ndjson_buf.as_bytes())
        .wrap_err("failed to write trace.ndjson artifact")?;

    let trace_commitment_artifact = build_trace_commitment_artifact(&trace_records)?;
    let trace_commitment_json = serde_json::to_string_pretty(&trace_commitment_artifact)?;
    std::fs::write(&trace_commitment_path, trace_commitment_json.as_bytes())
        .wrap_err("failed to write trace.commitment.json artifact")?;

    let elapsed = start.elapsed().as_millis();
    let exec_time_ms = elapsed.min(u64::MAX as u128) as u64;

    Ok(Some(RasterWorkloadResult {
        raster_revision: raster_revision(),
        exec_time_ms,
        trace_size_bytes: ndjson_buf.len() as u64,
        trace_step_count: trace_records.len(),
        trace_json_path: trace_json_path.to_string_lossy().to_string(),
        trace_ndjson_path: trace_ndjson_path.to_string_lossy().to_string(),
        trace_commitment_size_bytes: trace_commitment_json.len() as u64,
        trace_commitment_path: trace_commitment_path.to_string_lossy().to_string(),
        trace_aggregate_commitment: trace_commitment_artifact.aggregate_commitment,
    }))
}

fn workload_spec(workload: &str) -> Result<WorkloadSpec> {
    match workload {
        "raster-hello" => Ok(WorkloadSpec {
            run_dir: "apps/workloads/raster-hello",
            bin_path: "../../../target/debug/workload-raster-hello",
            input: WorkloadInput::Inline("\"Raster\""),
        }),
        "l2-kona-poc" => Ok(WorkloadSpec {
            run_dir: "apps/workloads/l2-kona-poc",
            bin_path: "../../../target/debug/workload-l2-kona-poc",
            input: WorkloadInput::FixtureFile("runs/fixtures/l2-poc-synth-fixture.json"),
        }),
        _ => Err(eyre!(
            "Unknown workload '{}'. Expected 'raster-hello' or 'l2-kona-poc'.",
            workload
        )),
    }
}

fn workload_input_json(spec: &WorkloadSpec) -> Result<String> {
    match spec.input {
        WorkloadInput::Inline(value) => Ok(value.to_string()),
        WorkloadInput::FixtureFile(path) => std::fs::read_to_string(path)
            .wrap_err_with(|| format!("failed to load fixture input JSON from {path}")),
    }
}

fn ensure_workload_binary(spec: &WorkloadSpec) -> Result<()> {
    let bin_abs = Path::new(spec.run_dir).join(spec.bin_path);
    if bin_abs.exists() {
        return Ok(());
    }

    let build_output = Command::new("cargo")
        .current_dir(spec.run_dir)
        .env("RISC0_SKIP_BUILD", "1")
        .args(["build", "--quiet"])
        .output()
        .wrap_err("failed to build Raster workload")?;

    if !build_output.status.success() {
        let stderr = String::from_utf8_lossy(&build_output.stderr);
        return Err(eyre!(
            "Raster workload build failed with status {}: {}",
            build_output.status,
            stderr.trim()
        ));
    }

    Ok(())
}

pub fn load_trace_payload(result: &RasterWorkloadResult) -> Result<Vec<u8>> {
    std::fs::read(&result.trace_ndjson_path)
        .wrap_err("failed to load trace payload for DA publication")
}

pub fn load_trace_commitment_payload(result: &RasterWorkloadResult) -> Result<Vec<u8>> {
    std::fs::read(&result.trace_commitment_path)
        .wrap_err("failed to load trace commitment payload for DA publication")
}

pub fn decode_trace_commitment_payload(payload: &[u8]) -> Result<TraceCommitmentArtifact> {
    serde_json::from_slice(payload).wrap_err("failed to decode trace commitment payload")
}

pub fn compare_trace_commitments(
    expected: &TraceCommitmentArtifact,
    observed: &TraceCommitmentArtifact,
) -> TraceCommitmentComparison {
    if expected.scheme != observed.scheme {
        return TraceCommitmentComparison {
            matches: false,
            reason: format!(
                "Trace commitment scheme mismatch: published {}, local {}",
                expected.scheme, observed.scheme
            ),
            first_divergence_index: None,
        };
    }

    if let Some(index) = first_divergence_index(expected, observed) {
        return TraceCommitmentComparison {
            matches: false,
            reason: "Local replay trace commitment differs from published trace commitment"
                .to_string(),
            first_divergence_index: Some(index),
        };
    }

    if expected.aggregate_commitment != observed.aggregate_commitment {
        return TraceCommitmentComparison {
            matches: false,
            reason: "Trace commitment aggregate digest differs despite matching item list"
                .to_string(),
            first_divergence_index: None,
        };
    }

    TraceCommitmentComparison {
        matches: true,
        reason: "Local replay matched published trace commitment".to_string(),
        first_divergence_index: None,
    }
}

pub fn rerun_trace_commitment(workload: &str, label: &str) -> Result<TraceCommitmentArtifact> {
    let run_id = format!(
        "audit-{}-{}-{}",
        workload,
        label,
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    );
    let result = run(workload, &run_id)?
        .ok_or_else(|| eyre!("Raster workload rerun returned no trace commitment"))?;
    let payload = load_trace_commitment_payload(&result)?;
    decode_trace_commitment_payload(&payload)
}

pub fn exec_step_metrics(result: &RasterWorkloadResult, workload: &str) -> HashMap<String, String> {
    HashMap::from([
        ("Workload".to_string(), workload.to_string()),
        (
            "Exec time (ms)".to_string(),
            result.exec_time_ms.to_string(),
        ),
        (
            "Trace steps".to_string(),
            result.trace_step_count.to_string(),
        ),
        (
            "Trace commitment".to_string(),
            result.trace_aggregate_commitment.clone(),
        ),
        (
            "Trace commitment size (bytes)".to_string(),
            result.trace_commitment_size_bytes.to_string(),
        ),
        (
            "Trace commitment file".to_string(),
            result.trace_commitment_path.clone(),
        ),
    ])
}

pub fn trace_step_metrics(result: &RasterWorkloadResult) -> HashMap<String, String> {
    HashMap::from([
        (
            "Trace size (bytes)".to_string(),
            result.trace_size_bytes.to_string(),
        ),
        ("Trace file".to_string(), result.trace_json_path.clone()),
        (
            "Trace commitment".to_string(),
            result.trace_aggregate_commitment.clone(),
        ),
        (
            "Trace commitment size (bytes)".to_string(),
            result.trace_commitment_size_bytes.to_string(),
        ),
        (
            "Trace commitment file".to_string(),
            result.trace_commitment_path.clone(),
        ),
        (
            "Raster revision".to_string(),
            result.raster_revision.clone(),
        ),
    ])
}

fn build_trace_commitment_artifact(records: &[Value]) -> Result<TraceCommitmentArtifact> {
    let mut item_commitments = Vec::with_capacity(records.len());
    let mut aggregate_hasher = Sha256::new();
    aggregate_hasher.update(TRACE_AGGREGATE_DOMAIN_SEPARATOR);

    for record in records {
        let encoded = trace_record_commitment_bytes(record)?;
        let item_commitment = Sha256::digest(&encoded);
        aggregate_hasher.update(item_commitment);
        item_commitments.push(format!("0x{}", alloy::hex::encode(item_commitment)));
    }

    Ok(TraceCommitmentArtifact {
        scheme: TRACE_COMMITMENT_SCHEME.to_string(),
        item_count: records.len(),
        aggregate_commitment: format!("0x{}", alloy::hex::encode(aggregate_hasher.finalize())),
        item_commitments,
    })
}

fn first_divergence_index(
    expected: &TraceCommitmentArtifact,
    observed: &TraceCommitmentArtifact,
) -> Option<u64> {
    let shared_len = expected
        .item_commitments
        .len()
        .min(observed.item_commitments.len());
    for index in 0..shared_len {
        if expected.item_commitments[index] != observed.item_commitments[index] {
            return Some(index as u64);
        }
    }

    if expected.item_commitments.len() != observed.item_commitments.len()
        || expected.item_count != observed.item_count
    {
        return Some(shared_len as u64);
    }

    None
}

fn trace_record_commitment_bytes(record: &Value) -> Result<Vec<u8>> {
    if let Ok(step_record) = serde_json::from_value::<StepRecord>(record.clone()) {
        return postcard::to_allocvec(&step_record)
            .wrap_err("failed to postcard-encode StepRecord trace item");
    }

    if let Ok(fn_call_record) = serde_json::from_value::<FnCallRecord>(record.clone()) {
        return postcard::to_allocvec(&fn_call_record)
            .wrap_err("failed to postcard-encode FnCallRecord trace item");
    }

    Err(eyre!(
        "unsupported Raster trace item shape for commitment generation"
    ))
}

fn raster_revision() -> String {
    let output = Command::new("git")
        .args(["-C", "../raster", "rev-parse", "HEAD"])
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let rev = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if rev.is_empty() {
                "path:../raster".to_string()
            } else {
                rev
            }
        }
        _ => "path:../raster".to_string(),
    }
}

struct WorkloadSpec {
    run_dir: &'static str,
    bin_path: &'static str,
    input: WorkloadInput,
}

#[derive(Clone, Copy)]
enum WorkloadInput {
    Inline(&'static str),
    FixtureFile(&'static str),
}

#[cfg(test)]
mod tests {
    use super::*;
    use raster_core::trace::FnInputParam;

    fn sample_record(name: &str, output: &[u8]) -> FnCallRecord {
        FnCallRecord {
            fn_name: name.to_string(),
            desc: None,
            inputs: vec![FnInputParam {
                name: "value".to_string(),
                ty: "u64".to_string(),
            }],
            input_data: vec![1, 2, 3],
            output_type: Some("u64".to_string()),
            output_data: output.to_vec(),
        }
    }

    #[test]
    fn trace_commitment_artifact_is_deterministic() {
        let records = vec![
            serde_json::to_value(sample_record("first", &[0x01])).expect("serialize first record"),
            serde_json::to_value(sample_record("second", &[0x02]))
                .expect("serialize second record"),
        ];

        let first = build_trace_commitment_artifact(&records).expect("build commitment");
        let second = build_trace_commitment_artifact(&records).expect("build commitment");

        assert_eq!(first.scheme, TRACE_COMMITMENT_SCHEME);
        assert_eq!(first.item_count, 2);
        assert_eq!(first.item_commitments.len(), 2);
        assert_eq!(first.aggregate_commitment, second.aggregate_commitment);
        assert_eq!(first.item_commitments, second.item_commitments);
    }

    #[test]
    fn trace_commitment_comparison_reports_first_diff() {
        let expected = TraceCommitmentArtifact {
            scheme: TRACE_COMMITMENT_SCHEME.to_string(),
            item_count: 3,
            aggregate_commitment: "0xabc".to_string(),
            item_commitments: vec![
                "0x01".to_string(),
                "0x02".to_string(),
                "0x03".to_string(),
            ],
        };
        let observed = TraceCommitmentArtifact {
            scheme: TRACE_COMMITMENT_SCHEME.to_string(),
            item_count: 3,
            aggregate_commitment: "0xdef".to_string(),
            item_commitments: vec![
                "0x01".to_string(),
                "0xff".to_string(),
                "0x03".to_string(),
            ],
        };

        let comparison = compare_trace_commitments(&expected, &observed);
        assert!(!comparison.matches);
        assert_eq!(comparison.first_divergence_index, Some(1));
    }
}
