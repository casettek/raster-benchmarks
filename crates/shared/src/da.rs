use std::fs;
use std::path::PathBuf;

use alloy::consensus::{SidecarBuilder, SidecarCoder, SimpleCoder};
use alloy::eips::eip4844::Blob;
use alloy::network::{TransactionBuilder, TransactionBuilder4844};
use alloy::primitives::{Address, B256, keccak256};
use alloy::providers::Provider;
use alloy::rpc::types::TransactionRequest;
use eyre::{Context, Result, eyre};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::anvil::AnvilProvider;

pub const TRACE_CODEC_COMMITMENT_JSON_V1: u8 = 2;
pub const INPUT_CODEC_TAR_V1: u8 = 10;
pub const INPUT_ARTIFACT_KIND: &str = "input-package";
pub const TRACE_ARTIFACT_KIND: &str = "trace-commitment";
const BLOB_SINK_ADDRESS: Address = Address::ZERO;
const SINGLE_BLOB_PAYLOAD_BYTES: usize = 120 * 1024;
const MANIFEST_SCHEMA_VERSION: u8 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlobPublication {
    pub kind: String,
    pub codec_id: u8,
    pub manifest_tx_hash: String,
    pub manifest_blob_versioned_hash: String,
    pub payload_hash: String,
    pub payload_bytes: u64,
    pub chunk_count: u32,
    pub total_gas_used: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlobChunkRef {
    pub tx_hash: String,
    pub blob_versioned_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlobManifest {
    pub schema_version: u8,
    pub kind: String,
    pub codec_id: u8,
    pub payload_hash: String,
    pub payload_bytes: u64,
    pub chunks: Vec<BlobChunkRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunBlobIndex {
    pub run_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<BlobManifestIndex>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace: Option<BlobManifestIndex>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlobManifestIndex {
    pub publication: BlobPublication,
    pub chunks: Vec<BlobChunkRef>,
}

pub async fn publish_input_package(
    provider: &AnvilProvider,
    payload: Vec<u8>,
) -> Result<(BlobPublication, BlobManifest)> {
    publish_blob_artifact(provider, INPUT_ARTIFACT_KIND, INPUT_CODEC_TAR_V1, payload).await
}

pub async fn publish_trace_commitment(
    provider: &AnvilProvider,
    payload: Vec<u8>,
) -> Result<(BlobPublication, BlobManifest)> {
    publish_blob_artifact(provider, TRACE_ARTIFACT_KIND, TRACE_CODEC_COMMITMENT_JSON_V1, payload)
        .await
}

pub fn persist_blob_index(
    run_id: &str,
    input: Option<(&BlobPublication, &BlobManifest)>,
    trace: Option<(&BlobPublication, &BlobManifest)>,
) -> Result<PathBuf> {
    let index_dir = PathBuf::from("runs").join("blob-index");
    fs::create_dir_all(&index_dir).wrap_err("failed to create runs/blob-index")?;

    let index = RunBlobIndex {
        run_id: run_id.to_string(),
        input: input.map(|(publication, manifest)| BlobManifestIndex {
            publication: publication.clone(),
            chunks: manifest.chunks.clone(),
        }),
        trace: trace.map(|(publication, manifest)| BlobManifestIndex {
            publication: publication.clone(),
            chunks: manifest.chunks.clone(),
        }),
    };

    let path = index_dir.join(format!("{run_id}.json"));
    let json = serde_json::to_string_pretty(&index)?;
    fs::write(&path, json.as_bytes()).wrap_err("failed to write blob index file")?;
    Ok(path)
}

pub async fn fetch_blob_artifact(
    provider: &AnvilProvider,
    manifest_blob_versioned_hash: B256,
) -> Result<(BlobManifest, Vec<u8>)> {
    let manifest_bytes = fetch_blob_bytes_by_hash(provider, manifest_blob_versioned_hash).await?;
    let manifest: BlobManifest = serde_json::from_slice(&manifest_bytes)
        .wrap_err("failed to decode blob manifest payload")?;

    let mut payload = Vec::with_capacity(manifest.payload_bytes as usize);
    for chunk in &manifest.chunks {
        let hash = parse_blob_versioned_hash(&chunk.blob_versioned_hash)?;
        let bytes = fetch_blob_bytes_by_hash(provider, hash).await?;
        payload.extend_from_slice(&bytes);
    }
    payload.truncate(manifest.payload_bytes as usize);

    let observed_payload_hash = format!("0x{}", alloy::hex::encode(Sha256::digest(&payload)));
    if observed_payload_hash != manifest.payload_hash {
        return Err(eyre!(
            "blob artifact payload hash mismatch: expected {}, got {}",
            manifest.payload_hash,
            observed_payload_hash
        ));
    }

    Ok((manifest, payload))
}

pub fn parse_blob_versioned_hash(hash: &str) -> Result<B256> {
    hash.parse::<B256>()
        .wrap_err("invalid blob versioned hash in publication pointer")
}

async fn publish_blob_artifact(
    provider: &AnvilProvider,
    kind: &str,
    codec_id: u8,
    payload: Vec<u8>,
) -> Result<(BlobPublication, BlobManifest)> {
    let payload_hash = format!("0x{}", alloy::hex::encode(Sha256::digest(&payload)));
    let payload_bytes =
        u64::try_from(payload.len()).wrap_err("blob artifact payload too large for u64 length")?;

    let mut chunks = Vec::new();
    let mut total_gas_used = 0u64;
    for bytes in payload.chunks(SINGLE_BLOB_PAYLOAD_BYTES) {
        let chunk = publish_single_blob(provider, bytes.to_vec()).await?;
        total_gas_used = total_gas_used.saturating_add(chunk.gas_used);
        chunks.push(BlobChunkRef {
            tx_hash: chunk.tx_hash,
            blob_versioned_hash: chunk.blob_versioned_hash,
        });
    }

    let manifest = BlobManifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        kind: kind.to_string(),
        codec_id,
        payload_hash: payload_hash.clone(),
        payload_bytes,
        chunks,
    };

    let manifest_payload = serde_json::to_vec_pretty(&manifest)?;
    if manifest_payload.len() > SINGLE_BLOB_PAYLOAD_BYTES {
        return Err(eyre!(
            "blob manifest too large to fit in a single blob: {} bytes",
            manifest_payload.len()
        ));
    }

    let manifest_chunk = publish_single_blob(provider, manifest_payload).await?;
    total_gas_used = total_gas_used.saturating_add(manifest_chunk.gas_used);

    Ok((
        BlobPublication {
            kind: kind.to_string(),
            codec_id,
            manifest_tx_hash: manifest_chunk.tx_hash,
            manifest_blob_versioned_hash: manifest_chunk.blob_versioned_hash,
            payload_hash,
            payload_bytes,
            chunk_count: u32::try_from(manifest.chunks.len())
                .wrap_err("too many blob chunks for u32 count")?,
            total_gas_used,
        },
        manifest,
    ))
}

async fn publish_single_blob(provider: &AnvilProvider, payload: Vec<u8>) -> Result<SingleBlobTx> {
    let sidecar = SidecarBuilder::<SimpleCoder>::from_slice(&payload)
        .build()
        .wrap_err("failed to build blob sidecar")?;
    let versioned_hashes: Vec<_> = sidecar.versioned_hashes().collect();
    if versioned_hashes.len() != 1 {
        return Err(eyre!(
            "expected a single blob chunk, got {} blobs",
            versioned_hashes.len()
        ));
    }

    let tx = TransactionRequest::default()
        .with_to(BLOB_SINK_ADDRESS)
        .with_blob_sidecar(sidecar);

    let pending = provider.send_transaction(tx).await?;
    let receipt = pending.get_receipt().await?;

    Ok(SingleBlobTx {
        tx_hash: format!("{}", receipt.transaction_hash),
        blob_versioned_hash: format!("0x{}", alloy::hex::encode(versioned_hashes[0])),
        gas_used: receipt.gas_used,
    })
}

async fn fetch_blob_bytes_by_hash(provider: &AnvilProvider, blob_hash: B256) -> Result<Vec<u8>> {
    let raw_blob: String = provider
        .raw_request(
            "anvil_getBlobByHash".into(),
            [serde_json::Value::String(format!("0x{}", alloy::hex::encode(blob_hash)))],
        )
        .await
        .wrap_err("failed to fetch blob bytes from Anvil")?;

    let blob_bytes = alloy::hex::decode(raw_blob.trim_start_matches("0x"))
        .wrap_err("blob response was not valid hex")?;
    let blob = Blob::try_from(blob_bytes.as_slice())
        .map_err(|_| eyre!("blob response had invalid length: {}", blob_bytes.len()))?;
    let decoded = SimpleCoder::default()
        .decode_all(&[blob])
        .ok_or_else(|| eyre!("failed to decode blob payload with SimpleCoder"))?;
    let payload = decoded
        .into_iter()
        .next()
        .ok_or_else(|| eyre!("blob payload decoded to an empty chunk set"))?;
    Ok(payload)
}

#[derive(Debug)]
struct SingleBlobTx {
    tx_hash: String,
    blob_versioned_hash: String,
    gas_used: u64,
}

#[allow(dead_code)]
fn _keccak_hex(payload: &[u8]) -> String {
    format!("0x{}", alloy::hex::encode(keccak256(payload)))
}
