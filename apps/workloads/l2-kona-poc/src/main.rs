extern crate alloc;

mod chunk_driver;
mod chunk_plan;

use std::env;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use alloy_consensus::{Header, Sealable};
use alloy_op_evm::OpEvmFactory;
use alloy_primitives::{keccak256, Address, Bytes, FixedBytes, B256};
use alloy_rlp::Decodable;
use alloy_rpc_types_engine::PayloadAttributes;
use kona_executor::{StatelessL2Builder, TrieDBProvider};
use kona_genesis::RollupConfig;
use kona_mpt::{NoopTrieHinter, TrieNode, TrieProvider};
use op_alloy_rpc_types_engine::OpPayloadAttributes;
use raster::{sequence, tile};
use rocksdb::DB;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::chunk_driver::{ChunkDriver, ChunkExecutionCheckpoint, ChunkFinalizedResult};
use crate::chunk_plan::{build_chunk_plan, DEFAULT_CHUNK_SIZE};

use std::cell::RefCell;

const CANONICAL_FIXTURE_PATH: &str = "runs/fixtures/l2-poc-synth-fixture.json";
const CANONICAL_RUNTIME_TILE_COUNT: usize = 10;

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
enum WorkloadError {
    #[error("missing --input '<fixture-json>'")]
    MissingInput,
    #[error("invalid --execution-mode '{value}' (expected 'strict' or 'fallback')")]
    InvalidExecutionMode { value: String },
    #[error("invalid --chunk-size '{value}' (expected a positive integer)")]
    InvalidChunkSizeValue { value: String },
    #[error("fixture parse failed: {0}")]
    FixtureParse(#[from] serde_json::Error),
    #[error("invalid fixture: {0}")]
    InvalidFixture(String),
    #[error("chunk size must be greater than 0")]
    InvalidChunkSize,
    #[error("invalid chunk resume: {0}")]
    InvalidChunkResume(String),
    #[error("explicit multi-tile runtime requires canonical chunk_size = 1 (got {chunk_size})")]
    UnsupportedRuntimeChunkSize { chunk_size: usize },
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

// ---------------------------------------------------------------------------
// Fixture and checkpoint types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OutputRootWitness {
    message_passer_storage_root: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PreCheckpoint {
    prev_output_root: String,
    parent_header_hash: String,
    parent_block_number: u64,
    rollup_config_ref: String,
    witness_bundle_ref: String,
    prev_randao: String,
    parent_beacon_block_root: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

// ---------------------------------------------------------------------------
// Raster tile I/O types — lightweight and postcard-serializable
// ---------------------------------------------------------------------------

/// Output of a single chunk tile execution.
///
/// Every tile produces a `TileOutput`. Non-final tiles leave the finalization
/// fields empty; the sealing tile populates them.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct TileOutput {
    tile_index: usize,
    tx_ids: Vec<String>,
    seals_block: bool,
    /// Populated only on the sealing tile.
    state_root: Option<String>,
    next_output_root: Option<String>,
    output_root_status: Option<String>,
    gas_used: Option<u64>,
    block_hash: Option<String>,
}

// ---------------------------------------------------------------------------
// Execution mode
// ---------------------------------------------------------------------------

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
    emit_chunk_plan: bool,
    chunk_size: usize,
}

// ---------------------------------------------------------------------------
// Internal execution context (not passed through tile boundaries)
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct LoadedExecutionContext {
    rollup_config: RollupConfig,
    provider: DiskTrieNodeProvider,
    parent_header: alloy_consensus::Sealed<Header>,
    eip_1559_params: Option<FixedBytes<8>>,
    prev_randao: B256,
    parent_beacon_block_root: B256,
    message_passer_storage_root: B256,
    fee_recipient: Address,
}

struct ReferenceExecutionOutcome {
    state_root: B256,
    next_output_root: B256,
    output_root_status: &'static str,
    block_hash: Option<String>,
    gas_used: Option<u64>,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

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

    // --emit-chunk-plan is a utility mode, not a Raster program execution.
    if args.emit_chunk_plan {
        let plan = build_chunk_plan(&fixture, args.chunk_size)?;
        println!(
            "{}",
            serde_json::to_string_pretty(&plan).map_err(WorkloadError::FixtureParse)?
        );
        return Ok(());
    }

