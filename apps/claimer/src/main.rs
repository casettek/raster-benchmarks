use std::env;
use std::path::PathBuf;

use eyre::{Result, eyre};
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

    // Standalone claim submission is no longer allowed without a published trace pointer.
    let _ = default_l2_claim_input();
    let _ = DEFAULT_MIN_BOND;
    Err(eyre!(
        "standalone claimer no longer supports zero trace pointers; submit through runner or web-server after DA publication"
    ))
}
