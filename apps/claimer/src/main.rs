use std::env;

fn main() {
    let contract = env::var("CLAIM_VERIFIER_ADDRESS")
        .unwrap_or_else(|_| "<set CLAIM_VERIFIER_ADDRESS>".to_string());
    let workload_id =
        env::var("WORKLOAD_ID").unwrap_or_else(|_| "<set WORKLOAD_ID>".to_string());
    let artifact_root = env::var("ARTIFACT_ROOT")
        .unwrap_or_else(|_| "<set ARTIFACT_ROOT>".to_string());
    let result_root =
        env::var("RESULT_ROOT").unwrap_or_else(|_| "<set RESULT_ROOT>".to_string());

    println!("claimer-app starter");
    println!("contract: {contract}");
    println!("workload_id: {workload_id}");
    println!("artifact_root: {artifact_root}");
    println!("result_root: {result_root}");
    println!("next: wire this app to call submitClaim()/settleClaim()");
}
