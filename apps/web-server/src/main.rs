use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Json};
use axum::routing::get;
use axum::Router;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use shared::anvil::AnvilProvider;
use shared::challenger::ReplayMode;
use shared::claimer::{L2ClaimInput, stub_l2_claim_input};
use shared::deploy::DEFAULT_MIN_BOND;
use shared::raster_workload;
use shared::run::{DivergenceSummary, RasterPin, RunOutput, StepOutput, SummaryOutput};
use tokio::sync::Mutex;
use tokio_stream::wrappers::ReceiverStream;
use tower_http::services::ServeDir;

use alloy::node_bindings::AnvilInstance;

/// Shared application state held for the server's lifetime.
struct AppState {
    provider: AnvilProvider,
    runs_dir: PathBuf,
    forge_out_dir: PathBuf,
    /// Mutex to serialize runs (one at a time). Wrapped in Arc so we can
    /// obtain an `OwnedMutexGuard` that can be moved into a spawned task.
    run_lock: Arc<Mutex<()>>,
    /// Held to keep the Anvil process alive. `None` when using external Anvil.
    _anvil: Option<AnvilInstance>,
}

#[derive(Deserialize)]
struct RunParams {
    workload: Option<String>,
    scenario: Option<String>,
}

#[derive(Serialize)]
struct ErrorPayload {
    message: String,
}

/// Serves `web/` as static files and exposes the run API.
///
/// Run from the repo root:
///
///   cargo run -p web-server
///
/// Env vars:
///   PORT       — listen port (default 8010)
///   ANVIL_URL  — connect to external Anvil instead of spawning one
///
/// The `web/` directory is resolved relative to the current working directory,
/// so this must be run from the repo root.
#[tokio::main]
async fn main() {
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8010);

    let web_dir = PathBuf::from("web");
    if !web_dir.exists() {
        eprintln!(
            "error: `web/` directory not found. Run this from the repo root:\n  \
             cargo run -p web-server"
        );
        std::process::exit(1);
    }

    let runs_dir = PathBuf::from("runs");
    std::fs::create_dir_all(&runs_dir).expect("failed to create runs/ directory");

    let forge_out_dir = PathBuf::from("contracts/out");

    eprintln!("Warming Raster workloads (first run optimization)...");
    if let Err(e) = shared::raster_workload::warmup_known_workloads() {
        eprintln!("warning: Raster workload warmup failed: {e}");
    }

    // Anvil lifecycle: spawn or connect
    let (anvil_instance, provider) = match std::env::var("ANVIL_URL") {
        Ok(url) => {
            eprintln!("Connecting to external Anvil at {url}...");
            let provider =
                shared::anvil::connect_provider(&url).expect("failed to connect to external Anvil");
            (None, provider)
        }
        Err(_) => {
            eprintln!("Spawning Anvil...");
            let (instance, provider) = shared::anvil::spawn_anvil().expect("failed to spawn Anvil");
            eprintln!("Anvil running at {}", instance.endpoint());
            (Some(instance), provider)
        }
    };

    // Verify contract artifacts are available (deploy once to validate, then redeploy per run)
    eprintln!("Verifying ClaimVerifier artifacts...");
    let test_address = shared::deploy::deploy_claim_verifier(&provider, &forge_out_dir)
        .await
        .expect("failed to deploy ClaimVerifier (did you run `forge build` in contracts/?)");
    eprintln!("ClaimVerifier verified at {test_address} (will redeploy per run for clean state)");

    let state = Arc::new(AppState {
        provider,
        runs_dir,
        forge_out_dir,
        run_lock: Arc::new(Mutex::new(())),
        _anvil: anvil_instance,
    });

    let api_routes = Router::new()
        .route("/api/run", get(handle_run_sse))
        .route("/api/runs", get(handle_list_runs))
        .route("/api/runs/:id", get(handle_get_run));

    let app = Router::new()
        .merge(api_routes)
        .with_state(state)
        .fallback_service(ServeDir::new(web_dir));

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    eprintln!("serving at http://{addr}");

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

