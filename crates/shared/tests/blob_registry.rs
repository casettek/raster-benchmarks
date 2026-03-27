use std::path::PathBuf;
use std::process::Command;

use alloy::primitives::B256;
use shared::contract::IClaimVerifier;

fn forge_out_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("contracts")
        .join("out")
}

#[tokio::test]
async fn manifest_publication_registers_blob_hash_onchain() {
    if Command::new("anvil").arg("--version").output().is_err() {
        eprintln!("skipping blob registry integration test: anvil not available");
        return;
    }

    let (_anvil, provider) = shared::anvil::spawn_anvil().expect("spawn anvil");
    let contract_address = shared::deploy::deploy_claim_verifier(&provider, &forge_out_dir())
        .await
        .expect("deploy verifier");

    let payload = br#"{"kind":"trace-commitment","value":"ok"}"#.to_vec();
    let (publication, _manifest) =
        shared::da::publish_trace_commitment(&provider, contract_address, payload.clone())
            .await
            .expect("publish trace commitment");

    assert!(publication.registration_block_number.is_some());
    assert!(publication.registration_timestamp.is_some());

    let contract = IClaimVerifier::new(contract_address, &provider);
    let registration = contract
        .getBlobRegistration(
            publication
                .manifest_blob_versioned_hash
                .parse::<B256>()
                .expect("parse manifest hash"),
        )
        .call()
        .await
        .expect("load blob registration");

    assert_eq!(
        registration.blockNumber,
        publication.registration_block_number.expect("registration block"),
    );
    assert_eq!(
        registration.timestamp,
        publication.registration_timestamp.expect("registration timestamp"),
    );

    let (_manifest, fetched_payload) = shared::da::fetch_blob_artifact(
        &provider,
        publication
            .manifest_blob_versioned_hash
            .parse::<B256>()
            .expect("parse manifest hash"),
    )
    .await
    .expect("fetch blob artifact");
    assert_eq!(fetched_payload, payload);
}
