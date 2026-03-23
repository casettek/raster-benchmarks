use serde::{Deserialize, Serialize};

use crate::{FixtureInput, Result, WorkloadError, CANONICAL_FIXTURE_PATH};

pub(crate) const DEFAULT_CHUNK_SIZE: usize = 1;

const CHUNK_PLAN_VERSION: &str = "l2-poc-chunk-plan-v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ChunkPlan {
    contract_version: String,
    fixture_ref: String,
    fixture_id: String,
    tile_progression_model: String,
    chunking_policy: ChunkingPolicy,
    block_global: BlockGlobalFields,
    tile_checkpoint_contract: TileCheckpointContract,
    tiles: Vec<ChunkTile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ChunkingPolicy {
    kind: String,
    chunk_size: usize,
    tracked_tx_count: usize,
    supplemental_tx_count: usize,
    execution_tx_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct BlockGlobalFields {
    start_block: u64,
    end_block: u64,
    start_timestamp: u64,
    gas_limit: u64,
    fee_recipient: String,
    batch_hash: String,
    prev_output_root: String,
    rollup_config_ref: String,
    witness_bundle_ref: String,
    message_passer_storage_root: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct TileCheckpointContract {
    block_global_fields: Vec<String>,
    per_tile_fields: Vec<String>,
    carry_forward_fields: Vec<String>,
    finalization_fields: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ChunkTile {
    tile_index: usize,
    tile_kind: String,
    start_tx_index: usize,
    end_tx_index_exclusive: usize,
    tx_count: usize,
    tx_ids: Vec<String>,
    tx_hashes: Vec<String>,
    tracked_tx_ids: Vec<String>,
    supplemental_tx_ids: Vec<String>,
    seals_block: bool,
    input_checkpoint: TileCheckpointRef,
    output_checkpoint: TileCheckpointRef,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct TileCheckpointRef {
    checkpoint_kind: String,
    tx_cursor: usize,
}

pub(crate) fn build_chunk_plan(fixture: &FixtureInput, chunk_size: usize) -> Result<ChunkPlan> {
    if chunk_size == 0 {
        return Err(WorkloadError::InvalidChunkSize);
    }

    let execution_transactions = fixture.execution_transactions();
    let tracked_tx_count = fixture.transactions.len();
    let supplemental_tx_count = fixture.supplemental_transactions.len();
    let execution_tx_count = execution_transactions.len();
    let mut tiles = Vec::new();

    for (tile_index, chunk) in execution_transactions.chunks(chunk_size).enumerate() {
        let start_tx_index = tile_index * chunk_size;
        let end_tx_index_exclusive = start_tx_index + chunk.len();
        let mut tx_ids = Vec::new();
        let mut tx_hashes = Vec::new();
        let mut tracked_tx_ids = Vec::new();
        let mut supplemental_tx_ids = Vec::new();

        for (chunk_offset, tx) in chunk.iter().enumerate() {
            let execution_index = start_tx_index + chunk_offset;
            tx_ids.push(tx.id.clone());
            tx_hashes.push(tx.hash.clone());

            if execution_index < tracked_tx_count {
                tracked_tx_ids.push(tx.id.clone());
            } else {
                supplemental_tx_ids.push(tx.id.clone());
            }
        }

        tiles.push(ChunkTile {
            tile_index,
            tile_kind: "execute_chunk".into(),
            start_tx_index,
            end_tx_index_exclusive,
            tx_count: chunk.len(),
            tx_ids,
            tx_hashes,
            tracked_tx_ids,
            supplemental_tx_ids,
            seals_block: end_tx_index_exclusive == execution_tx_count,
            input_checkpoint: TileCheckpointRef {
                checkpoint_kind: if start_tx_index == 0 {
                    "pre_checkpoint".into()
                } else {
                    "chunk_checkpoint".into()
                },
                tx_cursor: start_tx_index,
            },
            output_checkpoint: TileCheckpointRef {
                checkpoint_kind: if end_tx_index_exclusive == execution_tx_count {
                    "sealed_block_checkpoint".into()
                } else {
                    "chunk_checkpoint".into()
                },
                tx_cursor: end_tx_index_exclusive,
            },
        });
    }

    Ok(ChunkPlan {
        contract_version: CHUNK_PLAN_VERSION.into(),
        fixture_ref: CANONICAL_FIXTURE_PATH.into(),
        fixture_id: fixture.fixture_id.clone(),
        tile_progression_model: "uniform execute_chunk tiles seeded from pre_checkpoint; the final tile seals the canonical block".into(),
        chunking_policy: ChunkingPolicy {
            kind: "fixed-tx-count".into(),
            chunk_size,
            tracked_tx_count,
            supplemental_tx_count,
            execution_tx_count,
        },
        block_global: BlockGlobalFields {
            start_block: fixture.start_block,
            end_block: fixture.end_block,
            start_timestamp: fixture.start_timestamp,
            gas_limit: fixture.gas_limit,
            fee_recipient: fixture.fee_recipient.clone(),
            batch_hash: fixture.batch_hash.clone(),
            prev_output_root: fixture.pre_checkpoint.prev_output_root.clone(),
            rollup_config_ref: fixture.pre_checkpoint.rollup_config_ref.clone(),
            witness_bundle_ref: fixture.pre_checkpoint.witness_bundle_ref.clone(),
            message_passer_storage_root: fixture
                .output_root_witness
                .message_passer_storage_root
                .clone(),
        },
        tile_checkpoint_contract: TileCheckpointContract {
            block_global_fields: vec![
                "start_block".into(),
                "end_block".into(),
                "start_timestamp".into(),
                "gas_limit".into(),
                "fee_recipient".into(),
                "batch_hash".into(),
                "prev_output_root".into(),
                "rollup_config_ref".into(),
                "witness_bundle_ref".into(),
                "message_passer_storage_root".into(),
            ],
            per_tile_fields: vec![
                "start_tx_index".into(),
                "end_tx_index_exclusive".into(),
                "tx_ids".into(),
                "tx_hashes".into(),
                "tracked_tx_ids".into(),
                "supplemental_tx_ids".into(),
                "seals_block".into(),
            ],
            carry_forward_fields: vec![
                "tx_cursor".into(),
                "pending_header_hash".into(),
                "pending_state_root".into(),
                "gas_used_so_far".into(),
                "last_executed_tx_hash".into(),
                "witness_bundle_ref".into(),
            ],
            finalization_fields: vec!["next_output_root".into(), "sealed_block_hash".into(), "total_gas_used".into()],
        },
        tiles,
    })
}

impl ChunkPlan {
    pub(crate) fn tiles(&self) -> &[ChunkTile] {
        &self.tiles
    }

    pub(crate) fn tile(&self, tile_index: usize) -> Option<&ChunkTile> {
        self.tiles.get(tile_index)
    }
}

impl ChunkTile {
    pub(crate) fn tile_index(&self) -> usize {
        self.tile_index
    }

    pub(crate) fn start_tx_index(&self) -> usize {
        self.start_tx_index
    }

    pub(crate) fn end_tx_index_exclusive(&self) -> usize {
        self.end_tx_index_exclusive
    }

    pub(crate) fn tx_hashes(&self) -> &[String] {
        &self.tx_hashes
    }

    pub(crate) fn tx_ids(&self) -> &[String] {
        &self.tx_ids
    }

    pub(crate) fn seals_block(&self) -> bool {
        self.seals_block
    }
}
