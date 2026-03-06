use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Top-level run output record matching the scenario runner UI JSON shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunOutput {
    pub run_id: String,
    pub scenario: String,
    pub timestamp: String,
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummaryOutput {
    pub exec_time_ms: u64,
    pub trace_size_bytes: u64,
    pub da_gas: u64,
    pub claim_gas: u64,
    pub replay_time_ms: u64,
    pub fraud_proof_time_ms: u64,
    pub fraud_proof_gas: u64,
    pub total_time_ms: u64,
    pub outcome: String,
}
