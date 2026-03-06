use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Json};
use axum::routing::get;
use axum::Router;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use shared::anvil::AnvilProvider;
use shared::run::{RasterPin, RunOutput, StepOutput, SummaryOutput};
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

    // Anvil lifecycle: spawn or connect
    let (anvil_instance, provider) = match std::env::var("ANVIL_URL") {
        Ok(url) => {
            eprintln!("Connecting to external Anvil at {url}...");
            let provider = shared::anvil::connect_provider(&url)
                .expect("failed to connect to external Anvil");
            (None, provider)
        }
        Err(_) => {
            eprintln!("Spawning Anvil...");
            let (instance, provider) =
                shared::anvil::spawn_anvil().expect("failed to spawn Anvil");
            eprintln!("Anvil running at {}", instance.endpoint());
            (Some(instance), provider)
        }
    };

    // Verify contract artifacts are available (deploy once to validate, then redeploy per run)
    eprintln!("Verifying ClaimVerifier artifacts...");
    let test_address =
        shared::deploy::deploy_claim_verifier(&provider, &forge_out_dir)
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
                message: format!("Invalid scenario '{}'. Expected 'honest' or 'dishonest'.", scenario),
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

/// Execute the full claim/challenge pipeline, emitting SSE events along the way.
async fn run_pipeline(
    state: Arc<AppState>,
    tx: tokio::sync::mpsc::Sender<Result<Event, Infallible>>,
    workload: String,
    scenario: String,
) {
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

    // For phase 3, we redeploy the contract per run for clean state.
    // This ensures no claim ID conflicts across runs.
    let contract_address = match shared::deploy::deploy_claim_verifier(
        &state.provider,
        &state.forge_out_dir,
    )
    .await
    {
        Ok(addr) => addr,
        Err(e) => {
            let _ = tx
                .send(Ok(Event::default()
                    .event("error")
                    .data(serde_json::to_string(&ErrorPayload {
                        message: format!("Failed to deploy contract: {e}"),
                    })
                    .unwrap_or_default())))
                .await;
            return;
        }
    };

    // Emit Raster-only steps as pending immediately
    for (key, label) in [("exec", "Execute"), ("trace", "Trace"), ("da", "DA Submission")] {
        let step_data = serde_json::json!({
            "key": key,
            "label": label,
            "status": "pending",
            "metrics": {}
        });
        send(
            &tx,
            Event::default()
                .event("step")
                .data(step_data.to_string()),
        )
        .await;
    }

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
    let claim_result = match shared::claimer::submit_claim(&state.provider, contract_address).await
    {
        Ok(r) => r,
        Err(e) => {
            let _ = tx
                .send(Ok(Event::default()
                    .event("error")
                    .data(serde_json::to_string(&ErrorPayload {
                        message: format!("Claim submission failed: {e}"),
                    })
                    .unwrap_or_default())))
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
        "Artifact root".to_string(),
        claim_result.artifact_root.clone(),
    );
    claim_metrics.insert("Result root".to_string(), claim_result.result_root.clone());

    let claim_done = serde_json::json!({
        "key": "claim",
        "label": "Submit Claim",
        "status": "done",
        "metrics": claim_metrics
    });
    send(
        &tx,
        Event::default()
            .event("step")
            .data(claim_done.to_string()),
    )
    .await;

    // Challenger step
    let (outcome_status, _outcome_gas, outcome_metrics) = match scenario.as_str() {
        "honest" => {
            // Emit replay running
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

            match shared::challenger::settle_claim(
                &state.provider,
                contract_address,
                claim_result.claim_id,
            )
            .await
            {
                Ok(result) => {
                    // Emit replay done
                    let mut replay_metrics = HashMap::new();
                    replay_metrics.insert("Replay time".to_string(), "n/a".to_string());
                    replay_metrics.insert("Divergence".to_string(), "None".to_string());
                    let replay_done = serde_json::json!({
                        "key": "replay",
                        "label": "Replay",
                        "status": "done",
                        "metrics": replay_metrics
                    });
                    send(
                        &tx,
                        Event::default()
                            .event("step")
                            .data(replay_done.to_string()),
                    )
                    .await;

                    let mut om = HashMap::new();
                    om.insert("Tx hash".to_string(), result.tx_hash.clone());
                    om.insert("Gas used".to_string(), result.gas_used.to_string());
                    om.insert("Final state".to_string(), result.final_state.clone());
                    ("settled", result.gas_used, om)
                }
                Err(e) => {
                    let _ = tx
                        .send(Ok(Event::default()
                            .event("error")
                            .data(serde_json::to_string(&ErrorPayload {
                                message: format!("Settle failed: {e}"),
                            })
                            .unwrap_or_default())))
                        .await;
                    return;
                }
            }
        }
        "dishonest" => {
            // Emit replay running
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

            match shared::challenger::challenge_claim(
                &state.provider,
                contract_address,
                claim_result.claim_id,
            )
            .await
            {
                Ok(result) => {
                    // Emit replay done
                    let mut replay_metrics = HashMap::new();
                    replay_metrics.insert("Replay time".to_string(), "n/a".to_string());
                    replay_metrics.insert("Divergence".to_string(), "Detected".to_string());
                    let replay_done = serde_json::json!({
                        "key": "replay",
                        "label": "Replay",
                        "status": "done",
                        "metrics": replay_metrics
                    });
                    send(
                        &tx,
                        Event::default()
                            .event("step")
                            .data(replay_done.to_string()),
                    )
                    .await;

                    let mut om = HashMap::new();
                    om.insert("Tx hash".to_string(), result.tx_hash.clone());
                    om.insert("Gas used".to_string(), result.gas_used.to_string());
                    om.insert("Final state".to_string(), result.final_state.clone());
                    om.insert(
                        "Claimer artifact root".to_string(),
                        result.claimer_artifact_root.clone(),
                    );
                    om.insert(
                        "Claimer result root".to_string(),
                        result.claimer_result_root.clone(),
                    );
                    om.insert(
                        "Observed artifact root".to_string(),
                        result.observed_artifact_root.clone(),
                    );
                    om.insert(
                        "Observed result root".to_string(),
                        result.observed_result_root.clone(),
                    );
                    ("slashed", result.gas_used, om)
                }
                Err(e) => {
                    let _ = tx
                        .send(Ok(Event::default()
                            .event("error")
                            .data(serde_json::to_string(&ErrorPayload {
                                message: format!("Challenge failed: {e}"),
                            })
                            .unwrap_or_default())))
                        .await;
                    return;
                }
            }
        }
        _ => unreachable!(),
    };

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
                m.insert("Replay time".to_string(), "n/a".to_string());
                m.insert(
                    "Divergence".to_string(),
                    if scenario == "honest" {
                        "None"
                    } else {
                        "Detected"
                    }
                    .to_string(),
                );
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
        exec_time_ms: None,
        trace_size_bytes: None,
        da_gas: None,
        claim_gas: claim_result.gas_used,
        replay_time_ms: None,
        fraud_proof_time_ms: None,
        fraud_proof_gas: None,
        total_time_ms: None,
        outcome: outcome_status.to_string(),
    };

    let run_output = RunOutput {
        id: run_id.clone(),
        workload,
        scenario,
        timestamp: timestamp.to_rfc3339(),
        raster_pin: RasterPin::default(),
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
        .filter(|e| {
            e.path()
                .extension()
                .is_some_and(|ext| ext == "json")
        })
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