    // Fallback mode preserves the legacy whole-block path without Raster tracing.
    if args.execution_mode.allow_fallback() {
        eprintln!(
            "warning: running fixture '{}' with fallback mode enabled; strict completeness guarantees are disabled",
            fixture.fixture_id
        );
        let trace = execute_fallback_block(&fixture, args.execution_mode)?;
        println!("[trace]{}", trace);
        return Ok(());
    }

    // Strict mode: run the real Raster program with tile-level tracing.
    ensure_canonical_runtime_shape(&fixture, args.chunk_size)?;

    raster::init();
    let result = l2_block_execution(fixture);
    raster::finish();

    // The final tile must be the sealing tile with all finalization fields set.
    assert!(result.seals_block, "final tile must seal the block");

    // Emit a machine-readable summary line for the check script and runner.
    // This is NOT a trace record — it's a final program output summary.
    let summary = serde_json::json!({
        "next_output_root": result.next_output_root.as_deref().expect("sealing tile must set next_output_root"),
        "output_root_status": result.output_root_status.as_deref().expect("sealing tile must set output_root_status"),
        "state_root": result.state_root.as_deref().expect("sealing tile must set state_root"),
        "gas_used": result.gas_used.expect("sealing tile must set gas_used"),
        "block_hash": result.block_hash,
        "tile_count": CANONICAL_RUNTIME_TILE_COUNT,
        "tracked_tx_count": 5,
        "supplemental_tx_count": 5,
        "execution_tx_count": 10,
    });
    println!("[summary]{}", summary);

