use std::collections::HashMap;
use std::path::PathBuf;

use chrono::Utc;
use clap::Parser;
use eyre::Result;
use shared::challenger::ReplayMode;
use shared::raster_workload;
use shared::run::{DivergenceSummary, RasterPin, RunOutput, StepOutput, SummaryOutput};

#[derive(Parser)]
#[command(
    name = "runner",
    about = "Orchestrate a claimer → challenger scenario run"
)]
struct Cli {
    /// Scenario to run: "honest" or "dishonest"
    #[arg(long)]
    scenario: String,

    /// Workload name (stub only — no behavioral effect in this phase)
    #[arg(long, default_value = "stub")]
    workload: String,

    /// Path to Foundry build output directory
    #[arg(long, default_value = "contracts/out")]
    forge_out: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    if cli.scenario != "honest" && cli.scenario != "dishonest" {
        return Err(eyre::eyre!(
            "Unknown scenario '{}'. Expected 'honest' or 'dishonest'.",
            cli.scenario
        ));
    }

    let forge_out = PathBuf::from(&cli.forge_out);
    let timestamp = Utc::now();
    let run_id = format!(
        "{}-{}-{}",
        timestamp.format("%Y-%m-%dT%H-%M-%S"),
        cli.workload,
        cli.scenario
    );

    // 1. Execute Raster workload when requested
    let raster_workload_result = raster_workload::run(&cli.workload, &run_id)?;

    // 2. Start chain
    eprintln!("Spawning Anvil...");
    let (_anvil, provider) = shared::anvil::spawn_anvil()?;

    // 3. Deploy contract
    eprintln!("Deploying ClaimVerifier from {}...", forge_out.display());
    let contract_address = shared::deploy::deploy_claim_verifier(&provider, &forge_out).await?;
    eprintln!("ClaimVerifier deployed at {contract_address}");

    // 4. Claimer step — submit claim with stub roots
    let da_publication = if let Some(result) = &raster_workload_result {
        let trace_payload = raster_workload::load_trace_payload(result)?;
        let publication = shared::da::publish_trace(
            &provider,
            contract_address,
            trace_payload,
            shared::da::TRACE_CODEC_NDJSON_V1,
        )
        .await?;
        shared::da::persist_trace_index(&run_id, &publication)?;
        Some(publication)
    } else {
        None
    };

    eprintln!("Submitting claim...");
    let claim_result =
        shared::claimer::submit_claim(&provider, contract_address, da_publication.as_ref()).await?;
    eprintln!(
        "Claim submitted: id={}, gas={}",
        claim_result.claim_id, claim_result.gas_used
    );

    // 5. Challenger step — rerun-first audit with conditional trace fetch on mismatch.
    let replay_mode = if cli.scenario == "honest" {
        ReplayMode::Honest
    } else {
        ReplayMode::DishonestSimulation
    };
    let resolution = shared::challenger::resolve_claim_with_replay(
        &provider,
        contract_address,
        claim_result.claim_id,
        replay_mode,
    )
    .await?;

    let outcome_status = if resolution.final_state == "Settled" {
        "settled"
    } else {
        "slashed"
    };
    let mut outcome_metrics = HashMap::new();
    outcome_metrics.insert("Tx hash".to_string(), resolution.tx_hash.clone());
    outcome_metrics.insert("Gas used".to_string(), resolution.gas_used.to_string());
    outcome_metrics.insert("Final state".to_string(), resolution.final_state.clone());
    outcome_metrics.insert("Proof status".to_string(), resolution.proof_status.clone());
    outcome_metrics.insert(
        "Claimer artifact root".to_string(),
        resolution.claimer_artifact_root.clone(),
    );
    outcome_metrics.insert(
        "Claimer result root".to_string(),
        resolution.claimer_result_root.clone(),
    );
    outcome_metrics.insert(
        "Observed artifact root".to_string(),
        resolution.divergence.observed_artifact_root.clone(),
    );
    outcome_metrics.insert(
        "Observed result root".to_string(),
        resolution.divergence.observed_result_root.clone(),
    );

    if let Some(trace_tx_hash) = &resolution.divergence.trace_tx_hash {
        outcome_metrics.insert("Trace tx hash".to_string(), trace_tx_hash.clone());
    }
    if let Some(trace_payload_bytes) = resolution.divergence.trace_payload_bytes {
        outcome_metrics.insert(
            "Trace payload bytes".to_string(),
            trace_payload_bytes.to_string(),
        );
    }
    outcome_metrics.insert(
        "Trace fetch".to_string(),
        resolution.divergence.trace_fetch_status.clone(),
    );
    let outcome_gas = resolution.gas_used;

    eprintln!("Outcome: {outcome_status} (gas={outcome_gas})");

    // 6. Assemble RunOutput
    let (exec_step, trace_step, exec_time_ms, trace_size_bytes, raster_pin) =
        if let Some(result) = &raster_workload_result {
            (
                StepOutput {
                    key: "exec".to_string(),
                    label: "Execute".to_string(),
                    status: "done".to_string(),
                    metrics: raster_workload::exec_step_metrics(result, &cli.workload),
                },
                StepOutput {
                    key: "trace".to_string(),
                    label: "Trace".to_string(),
                    status: "done".to_string(),
                    metrics: raster_workload::trace_step_metrics(result),
                },
                Some(result.exec_time_ms),
                Some(result.trace_size_bytes),
                RasterPin {
                    revision: result.raster_revision.clone(),
                },
            )
        } else {
            (
                StepOutput {
                    key: "exec".to_string(),
                    label: "Execute".to_string(),
                    status: "pending".to_string(),
                    metrics: HashMap::new(),
                },
                StepOutput {
                    key: "trace".to_string(),
                    label: "Trace".to_string(),
                    status: "pending".to_string(),
                    metrics: HashMap::new(),
                },
                None,
                None,
                RasterPin::default(),
            )
        };

