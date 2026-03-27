use std::time::Instant;

use alloy::primitives::{Address, B256, FixedBytes, U256};
use alloy::providers::Provider;
use eyre::{Result, WrapErr, eyre};
use serde::Serialize;

use crate::anvil::AnvilProvider;
use crate::claimer::L2ClaimInput;
use crate::contract::IClaimVerifier;

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
    pub claimer_next_output_root: String,
    pub observed_next_output_root: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DivergenceReport {
    pub detected: bool,
    pub reason: String,
    pub first_divergence_index: Option<u64>,
    pub trace_fetch_status: String,
    pub input_fetch_status: Option<String>,
    pub input_blob_versioned_hash: Option<String>,
    pub trace_blob_versioned_hash: Option<String>,
    pub observed_next_output_root: String,
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
    pub claimer_next_output_root: String,
    pub challenge_deadline: u64,
}

/// Settle a pending claim after the challenge deadline has passed (honest path).
///
/// On local Anvil, the caller must advance the chain timestamp past the
/// challenge deadline before calling this.
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

/// Challenge a pending claim with a divergent nextOutputRoot.
pub async fn challenge_claim_with_observed(
    provider: &AnvilProvider,
    contract_address: Address,
    claim_id: U256,
    observed_next_output_root: FixedBytes<32>,
) -> Result<ChallengeResult> {
    let contract = IClaimVerifier::new(contract_address, provider);

    let claim = contract.getClaim(claim_id).call().await?;
    let claimer_next_output_root = format!("0x{}", alloy::hex::encode(claim.nextOutputRoot));

    let pending = contract
        .challengeClaim(claim_id, observed_next_output_root)
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
        claimer_next_output_root,
        observed_next_output_root: format!("0x{}", alloy::hex::encode(observed_next_output_root)),
    })
}

/// Advance the Anvil chain timestamp to just past the given deadline.
///
/// Uses Anvil-specific RPC methods `evm_setNextBlockTimestamp` and `evm_mine`.
pub async fn advance_past_deadline(provider: &AnvilProvider, deadline: u64) -> Result<()> {
    // Set the next block timestamp to deadline + 1
    let target_timestamp = deadline + 1;
    provider
        .raw_request::<_, serde_json::Value>(
            "evm_setNextBlockTimestamp".into(),
            [serde_json::Value::String(format!(
                "0x{:x}",
                target_timestamp
            ))],
        )
        .await
        .wrap_err("failed to set next block timestamp")?;

    // Mine a block to apply the timestamp
    provider
        .raw_request::<_, serde_json::Value>("evm_mine".into(), ())
        .await
        .wrap_err("failed to mine block")?;

    Ok(())
}

/// Result of the audit phase (replay + conditional trace fetch) before settlement.
///
/// Separating audit from settlement allows callers (runner, web-server) to emit
/// distinct `audit` and `await-finalization` step updates with intermediate state.
#[derive(Debug, Clone, Serialize)]
pub struct AuditResult {
    pub replay_time_ms: u64,
    pub divergence: DivergenceReport,
    pub claimer_next_output_root: String,
    pub challenge_deadline: u64,
    pub challenge_period: u64,
}

/// Perform the audit phase: local replay comparison and conditional trace fetch.
///
/// Returns an `AuditResult` that the caller uses to decide whether to settle
/// or challenge. Does not mutate chain state.
pub async fn audit_claim(
    provider: &AnvilProvider,
    contract_address: Address,
    claim_id: U256,
    workload: &str,
    mode: ReplayMode,
    l2_input: &L2ClaimInput,
) -> Result<AuditResult> {
    let contract = IClaimVerifier::new(contract_address, provider);
    let claim = contract
        .getClaim(claim_id)
        .call()
        .await
        .wrap_err("failed to fetch claim before replay")?;

    let challenge_period = contract
        .challengePeriod()
        .call()
        .await
        .wrap_err("failed to read challengePeriod")?;

    let replay_start = Instant::now();
    let observed_next_output_root = replay_next_output_root(mode, l2_input)?;
    let replay_time_ms = replay_start.elapsed().as_millis().min(u64::MAX as u128) as u64;

    let output_root_mismatch = claim.nextOutputRoot != observed_next_output_root;

    let input_blob_versioned_hash = B256::from(claim.inputBlobVersionedHash);
    let trace_blob_versioned_hash = B256::from(claim.traceBlobVersionedHash);
    if trace_blob_versioned_hash == B256::ZERO {
        return Err(eyre!(
            "claim {} is invalid: missing trace pointer",
            claim_id
        ));
    }

    let mut divergence = DivergenceReport {
        detected: output_root_mismatch,
        reason: if output_root_mismatch {
            "Local replay nextOutputRoot differs from claimed nextOutputRoot".to_string()
        } else {
            "Local replay matched claimed nextOutputRoot".to_string()
        },
        first_divergence_index: None,
        trace_fetch_status: "skipped".to_string(),
        input_fetch_status: None,
        input_blob_versioned_hash: if input_blob_versioned_hash == B256::ZERO {
            None
        } else {
            Some(format!("0x{}", alloy::hex::encode(input_blob_versioned_hash)))
        },
        trace_blob_versioned_hash: Some(format!(
            "0x{}",
            alloy::hex::encode(trace_blob_versioned_hash)
        )),
        observed_next_output_root: format!("0x{}", alloy::hex::encode(observed_next_output_root)),
    };

    let mut input_json_override = None;
    if workload == "l2-kona-poc" && input_blob_versioned_hash != B256::ZERO {
        let (_manifest, package_bytes) =
            crate::da::fetch_blob_artifact(provider, input_blob_versioned_hash).await?;
        input_json_override = Some(
            String::from_utf8(package_bytes)
                .map_err(|_| eyre!("fetched input package bytes were not valid UTF-8 json"))?,
        );
        divergence.input_fetch_status = Some("fetched".to_string());
    } else if workload == "l2-kona-poc" {
        divergence.input_fetch_status = Some("missing".to_string());
    }

    let local_trace_commitment = crate::raster_workload::rerun_trace_commitment_with_input_root(
        workload,
        &claim_id.to_string(),
        input_json_override,
        None,
    )?;
    let (_trace_manifest, fetched_payload) =
        crate::da::fetch_blob_artifact(provider, trace_blob_versioned_hash).await?;
    let published_trace_commitment =
        crate::raster_workload::decode_trace_commitment_payload(&fetched_payload)?;
    let commitment_comparison = crate::raster_workload::compare_trace_commitments(
        &published_trace_commitment,
        &local_trace_commitment,
    );
    let commitment_mismatch = !commitment_comparison.matches;

    divergence.detected = output_root_mismatch || commitment_mismatch;
    divergence.trace_fetch_status = "fetched".to_string();
    divergence.first_divergence_index = commitment_comparison.first_divergence_index;
    divergence.reason = match (output_root_mismatch, commitment_mismatch) {
        (true, true) => format!(
            "Local replay nextOutputRoot differs from claimed nextOutputRoot and {}",
            commitment_comparison.reason.to_lowercase()
        ),
        (true, false) => "Local replay nextOutputRoot differs from claimed nextOutputRoot while trace commitment matches".to_string(),
        (false, true) => commitment_comparison.reason,
        (false, false) => {
            "Local replay matched claimed nextOutputRoot and published trace commitment"
                .to_string()
        }
    };

    Ok(AuditResult {
        replay_time_ms,
        divergence,
        claimer_next_output_root: format!("0x{}", alloy::hex::encode(claim.nextOutputRoot)),
        challenge_deadline: claim.challengeDeadline,
        challenge_period,
    })
}

