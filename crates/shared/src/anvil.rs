use alloy::network::EthereumWallet;
use alloy::node_bindings::AnvilInstance;
use alloy::providers::ProviderBuilder;
use alloy::signers::local::PrivateKeySigner;
use eyre::Result;

/// Anvil instance handle + provider type returned by `spawn_anvil`.
pub type AnvilProvider = alloy::providers::fillers::FillProvider<
    alloy::providers::fillers::JoinFill<
        alloy::providers::fillers::JoinFill<
            alloy::providers::Identity,
            alloy::providers::fillers::JoinFill<
                alloy::providers::fillers::GasFiller,
                alloy::providers::fillers::JoinFill<
                    alloy::providers::fillers::BlobGasFiller,
                    alloy::providers::fillers::JoinFill<
                        alloy::providers::fillers::NonceFiller,
                        alloy::providers::fillers::ChainIdFiller,
                    >,
                >,
            >,
        >,
        alloy::providers::fillers::WalletFiller<EthereumWallet>,
    >,
    alloy::providers::RootProvider,
>;

/// Spawn a local Anvil instance and return the handle + connected provider.
///
/// The provider is configured with the first Anvil dev account as signer.
/// Hold the returned `AnvilInstance` handle — dropping it kills the process.
pub fn spawn_anvil() -> Result<(AnvilInstance, AnvilProvider)> {
    let anvil = alloy::node_bindings::Anvil::new()
        .args(["--hardfork", "cancun"])
        .try_spawn()?;
    let signer: PrivateKeySigner = anvil.keys()[0].clone().into();
    let wallet = EthereumWallet::from(signer);
    let provider = ProviderBuilder::new()
        .wallet(wallet)
        .connect_http(anvil.endpoint_url());
    Ok((anvil, provider))
}

/// Connect to an existing JSON-RPC endpoint with the default Anvil dev key.
///
/// Uses the well-known Anvil dev private key #0:
/// `0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80`
pub fn connect_provider(url: &str) -> Result<AnvilProvider> {
    let signer: PrivateKeySigner =
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80".parse()?;
    let wallet = EthereumWallet::from(signer);
    let rpc_url: alloy::transports::http::reqwest::Url = url.parse()?;
    let provider = ProviderBuilder::new().wallet(wallet).connect_http(rpc_url);
    Ok(provider)
}
