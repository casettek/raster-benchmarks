use alloy::primitives::{Address, FixedBytes, U256};
use alloy::providers::Provider;
use eyre::Result;
use serde::Serialize;

use crate::anvil::AnvilProvider;
use crate::contract::IClaimVerifier;
use crate::da::TracePublication;

/// L2 claim fields derived from workload execution and the canonical fixture.
#[derive(Debug, Clone)]
pub struct L2ClaimInput {
    pub prev_output_root: [u8; 32],
    pub next_output_root: [u8; 32],
    pub start_block: u64,
    pub end_block: u64,
    pub batch_hash: [u8; 32],
}

/// Hardcoded stub L2 claim fields for non-L2 workloads (raster-hello, stub).
pub fn stub_l2_claim_input() -> L2ClaimInput {
    L2ClaimInput {
        prev_output_root: [0xaa; 32],
        next_output_root: [0xbb; 32],
        start_block: 0,
        end_block: 0,
        batch_hash: [0x01; 32],
    }
}

/// Result of a successful claim submission.
#[derive(Debug, Clone, Serialize)]
pub struct ClaimResult {
    pub claim_id: U256,
    pub contract_address: Address,
    pub tx_hash: String,
    pub gas_used: u64,
    pub block_number: u64,
    pub block_timestamp: u64,
    pub prev_output_root: String,
    pub next_output_root: String,
    pub start_block: u64,
    pub end_block: u64,
    pub batch_hash: String,
    pub input_blob_versioned_hash: String,
    pub bond_amount: String,
    pub challenge_deadline: u64,
    pub trace_tx_hash: String,
    pub trace_payload_bytes: u32,
    pub trace_codec_id: u8,
    pub state: String,
}

/// Submit an L2 settlement claim to the deployed ClaimVerifier contract.
///
/// The caller provides L2-specific claim fields (output roots, block range,
/// batch hash) and optional trace publication metadata.
pub async fn submit_claim(
    provider: &AnvilProvider,
    contract_address: Address,
    l2_input: &L2ClaimInput,
    trace_publication: Option<&TracePublication>,
    bond_value: U256,
) -> Result<ClaimResult> {
    let prev_output_root = FixedBytes::from(l2_input.prev_output_root);
    let next_output_root = FixedBytes::from(l2_input.next_output_root);
    let batch_hash = FixedBytes::from(l2_input.batch_hash);

    let trace_tx_hash = match trace_publication {
        Some(publication) => crate::da::parse_trace_tx_hash(&publication.trace_tx_hash)?,
        None => alloy::primitives::B256::ZERO,
    };
    let trace_payload_bytes = trace_publication.map_or(0, |publication| publication.payload_bytes);
    let trace_codec_id = trace_publication.map_or(0, |publication| publication.codec_id);

    let contract = IClaimVerifier::new(contract_address, provider);
    let pending = contract
        .submitClaim(
            prev_output_root,
            next_output_root,
            l2_input.start_block,
            l2_input.end_block,
            batch_hash,
            trace_tx_hash,
            trace_payload_bytes,
            trace_codec_id,
        )
        .value(bond_value)
        .send()
        .await?;
    let receipt = pending.get_receipt().await?;

    // Decode ClaimSubmitted event
    let submitted = receipt
        .inner
        .logs()
        .iter()
        .find_map(|log| {
            log.log_decode::<IClaimVerifier::ClaimSubmitted>()
                .ok()
                .map(|decoded| decoded.inner)
        })
        .ok_or_else(|| eyre::eyre!("ClaimSubmitted event not found in receipt"))?;

    // Get block timestamp
    let block = provider
        .get_block_by_number(receipt.block_number.unwrap().into())
        .await?
        .ok_or_else(|| eyre::eyre!("Block not found"))?;

    Ok(ClaimResult {
        claim_id: submitted.claimId,
        contract_address,
        tx_hash: format!("{}", receipt.transaction_hash),
        gas_used: receipt.gas_used,
        block_number: receipt.block_number.unwrap(),
        block_timestamp: block.header.timestamp,
        prev_output_root: format!("0x{}", alloy::hex::encode(prev_output_root)),
        next_output_root: format!("0x{}", alloy::hex::encode(next_output_root)),
        start_block: l2_input.start_block,
        end_block: l2_input.end_block,
        batch_hash: format!("0x{}", alloy::hex::encode(batch_hash)),
        input_blob_versioned_hash: format!(
            "0x{}",
            alloy::hex::encode(submitted.inputBlobVersionedHash)
        ),
        bond_amount: format!("{}", submitted.bondAmount),
        challenge_deadline: submitted.challengeDeadline,
        trace_tx_hash: format!("0x{}", alloy::hex::encode(trace_tx_hash)),
        trace_payload_bytes,
        trace_codec_id,
        state: "Pending".to_string(),
    })
}