    Ok(())
}

// ---------------------------------------------------------------------------
// Raster program: tile and sequence definitions
//
// This is the canonical Raster program shape. The #[tile] attribute auto-
// instruments each tile function with Raster runtime tracing (serialized
// input/output via emit_trace). The #[sequence] attribute registers the
// tile call graph for CFS extraction.
//
// Traces are emitted automatically by the Raster runtime — no manual
// [trace] printing is needed.
// ---------------------------------------------------------------------------

/// Shared execution state for the current block execution.
///
/// In native execution mode, a single ChunkDriver persists across all tiles
/// within one `l2_block_execution` sequence call. The TrieDB and cumulative
/// EVM state are carried forward in-process so later tiles can resume from
/// where prior tiles left off without replaying from the parent checkpoint.
///
/// This shared state is an optimization for the native execution path. In a
/// real zkVM execution, each tile would be an independent proving unit with
/// its own witness data.
struct SharedExecutionState {
    driver: ChunkDriver,
    checkpoint: ChunkExecutionCheckpoint,
    context: LoadedExecutionContext,
    payload: OpPayloadAttributes,
}

thread_local! {
    static EXECUTION_STATE: RefCell<Option<SharedExecutionState>> = const { RefCell::new(None) };
}

fn init_shared_execution(fixture: &FixtureInput) -> Result<()> {
    let context = load_execution_context(fixture)?;
    let payload = build_payload(fixture, &context)?;
    let driver = ChunkDriver::new(
        context.rollup_config.clone(),
        payload.clone(),
        context.parent_header.clone(),
        context.provider.clone(),
        fixture.pre_checkpoint.witness_bundle_ref.clone(),
    )?;
    let checkpoint = driver.initial_checkpoint();
    EXECUTION_STATE.with(|cell| {
        *cell.borrow_mut() = Some(SharedExecutionState {
            driver,
            checkpoint,
            context,
            payload,
        });
    });
    Ok(())
}

fn take_shared_execution() -> Option<SharedExecutionState> {
    EXECUTION_STATE.with(|cell| cell.borrow_mut().take())
}

/// Top-level sequence: execute one L2 block as 10 deterministic chunk tiles.
///
/// Each tile executes exactly one transaction from the canonical batch.
/// Tiles 0–4 cover tracked txs; tiles 5–9 cover supplemental txs.
/// Tile 9 seals the block and yields the final output root.
///
/// The same `execute_chunk` tile function is called for every tile — it's
/// one ELF binary that handles any tile index. The tile index parameter
/// selects which transaction slice to execute.
#[sequence]
fn l2_block_execution(fixture: FixtureInput) -> TileOutput {
    // Initialize shared execution state for this block.
    init_shared_execution(&fixture).expect("failed to initialize execution context");
    let c0 = execute_chunk(fixture.clone(), 0usize);
    let c1 = execute_chunk(fixture.clone(), c0.tile_index + 1);
    let c2 = execute_chunk(fixture.clone(), c1.tile_index + 1);
    let c3 = execute_chunk(fixture.clone(), c2.tile_index + 1);
    let c4 = execute_chunk(fixture.clone(), c3.tile_index + 1);
    let c5 = execute_chunk(fixture.clone(), c4.tile_index + 1);
    let c6 = execute_chunk(fixture.clone(), c5.tile_index + 1);
    let c7 = execute_chunk(fixture.clone(), c6.tile_index + 1);
    let c8 = execute_chunk(fixture.clone(), c7.tile_index + 1);
    execute_chunk(fixture, c8.tile_index + 1)
}

/// Execute one chunk tile by index.
///
/// This is the single tile function for the entire L2 block execution program.
/// It handles both non-final tiles (which advance the execution cursor) and
/// the final sealing tile (which also runs the reference comparison and
/// produces the output root).
///
/// In a zkVM context, this compiles to one ELF binary parameterized by
/// `tile_index`. The prover invokes it N times with different indices.
#[tile(kind = iter)]
fn execute_chunk(fixture: FixtureInput, tile_index: usize) -> TileOutput {
    execute_tile_shared(&fixture, tile_index)
        .unwrap_or_else(|e| panic!("tile {tile_index} execution failed: {e}"))
}

// ---------------------------------------------------------------------------
// Tile implementation (internal — the #[tile] wrapper above is the traced
// entry point)
// ---------------------------------------------------------------------------

fn execute_tile_shared(fixture: &FixtureInput, tile_index: usize) -> Result<TileOutput> {
    let chunk_plan = build_chunk_plan(fixture, DEFAULT_CHUNK_SIZE)?;
    let tile = chunk_plan.tile(tile_index).ok_or_else(|| {
        WorkloadError::InvalidChunkResume(format!("missing canonical tile {tile_index}"))
    })?;

    if !tile.seals_block() {
        // --- Non-final tile: execute one tx slice, advance checkpoint ---
        EXECUTION_STATE.with(|cell| {
            let mut state = cell.borrow_mut();
            let exec = state
                .as_mut()
                .expect("shared execution state not initialized");

            match exec.driver.execute_tile(
                std::mem::replace(&mut exec.checkpoint, exec.driver.initial_checkpoint()),
                tile,
            )? {
                chunk_driver::ChunkTileExecutionOutcome::Checkpointed(next_checkpoint) => {
                    exec.checkpoint = *next_checkpoint;
                    Ok(TileOutput {
                        tile_index,
                        tx_ids: tile.tx_ids().to_vec(),
                        seals_block: false,
                        state_root: None,
                        next_output_root: None,
                        output_root_status: None,
                        gas_used: None,
                        block_hash: None,
                    })
                }
                chunk_driver::ChunkTileExecutionOutcome::Finalized(_) => {
                    Err(WorkloadError::InvalidChunkResume(format!(
                        "tile {tile_index} finalized unexpectedly"
                    )))
                }
            }
        })
    } else {
        // --- Sealing tile: execute final tx slice, verify against reference ---
        let batch_id = batch_id(fixture);
        let exec = take_shared_execution().expect("shared execution state not initialized");

        let finalized = match exec.driver.execute_tile(exec.checkpoint, tile)? {
            chunk_driver::ChunkTileExecutionOutcome::Finalized(finalized) => finalized,
            chunk_driver::ChunkTileExecutionOutcome::Checkpointed(_) => {
                return Err(WorkloadError::InvalidChunkResume(format!(
                    "tile {tile_index} checkpointed unexpectedly"
                )));
            }
        };

        let reference = execute_reference_block(
            fixture,
            &exec.context,
            exec.payload,
            ExecutionMode::Strict,
            &batch_id,
        )?;
        ensure_finalized_chunk_matches_reference(&finalized, &reference)?;

        Ok(TileOutput {
            tile_index,
            tx_ids: tile.tx_ids().to_vec(),
            seals_block: true,
            state_root: Some(finalized.final_state_root),
            next_output_root: Some(format!("{:#x}", reference.next_output_root)),
            output_root_status: Some(reference.output_root_status.to_string()),
            gas_used: Some(finalized.final_gas_used),
            block_hash: reference.block_hash,
        })
    }
}

// ---------------------------------------------------------------------------
// Shared execution helpers (used by both tile path and fallback path)
// ---------------------------------------------------------------------------

fn ensure_canonical_runtime_shape(fixture: &FixtureInput, chunk_size: usize) -> Result<()> {
    if chunk_size != DEFAULT_CHUNK_SIZE {
        return Err(WorkloadError::UnsupportedRuntimeChunkSize { chunk_size });
    }

    let chunk_plan = build_chunk_plan(fixture, chunk_size)?;
    if chunk_plan.tiles().len() != CANONICAL_RUNTIME_TILE_COUNT {
        return Err(WorkloadError::InvalidFixture(format!(
            "explicit multi-tile runtime expects exactly {CANONICAL_RUNTIME_TILE_COUNT} canonical tiles"
        )));
    }

    Ok(())
}

fn ensure_finalized_chunk_matches_reference(
    finalized: &ChunkFinalizedResult,
    reference: &ReferenceExecutionOutcome,
) -> Result<()> {
    if reference.output_root_status != "fixture_output_root" {
        return Ok(());
    }

    if finalized.final_state_root != format!("{:#x}", reference.state_root) {
        return Err(WorkloadError::InvalidChunkResume(format!(
            "final tile state root {} did not match reference {:#x}",
            finalized.final_state_root, reference.state_root
        )));
    }

    if Some(finalized.final_gas_used) != reference.gas_used {
        return Err(WorkloadError::InvalidChunkResume(format!(
            "final tile gas_used {} did not match reference {:?}",
            finalized.final_gas_used, reference.gas_used
        )));
    }

    Ok(())
}

fn execute_fallback_block(fixture: &FixtureInput, execution_mode: ExecutionMode) -> Result<Value> {
    let batch_id = batch_id(fixture);
    let context = load_execution_context(fixture)?;
    let payload = build_payload(fixture, &context)?;
    let reference = execute_reference_block(fixture, &context, payload, execution_mode, &batch_id)?;
    Ok(build_legacy_trace_record(
        fixture,
        &context.parent_header,
        &reference,
    ))
}

fn build_legacy_trace_record(
    fixture: &FixtureInput,
    parent_header: &alloy_consensus::Sealed<Header>,
    outcome: &ReferenceExecutionOutcome,
) -> Value {
    serde_json::json!({
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
        "next_output_root": format!("{:#x}", outcome.next_output_root),
        "output_root_status": outcome.output_root_status,
        "batch_hash": fixture.batch_hash,
        "block_hash": outcome.block_hash,
        "gas_used": outcome.gas_used,
    })
}

// ---------------------------------------------------------------------------
// Kona execution engine
// ---------------------------------------------------------------------------

fn load_execution_context(fixture: &FixtureInput) -> Result<LoadedExecutionContext> {
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

    Ok(LoadedExecutionContext {
        rollup_config,
        provider,
        parent_header: parent_header.seal_slow(),
        eip_1559_params,
        prev_randao,
        parent_beacon_block_root,
        message_passer_storage_root,
        fee_recipient,
    })
}

fn build_payload(
    fixture: &FixtureInput,
    context: &LoadedExecutionContext,
) -> Result<OpPayloadAttributes> {
    Ok(OpPayloadAttributes {
        payload_attributes: PayloadAttributes {
            timestamp: fixture.start_timestamp,
            prev_randao: context.prev_randao,
            suggested_fee_recipient: context.fee_recipient,
            withdrawals: None,
            parent_beacon_block_root: Some(context.parent_beacon_block_root),
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
        eip_1559_params: context.eip_1559_params,
    })
}

fn execute_reference_block(
    fixture: &FixtureInput,
    context: &LoadedExecutionContext,
    payload: OpPayloadAttributes,
    execution_mode: ExecutionMode,
    batch_id: &str,
) -> Result<ReferenceExecutionOutcome> {
    let mut builder = StatelessL2Builder::new(
        &context.rollup_config,
        OpEvmFactory::default(),
        context.provider.clone(),
        NoopTrieHinter,
        context.parent_header.clone(),
    );

    match builder.build_block(payload) {
        Ok(outcome) => Ok(ReferenceExecutionOutcome {
            state_root: outcome.header.state_root,
            next_output_root: seeded_output_root(
                outcome.header.state_root,
                context.message_passer_storage_root,
                outcome.header.hash(),
            ),
            output_root_status: "fixture_output_root",
            block_hash: Some(format!("{:#x}", outcome.header.hash())),
            gas_used: Some(outcome.execution_result.gas_used),
        }),
        Err(error) => {
            let reason = error.to_string();
            if let Some(class) = missing_witness_class(&reason) {
                if execution_mode.allow_fallback() {
                    Ok(ReferenceExecutionOutcome {
                        state_root: context.parent_header.state_root,
                        next_output_root: synthetic_next_output_root(
                            &fixture.pre_checkpoint.prev_output_root,
                            &fixture.batch_hash,
                            fixture.start_block,
                            fixture.start_timestamp,
                        ),
                        output_root_status: "synthetic_incomplete_witness",
                        block_hash: None,
                        gas_used: None,
                    })
                } else {
                    Err(WorkloadError::MissingWitness {
                        step_index: 0,
                        tx_id: batch_id.to_string(),
                        class,
                        detail: reason,
                    })
                }
            } else {
                Err(WorkloadError::KonaExecution {
                    tx_id: batch_id.to_string(),
                    reason,
                })
            }
        }
    }
}

// ---------------------------------------------------------------------------
// CLI argument parsing
// ---------------------------------------------------------------------------

fn parse_args() -> Result<CliArgs> {
    let mut args = env::args().skip(1);
    let mut input_json: Option<String> = None;
    let mut execution_mode: Option<String> = None;
    let mut emit_chunk_plan = false;
    let mut chunk_size = DEFAULT_CHUNK_SIZE;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--input" => {
                input_json = Some(args.next().ok_or(WorkloadError::MissingInput)?);
            }
            "--execution-mode" => {
                execution_mode = args.next();
            }
            "--emit-chunk-plan" => {
                emit_chunk_plan = true;
            }
            "--chunk-size" => {
                let value = args
                    .next()
                    .ok_or_else(|| WorkloadError::InvalidChunkSizeValue {
                        value: "<missing>".to_string(),
                    })?;
                chunk_size = value
                    .parse()
                    .map_err(|_| WorkloadError::InvalidChunkSizeValue {
                        value: value.clone(),
                    })?;
            }
            _ => {}
        }
    }

    if chunk_size == 0 {
        return Err(WorkloadError::InvalidChunkSize);
    }

    Ok(CliArgs {
        input_json: input_json.ok_or(WorkloadError::MissingInput)?,
        execution_mode: ExecutionMode::from_cli(execution_mode)?,
        emit_chunk_plan,
        chunk_size,
    })
}

