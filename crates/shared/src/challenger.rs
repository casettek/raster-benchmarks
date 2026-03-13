use std::time::Instant;

use alloy::primitives::{Address, B256, FixedBytes, U256};
use eyre::{Result, WrapErr};
use serde::Serialize;

use crate::anvil::AnvilProvider;
use crate::claimer::{ARTIFACT_ROOT, RESULT_ROOT, WORKLOAD_ID};
use crate::contract::IClaimVerifier;

/// Hardcoded dishonest observed roots (distinct from claimer's `0xaa`/`0xbb`).
pub const OBSERVED_ARTIFACT_ROOT: [u8; 32] = [0xcc; 32];
pub const OBSERVED_RESULT_ROOT: [u8; 32] = [0xdd; 32];

#[derive(Debug, Clone, Copy)]
pub enum ReplayMode {
    Honest,
    DishonestSimulation,
}

/// Result of a successful settle (honest path).
#[derive(Debug, Clone, Serialize)]
pub struct SettleResult {
    pub claim_id: U256,
    pub tx_hash: String,
    pub gas_used: u64,
    pub block_number: u64,
    pub final_state: String,
}

/// Result of a successful challenge (dishonest path).
#[derive(Debug, Clone, Serialize)]
pub struct ChallengeResult {
    pub claim_id: U256,
    pub tx_hash: String,
    pub gas_used: u64,
    pub block_number: u64,
    pub final_state: String,
    pub claimer_artifact_root: String,
    pub claimer_result_root: String,
    pub observed_artifact_root: String,
    pub observed_result_root: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DivergenceReport {
    pub detected: bool,
    pub reason: String,
    pub first_divergence_index: Option<u64>,
    pub trace_fetch_status: String,
    pub trace_tx_hash: Option<String>,
    pub trace_payload_bytes: Option<u32>,
    pub observed_artifact_root: String,
    pub observed_result_root: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuditResolution {
    pub replay_time_ms: u64,
    pub divergence: DivergenceReport,
    pub proof_status: String,
    pub final_state: String,
    pub tx_hash: String,
    pub gas_used: u64,
    pub claim_id: U256,
    pub claimer_artifact_root: String,
    pub claimer_result_root: String,
}

/// Settle a pending claim (honest path).
///
/// Calls `settleClaim(claimId)` and decodes the `ClaimSettled` event.
pub async fn settle_claim(
    provider: &AnvilProvider,
    contract_address: Address,
    claim_id: U256,
) -> Result<SettleResult> {
    let contract = IClaimVerifier::new(contract_address, provider);

    // Verify claim is pending
    let claim = contract.getClaim(claim_id).call().await?;
    let state: u8 = claim.state.into();
    if state != 1 {
        return Err(eyre::eyre!(
            "Expected claim state Pending (1), got {}",
            state
        ));
    }

    let pending = contract.settleClaim(claim_id).send().await?;
    let receipt = pending.get_receipt().await?;

    // Verify ClaimSettled event
    let _settled = receipt
        .inner
        .logs()
        .iter()
        .find_map(|log| {
            log.log_decode::<IClaimVerifier::ClaimSettled>()
                .ok()
                .map(|decoded| decoded.inner)
        })
        .ok_or_else(|| eyre::eyre!("ClaimSettled event not found in receipt"))?;

    Ok(SettleResult {
        claim_id,
        tx_hash: format!("{}", receipt.transaction_hash),
        gas_used: receipt.gas_used,
        block_number: receipt.block_number.unwrap(),
        final_state: "Settled".to_string(),
    })
}

/// Challenge a pending claim with divergent roots.
pub async fn challenge_claim_with_observed(
    provider: &AnvilProvider,
    contract_address: Address,
    claim_id: U256,
    observed_artifact: FixedBytes<32>,
    observed_result: FixedBytes<32>,
) -> Result<ChallengeResult> {
    let contract = IClaimVerifier::new(contract_address, provider);

    // Read the original claim to capture claimer roots
    let claim = contract.getClaim(claim_id).call().await?;
    let claimer_artifact_root = format!("0x{}", alloy::hex::encode(claim.artifactRoot));
    let claimer_result_root = format!("0x{}", alloy::hex::encode(claim.resultRoot));

    let pending = contract
        .challengeClaim(claim_id, observed_artifact, observed_result)
        .send()
        .await?;
    let receipt = pending.get_receipt().await?;

    // Verify ClaimChallenged event
    let _challenged = receipt
        .inner
        .logs()
        .iter()
        .find_map(|log| {
            log.log_decode::<IClaimVerifier::ClaimChallenged>()
                .ok()
                .map(|decoded| decoded.inner)
        })
        .ok_or_else(|| eyre::eyre!("ClaimChallenged event not found in receipt"))?;

    // Verify ClaimSlashed event
    let _slashed = receipt
        .inner
        .logs()
        .iter()
        .find_map(|log| {
            log.log_decode::<IClaimVerifier::ClaimSlashed>()
                .ok()
                .map(|decoded| decoded.inner)
        })
        .ok_or_else(|| eyre::eyre!("ClaimSlashed event not found in receipt"))?;

    Ok(ChallengeResult {
        claim_id,
        tx_hash: format!("{}", receipt.transaction_hash),
        gas_used: receipt.gas_used,
        block_number: receipt.block_number.unwrap(),
        final_state: "Slashed".to_string(),
        claimer_artifact_root,
        claimer_result_root,
        observed_artifact_root: format!("0x{}", alloy::hex::encode(observed_artifact)),
        observed_result_root: format!("0x{}", alloy::hex::encode(observed_result)),
    })
}

/// Challenge with the legacy hardcoded dishonest roots.
pub async fn challenge_claim(
    provider: &AnvilProvider,
    contract_address: Address,
    claim_id: U256,
) -> Result<ChallengeResult> {
    let observed_artifact = FixedBytes::from(OBSERVED_ARTIFACT_ROOT);
    let observed_result = FixedBytes::from(OBSERVED_RESULT_ROOT);
    challenge_claim_with_observed(
        provider,
        contract_address,
        claim_id,
        observed_artifact,
        observed_result,
    )
    .await
}

pub async fn resolve_claim_with_replay(
    provider: &AnvilProvider,
    contract_address: Address,
    claim_id: U256,
    mode: ReplayMode,
) -> Result<AuditResolution> {
    let contract = IClaimVerifier::new(contract_address, provider);
    let claim = contract
        .getClaim(claim_id)
        .call()
        .await
        .wrap_err("failed to fetch claim before replay")?;

    let replay_start = Instant::now();
    let (observed_artifact, observed_result) = replay_roots_for_mode(mode, claim.workloadId)?;
    let replay_time_ms = replay_start.elapsed().as_millis().min(u64::MAX as u128) as u64;

    let mismatch = claim.artifactRoot != observed_artifact || claim.resultRoot != observed_result;

    let mut divergence = DivergenceReport {
        detected: mismatch,
        reason: if mismatch {
            "Local replay output differs from claimed result".to_string()
        } else {
            "Local replay matched claimed result".to_string()
        },
        first_divergence_index: None,
        trace_fetch_status: "skipped".to_string(),
        trace_tx_hash: None,
        trace_payload_bytes: None,
        observed_artifact_root: format!("0x{}", alloy::hex::encode(observed_artifact)),
        observed_result_root: format!("0x{}", alloy::hex::encode(observed_result)),
    };

    if mismatch {
        let trace_hash = B256::from(claim.traceTxHash);
        if trace_hash == B256::ZERO {
            divergence.trace_fetch_status = "missing-pointer".to_string();
            divergence.reason =
                "Local replay mismatch and claim has no trace pointer for audit".to_string();
        } else {
            let fetched_payload = crate::da::fetch_trace_payload_from_tx(
                provider,
                contract_address,
                trace_hash,
                claim.tracePayloadBytes,
                claim.traceCodecId,
            )
            .await?;

            divergence.trace_fetch_status = "fetched".to_string();
            divergence.trace_tx_hash = Some(format!("0x{}", alloy::hex::encode(trace_hash)));
            divergence.trace_payload_bytes = Some(claim.tracePayloadBytes);
            if !fetched_payload.is_empty() {
                divergence.first_divergence_index = Some(0);
            }
        }
    }

    if mismatch {
        let challenge = challenge_claim_with_observed(
            provider,
            contract_address,
            claim_id,
            observed_artifact,
            observed_result,
        )
        .await?;

        Ok(AuditResolution {
            replay_time_ms,
            divergence,
            proof_status: "not-generated".to_string(),
            final_state: challenge.final_state,
            tx_hash: challenge.tx_hash,
            gas_used: challenge.gas_used,
            claim_id,
            claimer_artifact_root: challenge.claimer_artifact_root,
            claimer_result_root: challenge.claimer_result_root,
        })
    } else {
        let settled = settle_claim(provider, contract_address, claim_id).await?;
        Ok(AuditResolution {
            replay_time_ms,
            divergence,
            proof_status: "not-generated".to_string(),
            final_state: settled.final_state,
            tx_hash: settled.tx_hash,
            gas_used: settled.gas_used,
            claim_id,
            claimer_artifact_root: format!("0x{}", alloy::hex::encode(claim.artifactRoot)),
            claimer_result_root: format!("0x{}", alloy::hex::encode(claim.resultRoot)),
        })
    }
}

fn replay_roots_for_mode(
    mode: ReplayMode,
    workload_id: FixedBytes<32>,
) -> Result<(FixedBytes<32>, FixedBytes<32>)> {
    let expected_workload = FixedBytes::from(WORKLOAD_ID);
    if workload_id != expected_workload {
        return Err(eyre::eyre!(
            "unsupported workload id for replay: 0x{}",
            alloy::hex::encode(workload_id)
        ));
    }

    Ok(match mode {
        ReplayMode::Honest => (
            FixedBytes::from(ARTIFACT_ROOT),
            FixedBytes::from(RESULT_ROOT),
        ),
        ReplayMode::DishonestSimulation => (
            FixedBytes::from(OBSERVED_ARTIFACT_ROOT),
            FixedBytes::from(OBSERVED_RESULT_ROOT),
        ),
    })
}
