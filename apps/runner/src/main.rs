use std::collections::HashMap;
use std::path::PathBuf;

use chrono::Utc;
use clap::Parser;
use eyre::Result;
use shared::challenger::ReplayMode;
use shared::claimer::{L2ClaimInput, default_l2_claim_input};
use shared::deploy::{DEFAULT_CHALLENGE_PERIOD, DEFAULT_MIN_BOND};
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

    /// Workload name (raster-hello or l2-kona-poc)
    #[arg(long, default_value = "l2-kona-poc")]
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
    let is_l2 = cli.workload == "l2-kona-poc";

    // 1. Prepare canonical batch (L2 only — log the fixture identity)
    if is_l2 {
        eprintln!("Preparing canonical batch from synthetic fixture...");
    }

    // 2. Execute Raster workload when requested
    let raster_workload_result = raster_workload::run(&cli.workload, &run_id)?;

    // 3. Start chain
    eprintln!("Spawning Anvil...");
    let (_anvil, provider) = shared::anvil::spawn_anvil()?;

    // 4. Deploy contract
    eprintln!("Deploying ClaimVerifier from {}...", forge_out.display());
    let contract_address = shared::deploy::deploy_claim_verifier(&provider, &forge_out).await?;
    eprintln!("ClaimVerifier deployed at {contract_address}");

    // 5. DA publication — publish trace payload for real workloads
    let da_publication = if let Some(result) = &raster_workload_result {
        let trace_payload = raster_workload::load_trace_commitment_payload(result)?;
        let publication = shared::da::publish_trace(
            &provider,
            contract_address,
            trace_payload,
            shared::da::TRACE_CODEC_COMMITMENT_JSON_V1,
        )
        .await?;
        shared::da::persist_trace_index(&run_id, &publication)?;
        Some(publication)
    } else {
        None
    };

    // Derive L2 claim fields from fixture for l2-kona-poc, else use defaults
    let l2_input = resolve_l2_claim_input(&cli.workload)?;

    // 6. Submit claim
    eprintln!("Submitting claim...");
    let claim_result = shared::claimer::submit_claim(
        &provider,
        contract_address,
        &l2_input,
        da_publication.as_ref(),
        DEFAULT_MIN_BOND,
    )
    .await?;
    eprintln!(
        "Claim submitted: id={}, gas={}, deadline={}",
        claim_result.claim_id, claim_result.gas_used, claim_result.challenge_deadline
    );

    // 7. Audit + Await Finalization (two-phase for L2, combined for others)
    let replay_mode = if cli.scenario == "honest" {
        ReplayMode::Honest
    } else {
        ReplayMode::DishonestSimulation
    };

    let (audit_result, resolution) = if is_l2 {
        // Two-phase: audit first, then finalize
        let audit = shared::challenger::audit_claim(
            &provider,
            contract_address,
            claim_result.claim_id,
            &cli.workload,
            replay_mode,
            &l2_input,
        )
        .await?;

        eprintln!(
            "Audit complete: divergence={}, deadline={}",
            audit.divergence.detected, audit.challenge_deadline
        );

        if !audit.divergence.detected {
            eprintln!(
                "Awaiting finalization (challenge period {}s)...",
                audit.challenge_period
            );
        }

        let resolution = shared::challenger::finalize_claim(
            &provider,
            contract_address,
            claim_result.claim_id,
            &audit,
            &cli.workload,
            &l2_input,
            replay_mode,
        )
        .await?;

        (Some(audit), resolution)
    } else {
        // Combined single-step for non-L2 workloads
        let resolution = shared::challenger::resolve_claim_with_replay(
            &provider,
            contract_address,
            claim_result.claim_id,
            &cli.workload,
            replay_mode,
            &l2_input,
        )
        .await?;
        (None, resolution)
    };

    let outcome_status = if resolution.final_state == "Settled" {
        "settled"
    } else {
        "slashed"
    };
    let outcome_gas = resolution.gas_used;
    eprintln!("Outcome: {outcome_status} (gas={outcome_gas})");

    // 8. Assemble RunOutput
    let (exec_time_ms, trace_size_bytes, trace_commitment_size_bytes, raster_pin) =
        if let Some(result) = &raster_workload_result {
            (
                Some(result.exec_time_ms),
                Some(result.trace_size_bytes),
                Some(result.trace_commitment_size_bytes),
                RasterPin {
                    revision: result.raster_revision.clone(),
                },
            )
        } else {
            (None, None, None, RasterPin::default())
        };

    let steps = build_steps(
        is_l2,
        &cli.workload,
        &raster_workload_result,
        &da_publication,
        &claim_result,
        &audit_result,
        &resolution,
        outcome_status,
    );

    let summary = build_summary(
        exec_time_ms,
        trace_size_bytes,
        trace_commitment_size_bytes,
        &da_publication,
        &claim_result,
        &resolution,
        outcome_status,
        is_l2,
        None, // no total_time_ms for CLI runner
    );

    let run_output = RunOutput {
        id: run_id.clone(),
        workload: cli.workload.clone(),
        scenario: cli.scenario.clone(),
        timestamp: timestamp.to_rfc3339(),
        raster_pin,
        steps,
        summary,
    };

    // 9. Write run file
    let runs_dir = PathBuf::from("runs");
    std::fs::create_dir_all(&runs_dir)?;
    let file_name = format!("{run_id}.json");
    let file_path = runs_dir.join(&file_name);
    let json = serde_json::to_string_pretty(&run_output)?;
    std::fs::write(&file_path, &json)?;

    // Verify serde roundtrip
    let _roundtrip: RunOutput = serde_json::from_str(&json)?;

    // 10. Print summary
    eprintln!("---");
    println!(
        "outcome={} claim_gas={} outcome_gas={} contract={} file={}",
        outcome_status,
        claim_result.gas_used,
        outcome_gas,
        contract_address,
        file_path.display()
    );

    eprintln!("Run JSON written to {}", file_path.display());

    Ok(())
}