/// `GET /api/run?workload=stub&scenario=honest`
///
/// Returns `text/event-stream` with step-by-step progress.
/// Uses GET + query params so the browser `EventSource` can connect directly.
async fn handle_run_sse(
    State(state): State<Arc<AppState>>,
    Query(params): Query<RunParams>,
) -> Result<Sse<ReceiverStream<Result<Event, Infallible>>>, impl IntoResponse> {
    let workload = params.workload.unwrap_or_else(|| "stub".to_string());
    let scenario = params.scenario.unwrap_or_else(|| "honest".to_string());

    if scenario != "honest" && scenario != "dishonest" {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorPayload {
                message: format!(
                    "Invalid scenario '{}'. Expected 'honest' or 'dishonest'.",
                    scenario
                ),
            }),
        ));
    }

    // Try to acquire run lock (non-blocking check for concurrent runs)
    let lock = match state.run_lock.clone().try_lock_owned() {
        Ok(guard) => guard,
        Err(_) => {
            return Err((
                StatusCode::CONFLICT,
                Json(ErrorPayload {
                    message: "A run is already in progress. Please wait.".to_string(),
                }),
            ));
        }
    };

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(32);

    // Move the lock guard into the spawned task so it's held for the run duration
    let state_clone = Arc::clone(&state);
    tokio::spawn(async move {
        let _lock = lock; // hold until task completes
        run_pipeline(state_clone, tx, workload, scenario).await;
    });

    let stream = ReceiverStream::new(rx);
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

/// Resolve L2 claim input fields for the given workload.
fn resolve_l2_claim_input(workload: &str) -> eyre::Result<L2ClaimInput> {
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
        Ok(stub_l2_claim_input())
    }
}

