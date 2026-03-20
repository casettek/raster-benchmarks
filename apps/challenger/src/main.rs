use std::env;

use alloy::primitives::U256;
use eyre::Result;
use shared::challenger::ReplayMode;
use shared::claimer::default_l2_claim_input;

#[tokio::main]
async fn main() -> Result<()> {
    // Config from environment
    let anvil_url = env::var("ANVIL_URL").map_err(|_| {
        eyre::eyre!("ANVIL_URL is required (challenger connects to existing Anvil)")
    })?;
    let contract_address: alloy::primitives::Address = env::var("CONTRACT_ADDRESS")
        .map_err(|_| eyre::eyre!("CONTRACT_ADDRESS is required"))?
        .parse()
        .map_err(|e| eyre::eyre!("Invalid CONTRACT_ADDRESS: {e}"))?;
    let claim_id: U256 = env::var("CLAIM_ID")
        .map_err(|_| eyre::eyre!("CLAIM_ID is required"))?
        .parse()
        .map_err(|e| eyre::eyre!("Invalid CLAIM_ID: {e}"))?;
    let mode = env::var("MODE").unwrap_or_else(|_| "honest".to_string());

    // Connect to existing Anvil
    eprintln!("Connecting to Anvil at {anvil_url}");
    let provider = shared::anvil::connect_provider(&anvil_url)?;

    // Use default L2 claim input for standalone challenger usage
    let l2_input = default_l2_claim_input();

    match mode.as_str() {
        "honest" => {
            eprintln!("Replaying + resolving claim {claim_id} (honest mode)...");
            let result = shared::challenger::resolve_claim_with_replay(
                &provider,
                contract_address,
                claim_id,
                ReplayMode::Honest,
                &l2_input,
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        "dishonest" => {
            eprintln!("Replaying + resolving claim {claim_id} (dishonest simulation)...");
            let result = shared::challenger::resolve_claim_with_replay(
                &provider,
                contract_address,
                claim_id,
                ReplayMode::DishonestSimulation,
                &l2_input,
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        other => {
            return Err(eyre::eyre!(
                "Unknown MODE '{other}'. Expected 'honest' or 'dishonest'."
            ));
        }
    }

    Ok(())
}
