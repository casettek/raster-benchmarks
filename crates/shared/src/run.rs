use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Raster dependency pin — tracks the exact revision used for the workload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RasterPin {
    pub revision: String,
}

impl Default for RasterPin {
    fn default() -> Self {
        Self {
            revision: "unknown".to_string(),
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
/// applicable (e.g., workloads where a particular metric is not relevant).
///
/// L2 claim metadata fields are `Option` and only populated for L2 workloads
/// (`l2-kona-poc`). They capture enough information to explain what was
/// submitted onchain and when it can finalize.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummaryOutput {
    pub exec_time_ms: Option<u64>,
    pub trace_size_bytes: Option<u64>,
    pub trace_commitment_size_bytes: Option<u64>,
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

    // --- L2 claim metadata (populated for l2-kona-poc runs) ---
    /// Prior agreed OP output root (hex).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prev_output_root: Option<String>,
    /// Claimed OP output root after execution (hex).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_output_root: Option<String>,
    /// First L2 block in the claimed range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_block: Option<u64>,
    /// Last L2 block in the claimed range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_block: Option<u64>,
    /// keccak256(concat(tracked tx raw bytes)) (hex).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub batch_hash: Option<String>,
    /// Canonical input blob manifest tx hash (hex).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_blob_tx_hash: Option<String>,
    /// Canonical input blob manifest versioned hash (hex).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_blob_versioned_hash: Option<String>,
    /// Trace blob manifest tx hash (hex).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_blob_tx_hash: Option<String>,
    /// Trace blob manifest versioned hash (hex).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_blob_versioned_hash: Option<String>,
    /// Claimer bond amount in wei (decimal string).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bond_amount: Option<String>,
    /// Challenge deadline as unix timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub challenge_deadline: Option<u64>,
    /// Challenge period duration in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub challenge_period_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DivergenceSummary {
    pub detected: bool,
    pub reason: String,
    pub first_divergence_index: Option<u64>,
    pub trace_fetch_status: String,
    pub input_fetch_status: Option<String>,
    pub input_blob_versioned_hash: Option<String>,
    pub trace_blob_versioned_hash: Option<String>,
}