fn parse_hex_bytes32(hex_str: &str) -> eyre::Result<[u8; 32]> {
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

/// Execute the full claim/challenge pipeline, emitting SSE events along the way.
async fn run_pipeline(
    state: Arc<AppState>,
    tx: tokio::sync::mpsc::Sender<Result<Event, Infallible>>,
    workload: String,
    scenario: String,
) {
    let total_start = Instant::now();
    let timestamp = Utc::now();
    let run_id = format!(
        "{}-{}-{}",
        timestamp.format("%Y-%m-%dT%H-%M-%S"),
        workload,
        scenario
    );

    // Helper to send an SSE event (ignore send errors if client disconnected)
    let send = |tx: &tokio::sync::mpsc::Sender<Result<Event, Infallible>>, event: Event| {
        let tx = tx.clone();
        async move {
            let _ = tx.send(Ok(event)).await;
        }
    };

    // Emit Raster-only steps as pending immediately for early UI feedback.
    for (key, label) in [
        ("exec", "Execute"),
        ("trace", "Trace"),
        ("da", "DA Submission"),
    ] {
        let step_data = serde_json::json!({
            "key": key,
            "label": label,
            "status": "pending",
            "metrics": {}
        });
        send(
            &tx,
            Event::default().event("step").data(step_data.to_string()),
        )
        .await;
    }

    // For phase 3, we redeploy the contract per run for clean state.
    // This ensures no claim ID conflicts across runs.
    let contract_address =
        match shared::deploy::deploy_claim_verifier(&state.provider, &state.forge_out_dir).await {
            Ok(addr) => addr,
            Err(e) => {
                let _ = tx
                    .send(Ok(Event::default().event("error").data(
                        serde_json::to_string(&ErrorPayload {
                            message: format!("Failed to deploy contract: {e}"),
                        })
                        .unwrap_or_default(),
                    )))
                    .await;
                return;
            }
        };

    if workload != "stub" {
        let exec_running = serde_json::json!({
            "key": "exec",
            "label": "Execute",
            "status": "running",
            "metrics": {}
        });
        send(
            &tx,
            Event::default().event("step").data(exec_running.to_string()),
        )
        .await;
    }

    let raster_workload_result = match raster_workload::run(&workload, &run_id) {
        Ok(result) => result,
        Err(e) => {
            let _ = tx
                .send(Ok(Event::default().event("error").data(
                    serde_json::to_string(&ErrorPayload {
                        message: format!("Raster workload failed: {e}"),
                    })
                    .unwrap_or_default(),
                )))
                .await;
            return;
        }
    };

    if let Some(result) = &raster_workload_result {
        let exec_done = serde_json::json!({
            "key": "exec",
            "label": "Execute",
            "status": "done",
            "metrics": raster_workload::exec_step_metrics(result, &workload)
        });
        send(
            &tx,
            Event::default().event("step").data(exec_done.to_string()),
        )
        .await;

        let trace_running = serde_json::json!({
            "key": "trace",
            "label": "Trace",
            "status": "running",
            "metrics": {}
        });
        send(
            &tx,
            Event::default().event("step").data(trace_running.to_string()),
        )
        .await;

        let trace_done = serde_json::json!({
            "key": "trace",
            "label": "Trace",
            "status": "done",
            "metrics": raster_workload::trace_step_metrics(result)
        });
        send(
            &tx,
            Event::default().event("step").data(trace_done.to_string()),
        )
        .await;
    }

    let da_publication = if let Some(result) = &raster_workload_result {
        let da_running = serde_json::json!({
            "key": "da",
            "label": "DA Submission",
            "status": "running",
            "metrics": {}
        });
        send(
            &tx,
            Event::default().event("step").data(da_running.to_string()),
        )
        .await;

        let trace_payload = match raster_workload::load_trace_payload(result) {
            Ok(payload) => payload,
            Err(e) => {
                let _ = tx
                    .send(Ok(Event::default().event("error").data(
                        serde_json::to_string(&ErrorPayload {
                            message: format!("Failed to load trace payload: {e}"),
                        })
                        .unwrap_or_default(),
                    )))
                    .await;
                return;
            }
        };

        let publication = match shared::da::publish_trace(
            &state.provider,
            contract_address,
            trace_payload,
            shared::da::TRACE_CODEC_NDJSON_V1,
        )
        .await
        {
            Ok(publication) => publication,
            Err(e) => {
                let _ = tx
                    .send(Ok(Event::default().event("error").data(
                        serde_json::to_string(&ErrorPayload {
                            message: format!("Trace publication failed: {e}"),
                        })
                        .unwrap_or_default(),
                    )))
                    .await;
                return;
            }
        };

        if let Err(e) = shared::da::persist_trace_index(&run_id, &publication) {
            let _ = tx
                .send(Ok(Event::default().event("error").data(
                    serde_json::to_string(&ErrorPayload {
                        message: format!("Failed to persist trace index: {e}"),
                    })
                    .unwrap_or_default(),
                )))
                .await;
            return;
        }

        let da_done = serde_json::json!({
            "key": "da",
            "label": "DA Submission",
            "status": "done",
            "metrics": {
                "Blob tx hash": publication.trace_tx_hash.clone(),
                "Payload bytes": publication.payload_bytes.to_string(),
                "Codec id": publication.codec_id.to_string(),
                "Gas used": publication.gas_used.to_string(),
                "Payload hash": publication.payload_hash.clone()
            }
        });
        send(
            &tx,
            Event::default().event("step").data(da_done.to_string()),
        )
        .await;

        Some(publication)
    } else {
        None
    };

    // Resolve L2 claim input for the workload
    let l2_input = match resolve_l2_claim_input(&workload) {
        Ok(input) => input,
        Err(e) => {
            let _ = tx
                .send(Ok(Event::default().event("error").data(
                    serde_json::to_string(&ErrorPayload {
                        message: format!("Failed to resolve L2 claim input: {e}"),
                    })
                    .unwrap_or_default(),
                )))
                .await;
            return;
        }
    };

    // Emit claim step as "running"
    let claim_running = serde_json::json!({
        "key": "claim",
        "label": "Submit Claim",
        "status": "running",
        "metrics": {}
    });
    send(
        &tx,
        Event::default()
            .event("step")
            .data(claim_running.to_string()),
    )
    .await;

    // Submit claim
    let claim_result = match shared::claimer::submit_claim(
        &state.provider,
        contract_address,
        &l2_input,
        da_publication.as_ref(),
        DEFAULT_MIN_BOND,
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            let _ = tx
                .send(Ok(Event::default().event("error").data(
                    serde_json::to_string(&ErrorPayload {
                        message: format!("Claim submission failed: {e}"),
                    })
                    .unwrap_or_default(),
                )))
                .await;
            return;
        }
    };

    // Emit claim step as "done" with metrics
    let mut claim_metrics = HashMap::new();
    claim_metrics.insert("Claim ID".to_string(), claim_result.claim_id.to_string());
    claim_metrics.insert("Tx hash".to_string(), claim_result.tx_hash.clone());
    claim_metrics.insert("Gas used".to_string(), claim_result.gas_used.to_string());
    claim_metrics.insert(
        "prevOutputRoot".to_string(),
        claim_result.prev_output_root.clone(),
    );
    claim_metrics.insert(
        "nextOutputRoot".to_string(),
        claim_result.next_output_root.clone(),
    );
    claim_metrics.insert(
        "startBlock".to_string(),
        claim_result.start_block.to_string(),
    );
    claim_metrics.insert("endBlock".to_string(), claim_result.end_block.to_string());
    claim_metrics.insert("batchHash".to_string(), claim_result.batch_hash.clone());
    claim_metrics.insert(
        "Bond amount".to_string(),
        claim_result.bond_amount.clone(),
    );
    claim_metrics.insert(
        "Challenge deadline".to_string(),
        claim_result.challenge_deadline.to_string(),
    );
    claim_metrics.insert(
        "Trace tx hash".to_string(),
        claim_result.trace_tx_hash.clone(),
    );
    claim_metrics.insert(
        "Trace payload bytes".to_string(),
        claim_result.trace_payload_bytes.to_string(),
    );
    claim_metrics.insert(
        "Trace codec id".to_string(),
        claim_result.trace_codec_id.to_string(),
    );

    let claim_done = serde_json::json!({
        "key": "claim",
        "label": "Submit Claim",
        "status": "done",
        "metrics": claim_metrics
    });
    send(
        &tx,
        Event::default().event("step").data(claim_done.to_string()),
    )
    .await;

    // Challenger step
    let replay_running = serde_json::json!({
        "key": "replay",
        "label": "Replay",
        "status": "running",
        "metrics": {}
    });
    send(
        &tx,
        Event::default()
            .event("step")
            .data(replay_running.to_string()),
    )
    .await;

    let replay_mode = if scenario == "honest" {
        ReplayMode::Honest
    } else {
        ReplayMode::DishonestSimulation
    };
    let resolution = match shared::challenger::resolve_claim_with_replay(
        &state.provider,
        contract_address,
        claim_result.claim_id,
        replay_mode,
        &l2_input,
    )
    .await
    {
        Ok(result) => result,
        Err(e) => {
            let _ = tx
                .send(Ok(Event::default().event("error").data(
                    serde_json::to_string(&ErrorPayload {
                        message: format!("Replay/challenge resolution failed: {e}"),
                    })
                    .unwrap_or_default(),
                )))
                .await;
            return;
        }
    };

    let mut replay_metrics = HashMap::new();
    replay_metrics.insert(
        "Replay time (ms)".to_string(),
        resolution.replay_time_ms.to_string(),
    );
    replay_metrics.insert(
        "Divergence".to_string(),
        if resolution.divergence.detected {
            "Detected".to_string()
        } else {
            "None".to_string()
        },
    );
    replay_metrics.insert("Reason".to_string(), resolution.divergence.reason.clone());
    replay_metrics.insert(
        "Trace fetch".to_string(),
        resolution.divergence.trace_fetch_status.clone(),
    );
    if let Some(index) = resolution.divergence.first_divergence_index {
        replay_metrics.insert("First divergence index".to_string(), index.to_string());
    }

    let replay_done = serde_json::json!({
        "key": "replay",
        "label": "Replay",
        "status": "done",
        "metrics": replay_metrics
    });
    send(
        &tx,
        Event::default().event("step").data(replay_done.to_string()),
    )
    .await;

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
    outcome_metrics.insert(
        "Trace fetch".to_string(),
        resolution.divergence.trace_fetch_status.clone(),
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

    // Emit outcome step
    let outcome_step = serde_json::json!({
        "key": "outcome",
        "label": "Outcome",
        "status": outcome_status,
        "metrics": outcome_metrics
    });
    send(
        &tx,
        Event::default()
            .event("step")
            .data(outcome_step.to_string()),
    )
    .await;

    // Assemble RunOutput
    let steps = vec![
        if let Some(result) = &raster_workload_result {
            StepOutput {
                key: "exec".to_string(),
                label: "Execute".to_string(),
                status: "done".to_string(),
                metrics: raster_workload::exec_step_metrics(result, &workload),
            }
        } else {
            StepOutput {
                key: "exec".to_string(),
                label: "Execute".to_string(),
                status: "pending".to_string(),
                metrics: HashMap::new(),
            }
        },
        if let Some(result) = &raster_workload_result {
            StepOutput {
                key: "trace".to_string(),
                label: "Trace".to_string(),
                status: "done".to_string(),
                metrics: raster_workload::trace_step_metrics(result),
            }
        } else {
            StepOutput {
                key: "trace".to_string(),
                label: "Trace".to_string(),
                status: "pending".to_string(),
                metrics: HashMap::new(),
            }
        },
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
        StepOutput {
            key: "claim".to_string(),
            label: "Submit Claim".to_string(),
            status: "done".to_string(),
            metrics: claim_metrics,
        },
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
                        "Detected"
                    } else {
                        "None"
                    }
                    .to_string(),
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
        StepOutput {
            key: "outcome".to_string(),
            label: "Outcome".to_string(),
            status: outcome_status.to_string(),
            metrics: outcome_metrics.clone(),
        },
    ];

    let summary = SummaryOutput {
        exec_time_ms: raster_workload_result.as_ref().map(|r| r.exec_time_ms),
        trace_size_bytes: raster_workload_result.as_ref().map(|r| r.trace_size_bytes),
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
        total_time_ms: Some(total_start.elapsed().as_millis().min(u64::MAX as u128) as u64),
        outcome: outcome_status.to_string(),
    };

    let run_output = RunOutput {
        id: run_id.clone(),
        workload,
        scenario,
        timestamp: timestamp.to_rfc3339(),
        raster_pin: if let Some(result) = &raster_workload_result {
            RasterPin {
                revision: result.raster_revision.clone(),
            }
        } else {
            RasterPin::default()
        },
        steps,
        summary,
    };

    // Write run file
    let file_name = format!("{run_id}.json");
    let file_path = state.runs_dir.join(&file_name);
    let json = serde_json::to_string_pretty(&run_output).unwrap_or_default();
    if let Err(e) = std::fs::write(&file_path, &json) {
        eprintln!("Failed to write run file: {e}");
    }

    // Emit done event
    let done_data = serde_json::json!({
        "run_id": run_id,
        "file": format!("runs/{file_name}"),
        "run": run_output
    });
    let _ = tx
        .send(Ok(Event::default()
            .event("done")
            .data(done_data.to_string())))
        .await;
}

