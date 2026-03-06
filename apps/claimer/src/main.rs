use std::env;
use std::path::PathBuf;

use alloy::primitives::FixedBytes;
use alloy::providers::Provider;
use eyre::Result;
use shared::contract::IClaimVerifier;

#[tokio::main]
async fn main() -> Result<()> {
    // Config from environment
    let anvil_url = env::var("ANVIL_URL").ok();
    let forge_out = env::var("FORGE_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("contracts/out"));

    // Spawn or connect to Anvil
    let (_anvil_handle, provider) = if let Some(url) = &anvil_url {
        eprintln!("Connecting to external Anvil at {url}");
        let provider = shared::anvil::connect_provider(url)?;
        (None, provider)
    } else {
        eprintln!("Spawning local Anvil instance...");
        let (anvil, provider) = shared::anvil::spawn_anvil()?;
        (Some(anvil), provider)
    };

    // Deploy ClaimVerifier
    eprintln!("Deploying ClaimVerifier from {}", forge_out.display());
    let contract_address = shared::deploy::deploy_claim_verifier(&provider, &forge_out).await?;
    eprintln!("ClaimVerifier deployed at {contract_address}");

    // Hardcoded stub values
    let workload_id = FixedBytes::from([0x01u8; 32]);
    let artifact_root = FixedBytes::from([0xaau8; 32]);
    let result_root = FixedBytes::from([0xbbu8; 32]);

    // Submit claim
    let contract = IClaimVerifier::new(contract_address, &provider);
    let pending = contract
        .submitClaim(workload_id, artifact_root, result_root)
        .send()
        .await?;
    let receipt = pending.get_receipt().await?;

    // Decode ClaimSubmitted event from receipt logs
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

    // Emit structured JSON to stdout
    let output = serde_json::json!({
        "claim_id": claim_id.to_string(),
        "contract_address": format!("{contract_address}"),
        "tx_hash": format!("{}", receipt.transaction_hash),
        "gas_used": receipt.gas_used,
        "block_number": receipt.block_number,
        "block_timestamp": block.header.timestamp,
        "workload_id": format!("0x{}", alloy::hex::encode(workload_id)),
        "artifact_root": format!("0x{}", alloy::hex::encode(artifact_root)),
        "result_root": format!("0x{}", alloy::hex::encode(result_root)),
        "state": "Pending"
    });

    println!("{}", serde_json::to_string_pretty(&output)?);

    Ok(())
}
