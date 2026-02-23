use std::env;

fn main() {
    let contract = env::var("CLAIM_VERIFIER_ADDRESS")
        .unwrap_or_else(|_| "<set CLAIM_VERIFIER_ADDRESS>".to_string());
    let claim_id = env::var("CLAIM_ID").unwrap_or_else(|_| "<set CLAIM_ID>".to_string());
    let observed_artifact_root = env::var("OBSERVED_ARTIFACT_ROOT")
        .unwrap_or_else(|_| "<set OBSERVED_ARTIFACT_ROOT>".to_string());
    let observed_result_root = env::var("OBSERVED_RESULT_ROOT")
        .unwrap_or_else(|_| "<set OBSERVED_RESULT_ROOT>".to_string());

    println!("challenger-app starter");
    println!("contract: {contract}");
    println!("claim_id: {claim_id}");
    println!("observed_artifact_root: {observed_artifact_root}");
    println!("observed_result_root: {observed_result_root}");
    println!("next: wire this app to call challengeClaim()");
}
