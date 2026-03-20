use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

use eyre::{Context, Result, eyre};
use serde_json::Value;

pub struct RasterWorkloadResult {
    pub raster_revision: String,
    pub exec_time_ms: u64,
    pub trace_size_bytes: u64,
    pub trace_step_count: usize,
    pub trace_json_path: String,
    pub trace_ndjson_path: String,
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
    let mut trace_records: Vec<Value> = stdout
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

    trace_records.sort_by_key(|record| {
        record
            .get("exec_index")
            .and_then(Value::as_u64)
            .unwrap_or(u64::MAX)
    });

    let artifact_dir = PathBuf::from("runs").join("artifacts").join(run_id);
    std::fs::create_dir_all(&artifact_dir).wrap_err("failed to create trace artifact directory")?;

    let trace_json_path = artifact_dir.join("trace.json");
    let trace_ndjson_path = artifact_dir.join("trace.ndjson");

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

    let elapsed = start.elapsed().as_millis();
    let exec_time_ms = elapsed.min(u64::MAX as u128) as u64;

    Ok(Some(RasterWorkloadResult {
        raster_revision: raster_revision(),
        exec_time_ms,
        trace_size_bytes: ndjson_buf.len() as u64,
        trace_step_count: trace_records.len(),
        trace_json_path: trace_json_path.to_string_lossy().to_string(),
        trace_ndjson_path: trace_ndjson_path.to_string_lossy().to_string(),
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
            "Raster revision".to_string(),
            result.raster_revision.clone(),
        ),
    ])
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