/// Finalize or challenge a claim based on a prior `AuditResult`.
///
/// If audit found no divergence (honest), advances chain time past deadline
/// and settles. If divergence was detected, challenges with the observed root.
pub async fn finalize_claim(
    provider: &AnvilProvider,
    contract_address: Address,
    claim_id: U256,
    audit: &AuditResult,
    workload: &str,
    l2_input: &L2ClaimInput,
    mode: ReplayMode,
) -> Result<AuditResolution> {
    if audit.divergence.detected {
        if audit.claimer_next_output_root == audit.divergence.observed_next_output_root {
            return Err(eyre!(
                "trace commitment divergence detected for workload '{}' but contract challenge requires a divergent nextOutputRoot",
                workload
            ));
        }
        let observed_next_output_root = replay_next_output_root(mode, l2_input)?;
        let challenge = challenge_claim_with_observed(
            provider,
            contract_address,
            claim_id,
            observed_next_output_root,
        )
        .await?;

        Ok(AuditResolution {
            replay_time_ms: audit.replay_time_ms,
            divergence: audit.divergence.clone(),
            proof_status: "not-generated".to_string(),
            final_state: challenge.final_state,
            tx_hash: challenge.tx_hash,
            gas_used: challenge.gas_used,
            claim_id,
            claimer_next_output_root: challenge.claimer_next_output_root,
            challenge_deadline: audit.challenge_deadline,
        })
    } else {
        // Advance chain time past the challenge deadline before settling
        advance_past_deadline(provider, audit.challenge_deadline).await?;

        let settled = settle_claim(provider, contract_address, claim_id).await?;
        Ok(AuditResolution {
            replay_time_ms: audit.replay_time_ms,
            divergence: audit.divergence.clone(),
            proof_status: "not-generated".to_string(),
            final_state: settled.final_state,
            tx_hash: settled.tx_hash,
            gas_used: settled.gas_used,
            claim_id,
            claimer_next_output_root: audit.claimer_next_output_root.clone(),
            challenge_deadline: audit.challenge_deadline,
        })
    }
}

/// Resolve a claim via local replay and either settle or challenge.
///
/// In honest mode, the expected `nextOutputRoot` matches the claimer's; in
/// dishonest simulation mode, a deliberately wrong root triggers challenge.
///
/// For the honest path, the chain is advanced past the challenge deadline
/// before calling `settleClaim`.
///
/// This is the combined convenience function that calls `audit_claim` +
/// `finalize_claim` in sequence. Callers needing intermediate step events
/// should call the two-phase API instead.
pub async fn resolve_claim_with_replay(
    provider: &AnvilProvider,
    contract_address: Address,
    claim_id: U256,
    workload: &str,
    mode: ReplayMode,
    l2_input: &L2ClaimInput,
) -> Result<AuditResolution> {
    let audit = audit_claim(provider, contract_address, claim_id, workload, mode, l2_input).await?;
    finalize_claim(
        provider,
        contract_address,
        claim_id,
        &audit,
        workload,
        l2_input,
        mode,
    )
    .await
}

/// Produce the expected `nextOutputRoot` for a given replay mode.
///
/// In honest mode, returns the same `nextOutputRoot` from the L2 claim input
/// (matching the claimer). In dishonest simulation, returns a deliberately
/// different value to trigger challenge divergence.
fn replay_next_output_root(
    mode: ReplayMode,
    l2_input: &L2ClaimInput,
) -> Result<FixedBytes<32>> {
    Ok(match mode {
        ReplayMode::Honest => FixedBytes::from(l2_input.next_output_root),
        ReplayMode::DishonestSimulation => {
            // Produce a deterministic but wrong root by flipping a byte
            let mut wrong = l2_input.next_output_root;
            wrong[0] ^= 0xff;
            FixedBytes::from(wrong)
        }
    })
}
