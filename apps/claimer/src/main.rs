use std::env;
use std::path::PathBuf;

use eyre::Result;
use shared::claimer::default_l2_claim_input;
use shared::deploy::DEFAULT_MIN_BOND;

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

    // Submit claim via shared library with default L2 fields
    let l2_input = default_l2_claim_input();
    let result = shared::claimer::submit_claim(
        &provider,
        contract_address,
        &l2_input,
        None,
        DEFAULT_MIN_BOND,
    )
    .await?;
    println!("{}", serde_json::to_string_pretty(&result)?);

    Ok(())
}