/// Build the ordered steps vector for a run.
///
/// For L2 workloads (`l2-kona-poc`), uses the expanded lifecycle:
///   prepare → exec → da → claim → audit → await-finalization → outcome
///
/// For other workloads, uses the legacy lifecycle:
///   exec → trace → da → claim → replay → outcome
#[allow(clippy::too_many_arguments)]
fn build_steps(
    is_l2: bool,
    workload: &str,
    raster_result: &Option<raster_workload::RasterWorkloadResult>,
    da_publication: &Option<shared::da::TracePublication>,
    claim_result: &shared::claimer::ClaimResult,
    audit_result: &Option<shared::challenger::AuditResult>,
    resolution: &shared::challenger::AuditResolution,
    outcome_status: &str,
) -> Vec<StepOutput> {
    let mut steps = Vec::new();

    // --- Prepare step (L2 only) ---
    if is_l2 {
        let mut metrics = HashMap::new();
        metrics.insert(
            "Fixture".to_string(),
            "l2-poc-synth-fixture.json".to_string(),
        );
        metrics.insert(
            "Batch hash".to_string(),
            claim_result.batch_hash.clone(),
        );
        metrics.insert(
            "Block range".to_string(),
            format!("{} → {}", claim_result.start_block, claim_result.end_block),
        );
        steps.push(StepOutput {
            key: "prepare".to_string(),
            label: "Prepare Batch".to_string(),
            status: "done".to_string(),
            metrics,
        });
    }

    // --- Exec step ---
    if let Some(result) = raster_result {
        steps.push(StepOutput {
            key: "exec".to_string(),
            label: if is_l2 {
                "Execute Program".to_string()
            } else {
                "Execute".to_string()
            },
            status: "done".to_string(),
            metrics: raster_workload::exec_step_metrics(result, workload),
        });
    } else {
        steps.push(StepOutput {
            key: "exec".to_string(),
            label: "Execute".to_string(),
            status: "pending".to_string(),
            metrics: HashMap::new(),
        });
    }

    // --- Trace step (non-L2 only; L2 folds trace into exec) ---
    if !is_l2 {
        if let Some(result) = raster_result {
            steps.push(StepOutput {
                key: "trace".to_string(),
                label: "Trace".to_string(),
                status: "done".to_string(),
                metrics: raster_workload::trace_step_metrics(result),
            });
        } else {
            steps.push(StepOutput {
                key: "trace".to_string(),
                label: "Trace".to_string(),
                status: "pending".to_string(),
                metrics: HashMap::new(),
            });
        }
    }

    // --- DA step ---
    if let Some(publication) = da_publication {
        steps.push(StepOutput {
            key: "da".to_string(),
            label: if is_l2 {
                "Publish to DA".to_string()
            } else {
                "DA Submission".to_string()
            },
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
        });
    } else {
        steps.push(StepOutput {
            key: "da".to_string(),
            label: if is_l2 {
                "Publish to DA".to_string()
            } else {
                "DA Submission".to_string()
            },
            status: "pending".to_string(),
            metrics: HashMap::new(),
        });
    }

    // --- Claim step ---
    steps.push(StepOutput {
        key: "claim".to_string(),
        label: "Submit Claim".to_string(),
        status: "done".to_string(),
        metrics: build_claim_metrics(claim_result),
    });

    // --- Audit / Replay step ---
    if is_l2 {
        // L2: explicit audit step
        let audit = audit_result
            .as_ref()
            .expect("audit_result required for L2 workload");
        let mut m = HashMap::new();
        m.insert(
            "Replay time (ms)".to_string(),
            audit.replay_time_ms.to_string(),
        );
        m.insert(
            "Divergence".to_string(),
            if audit.divergence.detected {
                "Detected".to_string()
            } else {
                "None".to_string()
            },
        );
        m.insert("Reason".to_string(), audit.divergence.reason.clone());
        m.insert(
            "Trace fetch".to_string(),
            audit.divergence.trace_fetch_status.clone(),
        );
        if let Some(index) = audit.divergence.first_divergence_index {
            m.insert("First divergence index".to_string(), index.to_string());
        }
        steps.push(StepOutput {
            key: "audit".to_string(),
            label: "Audit".to_string(),
            status: "done".to_string(),
            metrics: m,
        });

        // L2: await-finalization step
        let mut await_metrics = HashMap::new();
        await_metrics.insert(
            "Challenge deadline".to_string(),
            audit.challenge_deadline.to_string(),
        );
        await_metrics.insert(
            "Challenge period (s)".to_string(),
            audit.challenge_period.to_string(),
        );
        await_metrics.insert(
            "Status".to_string(),
            if audit.divergence.detected {
                "Challenged before deadline".to_string()
            } else {
                "Deadline passed — settling".to_string()
            },
        );
        steps.push(StepOutput {
            key: "await-finalization".to_string(),
            label: "Await Finalization".to_string(),
            status: "done".to_string(),
            metrics: await_metrics,
        });
    } else {
        // Non-L2: legacy replay step
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
        steps.push(StepOutput {
            key: "replay".to_string(),
            label: "Replay".to_string(),
            status: "done".to_string(),
            metrics: m,
        });
    }

    // --- Outcome step ---
    let mut outcome_metrics = HashMap::new();
    outcome_metrics.insert("Tx hash".to_string(), resolution.tx_hash.clone());
    outcome_metrics.insert("Gas used".to_string(), resolution.gas_used.to_string());
    outcome_metrics.insert("Final state".to_string(), resolution.final_state.clone());
    outcome_metrics.insert("Proof status".to_string(), resolution.proof_status.clone());
    outcome_metrics.insert(
        "Claimer nextOutputRoot".to_string(),
        resolution.claimer_next_output_root.clone(),
    );
    outcome_metrics.insert(
        "Observed nextOutputRoot".to_string(),
        resolution.divergence.observed_next_output_root.clone(),
    );
    outcome_metrics.insert(
        "Challenge deadline".to_string(),
        resolution.challenge_deadline.to_string(),
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

    steps.push(StepOutput {
        key: "outcome".to_string(),
        label: "Outcome".to_string(),
        status: outcome_status.to_string(),
        metrics: outcome_metrics,
    });

    steps
}

/// Build claim step metrics from a `ClaimResult`.
fn build_claim_metrics(claim_result: &shared::claimer::ClaimResult) -> HashMap<String, String> {
    let mut m = HashMap::new();
    m.insert("Claim ID".to_string(), claim_result.claim_id.to_string());
    m.insert("Tx hash".to_string(), claim_result.tx_hash.clone());
    m.insert("Gas used".to_string(), claim_result.gas_used.to_string());
    m.insert(
        "prevOutputRoot".to_string(),
        claim_result.prev_output_root.clone(),
    );
    m.insert(
        "nextOutputRoot".to_string(),
        claim_result.next_output_root.clone(),
    );
    m.insert(
        "startBlock".to_string(),
        claim_result.start_block.to_string(),
    );
    m.insert("endBlock".to_string(), claim_result.end_block.to_string());
    m.insert("batchHash".to_string(), claim_result.batch_hash.clone());
    m.insert(
        "Bond amount".to_string(),
        claim_result.bond_amount.clone(),
    );
    m.insert(
        "Challenge deadline".to_string(),
        claim_result.challenge_deadline.to_string(),
    );
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
}

/// Build the aggregate summary for a run.
#[allow(clippy::too_many_arguments)]
fn build_summary(
    exec_time_ms: Option<u64>,
    trace_size_bytes: Option<u64>,
    trace_commitment_size_bytes: Option<u64>,
    da_publication: &Option<shared::da::TracePublication>,
    claim_result: &shared::claimer::ClaimResult,
    resolution: &shared::challenger::AuditResolution,
    outcome_status: &str,
    is_l2: bool,
    total_time_ms: Option<u64>,
) -> SummaryOutput {
    SummaryOutput {
        exec_time_ms,
        trace_size_bytes,
        trace_commitment_size_bytes,
        da_gas: da_publication
            .as_ref()
            .map(|publication| publication.gas_used),
        claim_gas: claim_result.gas_used,
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
        total_time_ms,
        outcome: outcome_status.to_string(),

        // L2 claim metadata
        prev_output_root: if is_l2 {
            Some(claim_result.prev_output_root.clone())
        } else {
            None
        },
        next_output_root: if is_l2 {
            Some(claim_result.next_output_root.clone())
        } else {
            None
        },
        start_block: if is_l2 {
            Some(claim_result.start_block)
        } else {
            None
        },
        end_block: if is_l2 {
            Some(claim_result.end_block)
        } else {
            None
        },
        batch_hash: if is_l2 {
            Some(claim_result.batch_hash.clone())
        } else {
            None
        },
        input_blob_versioned_hash: if is_l2 {
            Some(claim_result.input_blob_versioned_hash.clone())
        } else {
            None
        },
        bond_amount: if is_l2 {
            Some(claim_result.bond_amount.clone())
        } else {
            None
        },
        challenge_deadline: if is_l2 {
            Some(claim_result.challenge_deadline)
        } else {
            None
        },
        challenge_period_seconds: if is_l2 {
            Some(DEFAULT_CHALLENGE_PERIOD)
        } else {
            None
        },
    }
}

/// Resolve L2 claim input fields from the canonical fixture for l2-kona-poc,
/// or return default values for other workloads.
fn resolve_l2_claim_input(workload: &str) -> Result<L2ClaimInput> {
    if workload == "l2-kona-poc" {
        let fixture_path = "runs/fixtures/l2-poc-synth-fixture.json";
        let fixture_json = std::fs::read_to_string(fixture_path)
            .map_err(|e| eyre::eyre!("Failed to read fixture {fixture_path}: {e}"))?;
        let fixture: serde_json::Value = serde_json::from_str(&fixture_json)?;

        let prev_output_root = parse_hex_bytes32(
            fixture["pre_checkpoint"]["prev_output_root"]
                .as_str()
                .ok_or_else(|| eyre::eyre!("missing pre_checkpoint.prev_output_root"))?,
        )?;

        // The nextOutputRoot comes from the deterministic execution of the canonical
        // fixture. This is the value that the Kona workload computes. For now we
        // read the deterministic known value from the fixture's post-execution state
        // or use the canonical value from repeated strict runs.
        //
        // Canonical deterministic nextOutputRoot for l2-poc-synth-v1:
        let next_output_root = parse_hex_bytes32(
            "0xe13f82b2b6e02d94a7b1a2a5a8ca21da71c7d14c1e3e35d97687e7bf86425b17",
        )?;

        let start_block = fixture["start_block"]
            .as_u64()
            .ok_or_else(|| eyre::eyre!("missing start_block"))?;
        let end_block = fixture["end_block"]
            .as_u64()
            .ok_or_else(|| eyre::eyre!("missing end_block"))?;
        let batch_hash = parse_hex_bytes32(
            fixture["batch_hash"]
                .as_str()
                .ok_or_else(|| eyre::eyre!("missing batch_hash"))?,
        )?;

        Ok(L2ClaimInput {
            prev_output_root,
            next_output_root,
            start_block,
            end_block,
            batch_hash,
        })
    } else {
        Ok(default_l2_claim_input())
    }
}

fn parse_hex_bytes32(hex_str: &str) -> Result<[u8; 32]> {
    let hex_str = hex_str.strip_prefix("0x").unwrap_or(hex_str);
    let bytes = alloy::hex::decode(hex_str)?;
    if bytes.len() != 32 {
        return Err(eyre::eyre!(
            "expected 32 bytes, got {} from 0x{hex_str}",
            bytes.len()
        ));
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    Ok(arr)
}
