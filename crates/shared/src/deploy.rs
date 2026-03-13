use std::path::Path;

use alloy::hex;
use alloy::network::TransactionBuilder;
use alloy::primitives::Address;
use alloy::providers::Provider;
use alloy::rpc::types::TransactionRequest;
use eyre::{Result, eyre};

use crate::anvil::AnvilProvider;

/// Deploy the ClaimVerifier contract from Foundry build artifacts.
///
/// Reads `<forge_out_dir>/ClaimVerifier.sol/ClaimVerifier.json` for the
/// contract bytecode, deploys it, waits for the receipt, and returns the
/// deployed address.
pub async fn deploy_claim_verifier(
    provider: &AnvilProvider,
    forge_out_dir: &Path,
) -> Result<Address> {
    let artifact_path = forge_out_dir
        .join("ClaimVerifier.sol")
        .join("ClaimVerifier.json");
    let artifact_json = std::fs::read_to_string(&artifact_path).map_err(|e| {
        eyre!(
            "Failed to read contract artifact at {}: {}. Run `forge build` in contracts/ first.",
            artifact_path.display(),
            e
        )
    })?;

    let artifact: serde_json::Value = serde_json::from_str(&artifact_json)?;

    // Foundry artifacts store bytecode under bytecode.object
    let bytecode_hex = artifact["bytecode"]["object"]
        .as_str()
        .ok_or_else(|| eyre!("Missing bytecode.object in contract artifact"))?;

    // Strip 0x prefix if present
    let bytecode_hex = bytecode_hex.strip_prefix("0x").unwrap_or(bytecode_hex);
    let bytecode = hex::decode(bytecode_hex)?;

    let tx = TransactionRequest::default().with_deploy_code(bytecode);
    let pending = provider.send_transaction(tx).await?;
    let receipt = pending.get_receipt().await?;

    receipt
        .contract_address
        .ok_or_else(|| eyre!("No contract address in deploy receipt"))
}
