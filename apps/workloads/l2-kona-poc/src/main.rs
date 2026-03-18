use std::env;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use alloy_consensus::{Header, Sealable};
use alloy_op_evm::OpEvmFactory;
use alloy_primitives::{Address, B256, Bytes, FixedBytes, keccak256};
use alloy_rlp::Decodable;
use alloy_rpc_types_engine::PayloadAttributes;
use kona_executor::{StatelessL2Builder, TrieDBProvider};
use kona_genesis::RollupConfig;
use kona_mpt::{NoopTrieHinter, TrieNode, TrieProvider};
use op_alloy_rpc_types_engine::OpPayloadAttributes;
use rocksdb::DB;
use serde::Deserialize;
use serde_json::{Value, json};

const CANONICAL_FIXTURE_PATH: &str = "runs/fixtures/l2-poc-synth-fixture.json";

#[derive(Debug, thiserror::Error)]
enum WorkloadError {
    #[error("missing --input '<fixture-json>'")]
    MissingInput,
    #[error("invalid --execution-mode '{value}' (expected 'strict' or 'fallback')")]
    InvalidExecutionMode { value: String },
    #[error("fixture parse failed: {0}")]
    FixtureParse(#[from] serde_json::Error),
    #[error("invalid fixture: {0}")]
    InvalidFixture(String),
    #[error("io error at {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    #[error("rocksdb error at {path}: {source}")]
    RocksDb {
        path: String,
        source: rocksdb::Error,
    },
    #[error("invalid hash '{field}': {value}")]
    InvalidHash { field: &'static str, value: String },
    #[error("invalid address '{field}': {value}")]
    InvalidAddress { field: &'static str, value: String },
    #[error("invalid tx hex in {tx_id}")]
    InvalidTxHex { tx_id: String },
    #[error("missing trie preimage for hash {hash:#x}")]
    MissingPreimage { hash: B256 },
    #[error("missing bytecode preimage for hash {hash:#x}")]
    MissingBytecode { hash: B256 },
    #[error(
        "strict canonical execution failed at step {step_index} ({tx_id}): missing {class} witness data ({detail})"
    )]
    MissingWitness {
        step_index: usize,
        tx_id: String,
        class: &'static str,
        detail: String,
    },
    #[error("header decode failed for hash {hash:#x}: {source}")]
    HeaderDecode {
        hash: B256,
        source: alloy_rlp::Error,
    },
    #[error("trie node decode failed for hash {hash:#x}: {source}")]
    TrieNodeDecode {
        hash: B256,
        source: alloy_rlp::Error,
    },
    #[error("kona execution failed at {tx_id}: {reason}")]
    KonaExecution { tx_id: String, reason: String },
}

type Result<T> = std::result::Result<T, WorkloadError>;

#[derive(Debug, Deserialize)]
struct FixtureInput {
    fixture_id: String,
    pre_checkpoint: PreCheckpoint,
    output_root_witness: OutputRootWitness,
    transactions: Vec<FixtureTransaction>,
    #[serde(default)]
    supplemental_transactions: Vec<FixtureTransaction>,
    start_block: u64,
    end_block: u64,
    start_timestamp: u64,
    timestamp_delta_seconds: u64,
    gas_limit: u64,
    fee_recipient: String,
    batch_hash: String,
}

#[derive(Debug, Deserialize)]
struct OutputRootWitness {
    message_passer_storage_root: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExecutionMode {
    Strict,
    Fallback,
}

impl ExecutionMode {
    fn from_cli(value: Option<String>) -> Result<Self> {
        match value.as_deref() {
            None | Some("strict") => Ok(Self::Strict),
            Some("fallback") => Ok(Self::Fallback),
            Some(other) => Err(WorkloadError::InvalidExecutionMode {
                value: other.to_string(),
            }),
        }
    }

