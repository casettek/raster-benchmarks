use std::path::PathBuf;

use alloy::primitives::{Address, B256, Bytes};
use eyre::{Context, Result};
use serde::Serialize;

use crate::anvil::AnvilProvider;
use crate::contract::IClaimVerifier;

pub const TRACE_CODEC_NDJSON_V1: u8 = 1;

#[derive(Debug, Clone, Serialize)]
pub struct TracePublication {
    pub trace_tx_hash: String,
    pub payload_hash: String,
    pub payload_bytes: u32,
    pub codec_id: u8,
    pub gas_used: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct TracePointerIndex {
    pub run_id: String,
    pub trace_tx_hash: String,
    pub payload_hash: String,
    pub payload_bytes: u32,
    pub codec_id: u8,
}

pub async fn publish_trace(
    provider: &AnvilProvider,
    contract_address: Address,
    payload: Vec<u8>,
    codec_id: u8,
) -> Result<TracePublication> {
    let payload_bytes =
        u32::try_from(payload.len()).wrap_err("trace payload too large for u32 length")?;

    let contract = IClaimVerifier::new(contract_address, provider);
    let pending = contract
        .publishTrace(Bytes::from(payload), codec_id)
        .send()
        .await?;
    let receipt = pending.get_receipt().await?;

    let published = receipt
        .inner
        .logs()
        .iter()
        .find_map(|log| {
            log.log_decode::<IClaimVerifier::TracePublished>()
                .ok()
                .map(|decoded| decoded.inner)
        })
        .ok_or_else(|| eyre::eyre!("TracePublished event not found in receipt"))?;

    Ok(TracePublication {
        trace_tx_hash: format!("{}", receipt.transaction_hash),
        payload_hash: format!("0x{}", alloy::hex::encode(published.payloadHash)),
        payload_bytes,
        codec_id: published.codecId,
        gas_used: receipt.gas_used,
    })
}

pub fn persist_trace_index(run_id: &str, publication: &TracePublication) -> Result<PathBuf> {
    let index_dir = PathBuf::from("runs").join("blob-index");
    std::fs::create_dir_all(&index_dir).wrap_err("failed to create runs/blob-index")?;

    let index = TracePointerIndex {
        run_id: run_id.to_string(),
        trace_tx_hash: publication.trace_tx_hash.clone(),
        payload_hash: publication.payload_hash.clone(),
        payload_bytes: publication.payload_bytes,
        codec_id: publication.codec_id,
    };
    let json = serde_json::to_string_pretty(&index)?;

    let path = index_dir.join(format!("{run_id}.json"));
    std::fs::write(&path, json.as_bytes()).wrap_err("failed to write blob index file")?;
    Ok(path)
}

pub fn parse_trace_tx_hash(hash: &str) -> Result<B256> {
    hash.parse::<B256>()
        .wrap_err("invalid trace tx hash in publication pointer")
}
