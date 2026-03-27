use std::path::Path;

use alloy::hex;
use alloy::network::TransactionBuilder;
use alloy::primitives::{Address, U256};
use alloy::providers::Provider;
use alloy::rpc::types::TransactionRequest;
use alloy::sol_types::SolConstructor;
use eyre::{Result, eyre};

use crate::anvil::AnvilProvider;
use crate::contract::ClaimVerifier;

/// Default challenge period for local Anvil runs (seconds).
pub const DEFAULT_CHALLENGE_PERIOD: u64 = 120;

/// Default minimum bond for local Anvil runs (0.01 ether).
pub const DEFAULT_MIN_BOND: U256 = U256::from_limbs([10_000_000_000_000_000, 0, 0, 0]);

/// Default blob retention window for local Anvil runs (18 days).
pub const DEFAULT_BLOB_RETENTION_WINDOW: u64 = 18 * 24 * 60 * 60;

/// Deploy the ClaimVerifier contract from Foundry build artifacts.
///
/// Reads `<forge_out_dir>/ClaimVerifier.sol/ClaimVerifier.json` for the
/// contract bytecode, appends constructor arguments, deploys it, waits for
/// the receipt, and returns the deployed address.
///
/// Uses `DEFAULT_CHALLENGE_PERIOD` and `DEFAULT_MIN_BOND` as constructor
/// parameters.
pub async fn deploy_claim_verifier(
    provider: &AnvilProvider,
    forge_out_dir: &Path,
) -> Result<Address> {
    deploy_claim_verifier_with_config(
        provider,
        forge_out_dir,
        DEFAULT_CHALLENGE_PERIOD,
        DEFAULT_MIN_BOND,
        DEFAULT_BLOB_RETENTION_WINDOW,
    )
    .await
}

/// Deploy the ClaimVerifier contract with custom challenge period, bond, and
/// blob retention window.
pub async fn deploy_claim_verifier_with_config(
    provider: &AnvilProvider,
    forge_out_dir: &Path,
    challenge_period: u64,
    min_bond: U256,
    blob_retention_window: u64,
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

    // Encode constructor arguments
    let constructor_args =
        ClaimVerifier::constructorCall::new((challenge_period, min_bond, blob_retention_window))
            .abi_encode();

    // Concatenate bytecode + constructor args
    let mut deploy_code = bytecode;
    deploy_code.extend_from_slice(&constructor_args);

    let tx = TransactionRequest::default().with_deploy_code(deploy_code);
    let pending = provider.send_transaction(tx).await?;
    let receipt = pending.get_receipt().await?;

    receipt
        .contract_address
        .ok_or_else(|| eyre!("No contract address in deploy receipt"))
}