    fn allow_fallback(self) -> bool {
        matches!(self, Self::Fallback)
    }
}

struct CliArgs {
    input_json: String,
    execution_mode: ExecutionMode,
}

#[derive(Debug, Deserialize)]
struct PreCheckpoint {
    prev_output_root: String,
    parent_header_hash: String,
    parent_block_number: u64,
    rollup_config_ref: String,
    witness_bundle_ref: String,
    prev_randao: String,
    parent_beacon_block_root: String,
}

#[derive(Debug, Clone, Deserialize)]
struct FixtureTransaction {
    id: String,
    raw: String,
    hash: String,
}

#[derive(Debug, Deserialize)]
struct WitnessBundle {
    kv_store_ref: String,
    kv_store_refs: Option<Vec<String>>,
    parent_header: Value,
    source_payload_template: Option<SourcePayloadTemplate>,
    closure_manifest_ref: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SourcePayloadTemplate {
    #[serde(rename = "eip1559Params")]
    eip_1559_params: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WitnessClosureManifest {
    fixture_ref: String,
    identity: WitnessClosureIdentity,
    rollup_config_ref: String,
    kv_store_refs: Vec<String>,
    witness_bundle_ref: String,
}

#[derive(Debug, Deserialize)]
struct WitnessClosureIdentity {
    fixture_id: String,
    batch_hash: String,
    tx_hashes: Vec<String>,
    supplemental_tx_hashes: Vec<String>,
    start_block: u64,
    end_block: u64,
    parent_header_hash: String,
    parent_block_number: u64,
    message_passer_storage_root: String,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args = parse_args()?;
    let fixture: FixtureInput = serde_json::from_str(&args.input_json)?;
    validate_fixture(&fixture)?;

    let traces = execute_fixture(&fixture, args.execution_mode)?;
    for trace in traces {
        println!("[trace]{}", trace);
    }

    Ok(())
}

fn execute_fixture(fixture: &FixtureInput, execution_mode: ExecutionMode) -> Result<Vec<Value>> {
    let batch_id = batch_id(fixture);

    if execution_mode.allow_fallback() {
        eprintln!(
            "warning: running fixture '{}' with fallback mode enabled; strict completeness guarantees are disabled",
            fixture.fixture_id
        );
    }

    let rollup_path = resolve_ref_path(&fixture.pre_checkpoint.rollup_config_ref);
    let rollup_config = load_rollup_config(&rollup_path)?;

    let bundle_path = resolve_ref_path(&fixture.pre_checkpoint.witness_bundle_ref);
    let bundle = load_witness_bundle(&bundle_path)?;
    let kv_refs = bundle
        .kv_store_refs
        .clone()
        .unwrap_or_else(|| vec![bundle.kv_store_ref.clone()]);
    let kv_paths = kv_refs
        .into_iter()
        .map(|value| resolve_ref_path(&value))
        .collect::<Vec<_>>();
    validate_witness_package(fixture, &bundle, &bundle_path, &kv_paths)?;

    let provider = DiskTrieNodeProvider::open_many(&kv_paths)?;
    let parent_header: Header = serde_json::from_value(bundle.parent_header)?;
    let eip_1559_params = bundle
        .source_payload_template
        .as_ref()
        .and_then(|template| template.eip_1559_params.as_deref())
        .map(parse_eip_1559_params)
        .transpose()?;

    let prev_randao = parse_b256("prev_randao", &fixture.pre_checkpoint.prev_randao)?;
    let expected_parent_header_hash = parse_b256(
        "parent_header_hash",
        &fixture.pre_checkpoint.parent_header_hash,
    )?;
    let parent_beacon_block_root = parse_b256(
        "parent_beacon_block_root",
        &fixture.pre_checkpoint.parent_beacon_block_root,
    )?;
    let message_passer_storage_root = parse_b256(
        "message_passer_storage_root",
        &fixture.output_root_witness.message_passer_storage_root,
    )?;
    let fee_recipient = parse_address("fee_recipient", &fixture.fee_recipient)?;

    if parent_header.parent_hash != expected_parent_header_hash {
        return Err(WorkloadError::InvalidFixture(
            "parent_header_hash does not match witness bundle parentHash".to_string(),
        ));
    }
    if parent_header.number != fixture.pre_checkpoint.parent_block_number {
        return Err(WorkloadError::InvalidFixture(
            "parent block number does not match witness bundle parent header".to_string(),
        ));
    }

    let mut builder = StatelessL2Builder::new(
        &rollup_config,
        OpEvmFactory::default(),
        provider,
        NoopTrieHinter,
        parent_header.clone().seal_slow(),
    );

    let payload = OpPayloadAttributes {
        payload_attributes: PayloadAttributes {
            timestamp: fixture.start_timestamp,
            prev_randao,
            suggested_fee_recipient: fee_recipient,
            withdrawals: None,
            parent_beacon_block_root: Some(parent_beacon_block_root),
        },
        gas_limit: Some(fixture.gas_limit),
        transactions: Some(
            fixture
                .execution_transactions()
                .into_iter()
                .map(decode_tx_bytes)
                .collect::<Result<Vec<_>>>()?,
        ),
        no_tx_pool: None,
        eip_1559_params,
    };

    let (next_output_root, output_root_status, block_hash, gas_used) =
        match builder.build_block(payload) {
            Ok(outcome) => (
                seeded_output_root(
                    outcome.header.state_root,
                    message_passer_storage_root,
                    outcome.header.hash(),
                ),
                "fixture_output_root",
                Some(format!("{:#x}", outcome.header.hash())),
                Some(outcome.execution_result.gas_used),
            ),
            Err(error) => {
                let reason = error.to_string();
                if let Some(class) = missing_witness_class(&reason) {
                    if execution_mode.allow_fallback() {
                        (
                            synthetic_next_output_root(
                                &fixture.pre_checkpoint.prev_output_root,
                                &fixture.batch_hash,
                                fixture.start_block,
                                fixture.start_timestamp,
                            ),
                            "synthetic_incomplete_witness",
                            None,
                            None,
                        )
                    } else {
                        return Err(WorkloadError::MissingWitness {
                            step_index: 0,
                            tx_id: batch_id.clone(),
                            class,
                            detail: reason,
                        });
                    }
                } else {
                    return Err(WorkloadError::KonaExecution {
                        tx_id: batch_id,
                        reason,
                    });
                }
            }
        };

    Ok(vec![json!({
        "exec_index": 0,
        "tile": "execute_l2_block",
        "tx_ids": fixture.transactions.iter().map(|tx| tx.id.as_str()).collect::<Vec<_>>(),
        "tx_hashes": fixture.transactions.iter().map(|tx| tx.hash.as_str()).collect::<Vec<_>>(),
        "tracked_tx_count": fixture.transactions.len(),
        "supplemental_tx_count": fixture.supplemental_transactions.len(),
        "execution_tx_count": fixture.execution_transactions().len(),
        "block_number": fixture.start_block,
        "parent_block_number": parent_header.number,
        "parent_header_hash": format!("{:#x}", parent_header.hash_slow()),
        "prev_output_root": fixture.pre_checkpoint.prev_output_root,
        "next_output_root": format!("{next_output_root:#x}"),
        "output_root_status": output_root_status,
        "batch_hash": fixture.batch_hash,
        "block_hash": block_hash,
        "gas_used": gas_used,
    })])
}

fn parse_args() -> Result<CliArgs> {
    let mut args = env::args().skip(1);
    let mut input_json: Option<String> = None;
    let mut execution_mode: Option<String> = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--input" => {
                input_json = Some(args.next().ok_or(WorkloadError::MissingInput)?);
            }
            "--execution-mode" => {
                execution_mode = args.next();
            }
            _ => {}
        }
    }