/// `GET /api/runs` — list all past runs, newest first.
async fn handle_list_runs(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<RunOutput>>, (StatusCode, String)> {
    let mut runs = Vec::new();

    let entries = match std::fs::read_dir(&state.runs_dir) {
        Ok(e) => e,
        Err(_) => return Ok(Json(runs)),
    };

    let mut files: Vec<_> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
        .collect();

    // Sort by filename descending (newest first, since filenames start with timestamps)
    files.sort_by_key(|e| std::cmp::Reverse(e.file_name()));

    for entry in files {
        let path = entry.path();
        match std::fs::read_to_string(&path) {
            Ok(content) => match serde_json::from_str::<RunOutput>(&content) {
                Ok(run) => runs.push(run),
                Err(e) => eprintln!("Skipping invalid run file {}: {e}", path.display()),
            },
            Err(e) => eprintln!("Failed to read {}: {e}", path.display()),
        }
    }

    Ok(Json(runs))
}

/// `GET /api/runs/:id` — get a single run by ID.
async fn handle_get_run(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<RunOutput>, StatusCode> {
    let file_path = state.runs_dir.join(format!("{id}.json"));
    let content = std::fs::read_to_string(&file_path).map_err(|_| StatusCode::NOT_FOUND)?;
    let run: RunOutput = serde_json::from_str(&content).map_err(|_| StatusCode::NOT_FOUND)?;
    Ok(Json(run))
}