// ---------------------------------------------------------------------------
// Fixture validation
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Utility functions
// ---------------------------------------------------------------------------

impl FixtureInput {
    fn execution_transactions(&self) -> Vec<&FixtureTransaction> {
        self.transactions
            .iter()
            .chain(self.supplemental_transactions.iter())
            .collect()
    }
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

// ---------------------------------------------------------------------------
// Disk-backed trie node provider (RocksDB)
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

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
    fn strict_canonical_fixture_executes_all_tiles() {
        let fixture = canonical_fixture();
        ensure_canonical_runtime_shape(&fixture, DEFAULT_CHUNK_SIZE)
            .expect("canonical runtime shape should be valid");

        let result = l2_block_execution(fixture);

        assert_eq!(result.tile_index, 9);
        assert!(result.seals_block);
        assert_eq!(
            result.output_root_status.as_deref(),
            Some("fixture_output_root")
        );
        assert!(result.gas_used.unwrap_or(0) > 0);
        assert!(result.block_hash.is_some());
    }

    #[test]
    fn strict_canonical_fixture_is_deterministic_across_repeated_runs() {
        let fixture = canonical_fixture();

        let run_one = l2_block_execution(fixture.clone());
        let run_two = l2_block_execution(fixture);

        assert_eq!(run_one.next_output_root, run_two.next_output_root);
        assert_eq!(run_one.state_root, run_two.state_root);
        assert_eq!(run_one.gas_used, run_two.gas_used);
        assert_eq!(run_one.block_hash, run_two.block_hash);
    }