    Ok(CliArgs {
        input_json: input_json.ok_or(WorkloadError::MissingInput)?,
        execution_mode: ExecutionMode::from_cli(execution_mode)?,
    })
}

fn missing_witness_class(error: &str) -> Option<&'static str> {
    if error.contains("missing trie preimage") {
        Some("trie-node")
    } else if error.contains("missing bytecode preimage") {
        Some("bytecode")
    } else {
        None
    }
}

fn validate_fixture(fixture: &FixtureInput) -> Result<()> {
    if fixture.transactions.len() != 5 {
        return Err(WorkloadError::InvalidFixture(
            "expected exactly 5 transactions".to_string(),
        ));
    }

    if fixture.end_block != fixture.start_block {
        return Err(WorkloadError::InvalidFixture(
            "expected a single block transition with start_block == end_block".to_string(),
        ));
    }

    if fixture.start_block != fixture.pre_checkpoint.parent_block_number + 1 {
        return Err(WorkloadError::InvalidFixture(
            "start_block must advance exactly one block beyond the parent checkpoint".to_string(),
        ));
    }

    if fixture.timestamp_delta_seconds != 0 {
        return Err(WorkloadError::InvalidFixture(
            "timestamp_delta_seconds must be 0 for single-block batch execution".to_string(),
        ));
    }

    for tx in fixture.execution_transactions() {
        let tx_bytes = decode_hex(&tx.raw).ok_or_else(|| WorkloadError::InvalidTxHex {
            tx_id: tx.id.clone(),
        })?;
        let tx_hash = format!("{:#x}", keccak256(tx_bytes));
        if tx_hash != tx.hash {
            return Err(WorkloadError::InvalidFixture(format!(
                "tx hash mismatch for {}",
                tx.id
            )));
        }
    }

    let mut batch_bytes = Vec::new();
    for tx in &fixture.transactions {
        let tx_bytes = decode_hex(&tx.raw).ok_or_else(|| WorkloadError::InvalidTxHex {
            tx_id: tx.id.clone(),
        })?;
        batch_bytes.extend_from_slice(&tx_bytes);
    }

    let batch_hash = format!("{:#x}", keccak256(batch_bytes));
    if batch_hash != fixture.batch_hash {
        return Err(WorkloadError::InvalidFixture(
            "batch hash mismatch".to_string(),
        ));
    }

    if fixture.supplemental_transactions.is_empty() {
        return Err(WorkloadError::InvalidFixture(
            "expected supplemental block transactions for witness-complete execution".to_string(),
        ));
    }

    Ok(())
}

