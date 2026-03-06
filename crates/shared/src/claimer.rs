use alloy::primitives::{Address, FixedBytes, U256};
use alloy::providers::Provider;
use eyre::Result;
use serde::Serialize;

use crate::anvil::AnvilProvider;
use crate::contract::IClaimVerifier;

/// Hardcoded stub roots for the claimer.
pub const ARTIFACT_ROOT: [u8; 32] = [0xaa; 32];
pub const RESULT_ROOT: [u8; 32] = [0xbb; 32];
pub const WORKLOAD_ID: [u8; 32] = [0x01; 32];

/// Result of a successful claim submission.
#[derive(Debug, Clone, Serialize)]
pub struct ClaimResult {
    pub claim_id: U256,
    pub contract_address: Address,
    pub tx_hash: String,
    pub gas_used: u64,
    pub block_number: u64,
    pub block_timestamp: u64,
    pub workload_id: String,
    pub artifact_root: String,
    pub result_root: String,
    pub state: String,
}

/// Submit a claim to the deployed ClaimVerifier contract.
///
/// Uses hardcoded stub roots (`0xaa..`, `0xbb..`) and workload ID (`0x01..`).
/// Returns a `ClaimResult` with all receipt data.
pub async fn submit_claim(
    provider: &AnvilProvider,
    contract_address: Address,
) -> Result<ClaimResult> {
    let workload_id = FixedBytes::from(WORKLOAD_ID);
    let artifact_root = FixedBytes::from(ARTIFACT_ROOT);
    let result_root = FixedBytes::from(RESULT_ROOT);

    let contract = IClaimVerifier::new(contract_address, provider);
    let pending = contract
        .submitClaim(workload_id, artifact_root, result_root)
        .send()
        .await?;
    let receipt = pending.get_receipt().await?;

    // Decode ClaimSubmitted event
    let claim_id = receipt
        .inner
        .logs()
        .iter()
        .find_map(|log| {
            log.log_decode::<IClaimVerifier::ClaimSubmitted>()
                .ok()
                .map(|decoded| decoded.inner.claimId)
        })
        .ok_or_else(|| eyre::eyre!("ClaimSubmitted event not found in receipt"))?;

    // Get block timestamp
    let block = provider
        .get_block_by_number(receipt.block_number.unwrap().into())
        .await?
        .ok_or_else(|| eyre::eyre!("Block not found"))?;

    Ok(ClaimResult {
        claim_id,
        contract_address,
        tx_hash: format!("{}", receipt.transaction_hash),
        gas_used: receipt.gas_used,
        block_number: receipt.block_number.unwrap(),
        block_timestamp: block.header.timestamp,
        workload_id: format!("0x{}", alloy::hex::encode(workload_id)),
        artifact_root: format!("0x{}", alloy::hex::encode(artifact_root)),
        result_root: format!("0x{}", alloy::hex::encode(result_root)),
        state: "Pending".to_string(),
    })
}