    #[test]
    fn strict_runtime_rejects_noncanonical_chunk_size() {
        let fixture = canonical_fixture();
        let error = ensure_canonical_runtime_shape(&fixture, 2)
            .expect_err("strict runtime should reject non-canonical chunk sizes");

        assert!(matches!(
            error,
            WorkloadError::UnsupportedRuntimeChunkSize { chunk_size: 2 }
        ));
    }

    #[test]
    fn canonical_chunk_plan_defaults_to_one_tx_tiles() {
        let fixture = canonical_fixture();
        let plan = build_chunk_plan(&fixture, DEFAULT_CHUNK_SIZE)
            .expect("chunk planning should succeed for canonical fixture");
        let plan_json = serde_json::to_value(plan).expect("chunk plan should serialize");

        assert_eq!(plan_json["chunking_policy"]["kind"], "fixed-tx-count");
        assert_eq!(plan_json["chunking_policy"]["chunk_size"], 1);
        assert_eq!(plan_json["chunking_policy"]["execution_tx_count"], 10);
        let tiles = plan_json["tiles"]
            .as_array()
            .expect("tiles should be an array");
        assert_eq!(tiles.len(), 10);
        assert_eq!(tiles[0]["tx_ids"], serde_json::json!(["alice_to_bob_1"]));
        assert_eq!(tiles[4]["tx_ids"], serde_json::json!(["alice_to_bob_2"]));
        assert_eq!(tiles[5]["tx_ids"], serde_json::json!(["supporting_tx_1"]));
        assert_eq!(tiles[9]["seals_block"], true);
        assert_eq!(
            tiles[9]["output_checkpoint"]["checkpoint_kind"],
            "sealed_block_checkpoint"
        );
    }