fn validate_witness_package(
    fixture: &FixtureInput,
    bundle: &WitnessBundle,
    bundle_path: &Path,
    kv_paths: &[PathBuf],
) -> Result<()> {
    let manifest_ref = bundle.closure_manifest_ref.as_deref().ok_or_else(|| {
        WorkloadError::InvalidFixture(
            "witness bundle missing closure_manifest_ref for strict package validation".to_string(),
        )
    })?;
    let manifest_path = resolve_ref_path(manifest_ref);
    ensure_fixture_path(&manifest_path, "closure manifest")?;

    for path in kv_paths {
        ensure_fixture_path(path, "witness kv store")?;
    }

    let manifest = load_witness_closure_manifest(&manifest_path)?;
    validate_witness_closure_manifest(fixture, bundle, &manifest, bundle_path)?;
    Ok(())
}

fn validate_witness_closure_manifest(
    fixture: &FixtureInput,
    bundle: &WitnessBundle,
    manifest: &WitnessClosureManifest,
    bundle_path: &Path,
) -> Result<()> {
    let expected_bundle_ref = normalized_ref(bundle_path);
    if manifest.witness_bundle_ref != expected_bundle_ref {
        return Err(WorkloadError::InvalidFixture(format!(
            "closure manifest witness bundle ref mismatch: expected {}, got {}",
            expected_bundle_ref, manifest.witness_bundle_ref
        )));
    }

    if manifest.fixture_ref != CANONICAL_FIXTURE_PATH && fixture.fixture_id == "l2-poc-synth-v1" {
        return Err(WorkloadError::InvalidFixture(format!(
            "closure manifest fixture ref mismatch for canonical synthetic fixture: expected {}, got {}",
            CANONICAL_FIXTURE_PATH, manifest.fixture_ref
        )));
    }

    if manifest.rollup_config_ref != fixture.pre_checkpoint.rollup_config_ref {
        return Err(WorkloadError::InvalidFixture(
            "closure manifest rollup_config_ref mismatch".to_string(),
        ));
    }

    let manifest_kv_refs = sorted_refs(manifest.kv_store_refs.clone());
    let bundle_kv_refs = sorted_refs(bundle_kv_refs(bundle));
    if manifest_kv_refs != bundle_kv_refs {
        return Err(WorkloadError::InvalidFixture(
            "closure manifest kv_store_refs do not match witness bundle".to_string(),
        ));
    }

    let identity = &manifest.identity;
    if identity.fixture_id != fixture.fixture_id {
        return Err(WorkloadError::InvalidFixture(
            "closure manifest fixture_id mismatch".to_string(),
        ));
    }
    if identity.batch_hash != fixture.batch_hash {
        return Err(WorkloadError::InvalidFixture(
            "closure manifest batch_hash mismatch".to_string(),
        ));
    }
    if identity.tx_hashes
        != fixture
            .transactions
            .iter()
            .map(|tx| tx.hash.clone())
            .collect::<Vec<_>>()
    {
        return Err(WorkloadError::InvalidFixture(
            "closure manifest tracked tx hashes mismatch".to_string(),
        ));
    }
    if identity.supplemental_tx_hashes
        != fixture
            .supplemental_transactions
            .iter()
            .map(|tx| tx.hash.clone())
            .collect::<Vec<_>>()
    {
        return Err(WorkloadError::InvalidFixture(
            "closure manifest supplemental tx hashes mismatch".to_string(),
        ));
    }
    if identity.start_block != fixture.start_block || identity.end_block != fixture.end_block {
        return Err(WorkloadError::InvalidFixture(
            "closure manifest block window mismatch".to_string(),
        ));
    }
    if identity.parent_header_hash != fixture.pre_checkpoint.parent_header_hash {
        return Err(WorkloadError::InvalidFixture(
            "closure manifest parent_header_hash mismatch".to_string(),
        ));
    }
    if identity.parent_block_number != fixture.pre_checkpoint.parent_block_number {
        return Err(WorkloadError::InvalidFixture(
            "closure manifest parent_block_number mismatch".to_string(),
        ));
    }
    if identity.message_passer_storage_root
        != fixture.output_root_witness.message_passer_storage_root
    {
        return Err(WorkloadError::InvalidFixture(
            "closure manifest message_passer_storage_root mismatch".to_string(),
        ));
    }

    Ok(())
}

