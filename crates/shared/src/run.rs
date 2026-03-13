use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Stub Raster pin — no real Raster version to reference yet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RasterPin {
    pub revision: String,
}

impl Default for RasterPin {
    fn default() -> Self {
        Self {
            revision: "stub".to_string(),
        }
    }
}

/// Top-level run output record matching the scenario runner UI JSON shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunOutput {
    pub id: String,
    pub workload: String,
    pub scenario: String,
    pub timestamp: String,
    pub raster_pin: RasterPin,
    pub steps: Vec<StepOutput>,
    pub summary: SummaryOutput,
}

/// Per-step record with key, label, status, and arbitrary metrics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepOutput {
    pub key: String,
    pub label: String,
    pub status: String,
    pub metrics: HashMap<String, String>,
}

/// Aggregate metrics for a completed run.
///
/// Raster-only fields are `Option` so they serialize as `null` when not
/// applicable (e.g., stub workloads with no real execution).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummaryOutput {
    pub exec_time_ms: Option<u64>,
    pub trace_size_bytes: Option<u64>,
    pub da_gas: Option<u64>,
    pub claim_gas: u64,
    pub replay_time_ms: Option<u64>,
    pub fraud_proof_time_ms: Option<u64>,
    pub fraud_proof_gas: Option<u64>,
    #[serde(default)]
    pub proof_status: String,
    #[serde(default)]
    pub divergence: Option<DivergenceSummary>,
    pub total_time_ms: Option<u64>,
    pub outcome: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DivergenceSummary {
    pub detected: bool,
    pub reason: String,
    pub first_divergence_index: Option<u64>,
    pub trace_fetch_status: String,
    pub trace_tx_hash: Option<String>,
    pub trace_payload_bytes: Option<u32>,
}