    #[test]
    fn chunk_plan_is_deterministic_for_same_fixture() {
        let fixture = canonical_fixture();
        let plan_one = build_chunk_plan(&fixture, DEFAULT_CHUNK_SIZE)
            .expect("first chunk plan should succeed");
        let plan_two = build_chunk_plan(&fixture, DEFAULT_CHUNK_SIZE)
            .expect("second chunk plan should succeed");

        let json_one = serde_json::to_value(plan_one).expect("first chunk plan should serialize");
        let json_two = serde_json::to_value(plan_two).expect("second chunk plan should serialize");
        assert_eq!(json_one, json_two);
    }

    #[test]
    fn chunk_driver_matches_reference_state_root_and_gas() {
        let fixture = canonical_fixture();
        let context = load_execution_context(&fixture).expect("execution context should load");
        let payload = build_payload(&fixture, &context).expect("payload should build");
        let plan = build_chunk_plan(&fixture, DEFAULT_CHUNK_SIZE).expect("chunk plan should build");
        let driver = ChunkDriver::new(
            context.rollup_config.clone(),
            payload.clone(),
            context.parent_header.clone(),
            context.provider.clone(),
            fixture.pre_checkpoint.witness_bundle_ref.clone(),
        )
        .expect("chunk driver should build");
        let chunk_result = driver
            .execute_plan(&plan)
            .expect("chunked execution should succeed");
        let reference = execute_reference_block(
            &fixture,
            &context,
            payload,
            ExecutionMode::Strict,
            &batch_id(&fixture),
        )
        .expect("reference execution should succeed");

        assert_eq!(
            chunk_result.final_state_root,
            format!("{:#x}", reference.state_root)
        );
        assert_eq!(Some(chunk_result.final_gas_used), reference.gas_used);
    }

    #[test]
    fn chunk_driver_resumes_from_checkpoint_cursor() {
        let fixture = canonical_fixture();
        let context = load_execution_context(&fixture).expect("execution context should load");
        let payload = build_payload(&fixture, &context).expect("payload should build");
        let plan = build_chunk_plan(&fixture, DEFAULT_CHUNK_SIZE).expect("chunk plan should build");
        let driver = ChunkDriver::new(
            context.rollup_config,
            payload,
            context.parent_header,
            context.provider,
            fixture.pre_checkpoint.witness_bundle_ref.clone(),
        )
        .expect("chunk driver should build");

        let checkpoint = driver.initial_checkpoint();
        let checkpoint = match driver
            .execute_tile(checkpoint, &plan.tiles()[0])
            .expect("first tile should succeed")
        {
            chunk_driver::ChunkTileExecutionOutcome::Checkpointed(checkpoint) => *checkpoint,
            chunk_driver::ChunkTileExecutionOutcome::Finalized(_) => {
                panic!("first tile should not finalize block")
            }
        };

        assert_eq!(checkpoint.tx_cursor(), 1);

        let checkpoint = match driver
            .execute_tile(checkpoint, &plan.tiles()[1])
            .expect("second tile should resume from checkpoint")
        {
            chunk_driver::ChunkTileExecutionOutcome::Checkpointed(checkpoint) => *checkpoint,
            chunk_driver::ChunkTileExecutionOutcome::Finalized(_) => {
                panic!("second tile should not finalize block")
            }
        };

        assert_eq!(checkpoint.tx_cursor(), 2);
    }

    #[test]
    fn nonfinal_tile_returns_correct_output() {
        let fixture = canonical_fixture();
        init_shared_execution(&fixture).expect("shared execution init should succeed");
        let result = execute_tile_shared(&fixture, 0).expect("tile 0 should execute successfully");

        assert_eq!(result.tile_index, 0);
        assert!(!result.seals_block);
        assert!(!result.tx_ids.is_empty());
    }
}