    let steps = vec![
        exec_step,
        trace_step,
        if let Some(publication) = &da_publication {
            StepOutput {
                key: "da".to_string(),
                label: "DA Submission".to_string(),
                status: "done".to_string(),
                metrics: HashMap::from([
                    (
                        "Blob tx hash".to_string(),
                        publication.trace_tx_hash.clone(),
                    ),
                    (
                        "Payload bytes".to_string(),
                        publication.payload_bytes.to_string(),
                    ),
                    ("Codec id".to_string(), publication.codec_id.to_string()),
                    ("Gas used".to_string(), publication.gas_used.to_string()),
                    ("Payload hash".to_string(), publication.payload_hash.clone()),
                ]),
            }
        } else {
            StepOutput {
                key: "da".to_string(),
                label: "DA Submission".to_string(),
                status: "pending".to_string(),
                metrics: HashMap::new(),
            }
        },
        // Claim step
        StepOutput {
            key: "claim".to_string(),
            label: "Submit Claim".to_string(),
            status: "done".to_string(),
            metrics: {
                let mut m = HashMap::new();
                m.insert("Claim ID".to_string(), claim_result.claim_id.to_string());
                m.insert("Tx hash".to_string(), claim_result.tx_hash.clone());
                m.insert("Gas used".to_string(), claim_result.gas_used.to_string());
                m.insert(
                    "Artifact root".to_string(),
                    claim_result.artifact_root.clone(),
                );
                m.insert("Result root".to_string(), claim_result.result_root.clone());
                m.insert(
                    "Trace tx hash".to_string(),
                    claim_result.trace_tx_hash.clone(),
                );
                m.insert(
                    "Trace payload bytes".to_string(),
                    claim_result.trace_payload_bytes.to_string(),
                );
                m.insert(
                    "Trace codec id".to_string(),
                    claim_result.trace_codec_id.to_string(),
                );
                m
            },
        },
        // Replay step
        StepOutput {
            key: "replay".to_string(),
            label: "Replay".to_string(),
            status: "done".to_string(),
            metrics: {
                let mut m = HashMap::new();
                m.insert(
                    "Replay time (ms)".to_string(),
                    resolution.replay_time_ms.to_string(),
                );
                m.insert(
                    "Divergence".to_string(),
                    if resolution.divergence.detected {
                        "Detected".to_string()
                    } else {
                        "None".to_string()
                    },
                );
                m.insert("Reason".to_string(), resolution.divergence.reason.clone());
                m.insert(
                    "Trace fetch".to_string(),
                    resolution.divergence.trace_fetch_status.clone(),
                );
                if let Some(index) = resolution.divergence.first_divergence_index {
                    m.insert("First divergence index".to_string(), index.to_string());
                }
                m
            },
        },
        // Outcome step
        StepOutput {
            key: "outcome".to_string(),
            label: "Outcome".to_string(),
            status: outcome_status.to_string(),
            metrics: outcome_metrics,
        },
    ];

    let summary = SummaryOutput {
        exec_time_ms,
        trace_size_bytes,
        da_gas: da_publication
            .as_ref()
            .map(|publication| publication.gas_used),
        claim_gas: claim_result.gas_used as u64,
        replay_time_ms: Some(resolution.replay_time_ms),
        fraud_proof_time_ms: None,
        fraud_proof_gas: None,
        proof_status: resolution.proof_status.clone(),
        divergence: Some(DivergenceSummary {
            detected: resolution.divergence.detected,
            reason: resolution.divergence.reason.clone(),
            first_divergence_index: resolution.divergence.first_divergence_index,
            trace_fetch_status: resolution.divergence.trace_fetch_status.clone(),
            trace_tx_hash: resolution.divergence.trace_tx_hash.clone(),
            trace_payload_bytes: resolution.divergence.trace_payload_bytes,
        }),
        total_time_ms: None,
        outcome: outcome_status.to_string(),
    };

    let run_output = RunOutput {
        id: run_id.clone(),
        workload: cli.workload.clone(),
        scenario: cli.scenario.clone(),
        timestamp: timestamp.to_rfc3339(),
        raster_pin,
        steps,
        summary,
    };

    // 7. Write run file
    let runs_dir = PathBuf::from("runs");
    std::fs::create_dir_all(&runs_dir)?;
    let file_name = format!("{run_id}.json");
    let file_path = runs_dir.join(&file_name);
    let json = serde_json::to_string_pretty(&run_output)?;
    std::fs::write(&file_path, &json)?;

    // Verify serde roundtrip
    let _roundtrip: RunOutput = serde_json::from_str(&json)?;

    // 8. Print summary
    eprintln!("---");
    println!(
        "outcome={} claim_gas={} outcome_gas={} contract={} file={}",
        outcome_status,
        claim_result.gas_used,
        outcome_gas,
        contract_address,
        file_path.display()
    );

    // Also print the full run JSON to stderr for debugging
    eprintln!("Run JSON written to {}", file_path.display());

    Ok(())
}