impl FixtureInput {
    fn execution_transactions(&self) -> Vec<&FixtureTransaction> {
        self.transactions
            .iter()
            .chain(self.supplemental_transactions.iter())
            .collect()
    }
}

fn resolve_ref_path(reference: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("..");
    root.join(reference)
}

fn load_rollup_config(path: &Path) -> Result<RollupConfig> {
    let raw = std::fs::read_to_string(path).map_err(|source| WorkloadError::Io {
        path: path.display().to_string(),
        source,
    })?;
    serde_json::from_str(&raw).map_err(WorkloadError::from)
}

fn load_witness_bundle(path: &Path) -> Result<WitnessBundle> {
    let raw = std::fs::read_to_string(path).map_err(|source| WorkloadError::Io {
        path: path.display().to_string(),
        source,
    })?;
    serde_json::from_str(&raw).map_err(WorkloadError::from)
}

fn load_witness_closure_manifest(path: &Path) -> Result<WitnessClosureManifest> {
    let raw = std::fs::read_to_string(path).map_err(|source| WorkloadError::Io {
        path: path.display().to_string(),
        source,
    })?;
    serde_json::from_str(&raw).map_err(WorkloadError::from)
}

fn ensure_fixture_path(path: &Path, label: &str) -> Result<()> {
    if path.exists() {
        Ok(())
    } else {
        Err(WorkloadError::InvalidFixture(format!(
            "missing {label} at {}",
            path.display()
        )))
    }
}

