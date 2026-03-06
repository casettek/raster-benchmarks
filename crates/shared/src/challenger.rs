use alloy::primitives::{Address, FixedBytes, U256};
use eyre::Result;
use serde::Serialize;

use crate::anvil::AnvilProvider;
use crate::contract::IClaimVerifier;

/// Hardcoded dishonest observed roots (distinct from claimer's `0xaa`/`0xbb`).
pub const OBSERVED_ARTIFACT_ROOT: [u8; 32] = [0xcc; 32];
pub const OBSERVED_RESULT_ROOT: [u8; 32] = [0xdd; 32];

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

/// Challenge a pending claim with divergent roots (dishonest path).
///
/// Calls `challengeClaim(claimId, observedArtifactRoot, observedResultRoot)`
/// with hardcoded dishonest roots (`0xcc..`, `0xdd..`). Decodes both
/// `ClaimChallenged` and `ClaimSlashed` events.
pub async fn challenge_claim(
    provider: &AnvilProvider,
    contract_address: Address,
    claim_id: U256,
) -> Result<ChallengeResult> {
    let contract = IClaimVerifier::new(contract_address, provider);

    let observed_artifact = FixedBytes::from(OBSERVED_ARTIFACT_ROOT);
    let observed_result = FixedBytes::from(OBSERVED_RESULT_ROOT);

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
