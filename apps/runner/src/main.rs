use std::collections::HashMap;
use std::path::PathBuf;

use chrono::Utc;
use clap::Parser;
use eyre::Result;
use shared::run::{RasterPin, RunOutput, StepOutput, SummaryOutput};

#[derive(Parser)]
#[command(name = "runner", about = "Orchestrate a claimer → challenger scenario run")]
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

    // 1. Start chain
    eprintln!("Spawning Anvil...");
    let (_anvil, provider) = shared::anvil::spawn_anvil()?;

    // 2. Deploy contract
    eprintln!("Deploying ClaimVerifier from {}...", forge_out.display());
    let contract_address =
        shared::deploy::deploy_claim_verifier(&provider, &forge_out).await?;
    eprintln!("ClaimVerifier deployed at {contract_address}");

    // 3. Claimer step — submit claim with stub roots
    eprintln!("Submitting claim...");
    let claim_result = shared::claimer::submit_claim(&provider, contract_address).await?;
    eprintln!(
        "Claim submitted: id={}, gas={}",
        claim_result.claim_id, claim_result.gas_used
    );

    // 4. Challenger step — honest (settle) or dishonest (challenge+slash)
    let (_outcome_tx_hash, outcome_gas, outcome_status, outcome_metrics) = match cli.scenario.as_str()
    {
        "honest" => {
            eprintln!("Settling claim (honest path)...");
            let result = shared::challenger::settle_claim(
                &provider,
                contract_address,
                claim_result.claim_id,
            )
            .await?;
            let mut metrics = HashMap::new();
            metrics.insert("Tx hash".to_string(), result.tx_hash.clone());
            metrics.insert("Gas used".to_string(), result.gas_used.to_string());
            metrics.insert("Final state".to_string(), result.final_state.clone());
            (result.tx_hash, result.gas_used, "settled", metrics)
        }
        "dishonest" => {
            eprintln!("Challenging claim (dishonest path)...");
            let result = shared::challenger::challenge_claim(
                &provider,
                contract_address,
                claim_result.claim_id,
            )
            .await?;
            let mut metrics = HashMap::new();
            metrics.insert("Tx hash".to_string(), result.tx_hash.clone());
            metrics.insert("Gas used".to_string(), result.gas_used.to_string());
            metrics.insert("Final state".to_string(), result.final_state.clone());
            metrics.insert(
                "Claimer artifact root".to_string(),
                result.claimer_artifact_root.clone(),
            );
            metrics.insert(
                "Claimer result root".to_string(),
                result.claimer_result_root.clone(),
            );
            metrics.insert(
                "Observed artifact root".to_string(),
                result.observed_artifact_root.clone(),
            );
            metrics.insert(
                "Observed result root".to_string(),
                result.observed_result_root.clone(),
            );
            (result.tx_hash, result.gas_used, "slashed", metrics)
        }
        _ => unreachable!(),
    };

    eprintln!("Outcome: {outcome_status} (gas={outcome_gas})");

    // 5. Assemble RunOutput
    let steps = vec![
        // Raster-only steps — pending placeholders
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
        StepOutput {
            key: "da".to_string(),
            label: "DA Submission".to_string(),
            status: "pending".to_string(),
            metrics: HashMap::new(),
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
                m.insert("Artifact root".to_string(), claim_result.artifact_root.clone());
                m.insert("Result root".to_string(), claim_result.result_root.clone());
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
                m.insert("Replay time".to_string(), "n/a".to_string());
                m.insert(
                    "Divergence".to_string(),
                    if cli.scenario == "honest" {
                        "None".to_string()
                    } else {
                        "Detected".to_string()
                    },
                );
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
        exec_time_ms: None,
        trace_size_bytes: None,
        da_gas: None,
        claim_gas: claim_result.gas_used as u64,
        replay_time_ms: None,
        fraud_proof_time_ms: None,
        fraud_proof_gas: None,
        total_time_ms: None,
        outcome: outcome_status.to_string(),
    };

    let run_output = RunOutput {
        id: run_id.clone(),
        workload: cli.workload.clone(),
        scenario: cli.scenario.clone(),
        timestamp: timestamp.to_rfc3339(),
        raster_pin: RasterPin::default(),
        steps,
        summary,
    };

    // 6. Write run file
    let runs_dir = PathBuf::from("runs");
    std::fs::create_dir_all(&runs_dir)?;
    let file_name = format!("{run_id}.json");
    let file_path = runs_dir.join(&file_name);
    let json = serde_json::to_string_pretty(&run_output)?;
    std::fs::write(&file_path, &json)?;

    // Verify serde roundtrip
    let _roundtrip: RunOutput = serde_json::from_str(&json)?;

    // 7. Print summary
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