fn normalized_ref(path: &Path) -> String {
    let root = resolve_ref_path("");
    path.strip_prefix(&root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn bundle_kv_refs(bundle: &WitnessBundle) -> Vec<String> {
    bundle
        .kv_store_refs
        .clone()
        .unwrap_or_else(|| vec![bundle.kv_store_ref.clone()])
}

fn sorted_refs(mut refs: Vec<String>) -> Vec<String> {
    refs.sort();
    refs
}

fn parse_b256(field: &'static str, value: &str) -> Result<B256> {
    value.parse().map_err(|_| WorkloadError::InvalidHash {
        field,
        value: value.to_string(),
    })
}

fn parse_address(field: &'static str, value: &str) -> Result<Address> {
    value.parse().map_err(|_| WorkloadError::InvalidAddress {
        field,
        value: value.to_string(),
    })
}

fn decode_tx_bytes(tx: &FixtureTransaction) -> Result<Bytes> {
    let bytes = decode_hex(&tx.raw).ok_or_else(|| WorkloadError::InvalidTxHex {
        tx_id: tx.id.clone(),
    })?;
    Ok(bytes.into())
}

fn decode_hex(value: &str) -> Option<Vec<u8>> {
    let raw = value.strip_prefix("0x").unwrap_or(value);
    if !raw.len().is_multiple_of(2) {
        return None;
    }

    let mut output = Vec::with_capacity(raw.len() / 2);
    let bytes = raw.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let high = decode_nibble(bytes[i])?;
        let low = decode_nibble(bytes[i + 1])?;
        output.push((high << 4) | low);
        i += 2;
    }
    Some(output)
}

fn decode_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn parse_eip_1559_params(value: &str) -> Result<FixedBytes<8>> {
    let bytes = decode_hex(value).ok_or_else(|| {
        WorkloadError::InvalidFixture("invalid eip1559Params in witness bundle".to_string())
    })?;
    let array: [u8; 8] = bytes.try_into().map_err(|_| {
        WorkloadError::InvalidFixture("eip1559Params must be exactly 8 bytes".to_string())
    })?;
    Ok(FixedBytes::from(array))
}

fn seeded_output_root(
    state_root: B256,
    message_passer_storage_root: B256,
    block_hash: B256,
) -> B256 {
    let mut encoded = [0u8; 128];
    encoded[32..64].copy_from_slice(state_root.as_slice());
    encoded[64..96].copy_from_slice(message_passer_storage_root.as_slice());
    encoded[96..128].copy_from_slice(block_hash.as_slice());
    keccak256(encoded)
}

fn synthetic_next_output_root(
    prev_output_root: &str,
    batch_hash: &str,
    block_number: u64,
    timestamp: u64,
) -> B256 {
    let mut material = Vec::new();
    material.extend_from_slice(prev_output_root.as_bytes());
    material.extend_from_slice(batch_hash.as_bytes());
    material.extend_from_slice(block_number.to_string().as_bytes());
    material.extend_from_slice(timestamp.to_string().as_bytes());
    keccak256(material)
}

fn batch_id(fixture: &FixtureInput) -> String {
    let ids = fixture
        .transactions
        .iter()
        .map(|tx| tx.id.as_str())
        .collect::<Vec<_>>()
        .join(",");
    format!("batch[{ids}]")
}

#[derive(Debug, Clone)]
struct DiskTrieNodeProvider {
    dbs: Vec<Arc<DB>>,
}

impl DiskTrieNodeProvider {
    fn open_many(paths: &[PathBuf]) -> Result<Self> {
        let mut options = rocksdb::Options::default();
        options.create_if_missing(false);
        let mut dbs = Vec::new();
        for path in paths {
            let db = DB::open_for_read_only(&options, path, false).map_err(|source| {
                WorkloadError::RocksDb {
                    path: path.display().to_string(),
                    source,
                }
            })?;
            dbs.push(Arc::new(db));
        }
        Ok(Self { dbs })
    }

    fn get_preimage(&self, hash: B256) -> Option<Vec<u8>> {
        self.dbs
            .iter()
            .find_map(|db| db.get(hash).ok().and_then(|value| value))
    }
}

impl TrieProvider for DiskTrieNodeProvider {
    type Error = WorkloadError;

    fn trie_node_by_hash(&self, hash: B256) -> std::result::Result<TrieNode, Self::Error> {
        let preimage = self
            .get_preimage(hash)
            .ok_or(WorkloadError::MissingPreimage { hash })?;

        TrieNode::decode(&mut preimage.as_slice())
            .map_err(|source| WorkloadError::TrieNodeDecode { hash, source })
    }
}

impl TrieDBProvider for DiskTrieNodeProvider {
    fn bytecode_by_hash(&self, hash: B256) -> std::result::Result<Bytes, Self::Error> {
        self.get_preimage(hash)
            .map(Bytes::from)
            .ok_or(WorkloadError::MissingBytecode { hash })
    }

    fn header_by_hash(&self, hash: B256) -> std::result::Result<Header, Self::Error> {
        let preimage = self
            .get_preimage(hash)
            .ok_or(WorkloadError::MissingPreimage { hash })?;
        Header::decode(&mut preimage.as_slice())
            .map_err(|source| WorkloadError::HeaderDecode { hash, source })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canonical_fixture() -> FixtureInput {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("..")
            .join("runs")
            .join("fixtures")
            .join("l2-poc-synth-fixture.json");
        let raw = std::fs::read_to_string(path).expect("failed to read canonical fixture");
        serde_json::from_str(&raw).expect("failed to parse canonical fixture")
    }

    #[test]
    fn strict_canonical_fixture_executes_full_batch_without_missing_witness() {
        let fixture = canonical_fixture();
        let traces = execute_fixture(&fixture, ExecutionMode::Strict)
            .expect("strict canonical fixture should execute without missing witness data");

        assert_eq!(traces.len(), 1);
        let trace = &traces[0];
        assert_eq!(trace["output_root_status"], "fixture_output_root");
        assert_eq!(trace["tracked_tx_count"], 5);
        assert_eq!(trace["execution_tx_count"], 10);
        assert_eq!(trace["block_number"], fixture.start_block);
    }

    #[test]
    fn strict_canonical_fixture_is_deterministic_across_repeated_runs() {
        let fixture = canonical_fixture();
        let run_one = execute_fixture(&fixture, ExecutionMode::Strict)
            .expect("first strict canonical execution should succeed");
        let run_two = execute_fixture(&fixture, ExecutionMode::Strict)
            .expect("second strict canonical execution should succeed");

        assert_eq!(run_one, run_two);
    }
}
