use alloy_consensus::{transaction::Recovered, BlockHeader, Header, Sealed};
use alloy_eips::{eip1559::BaseFeeParams, eip2718::WithEncoded};
use alloy_evm::{
    block::{BlockExecutor, BlockExecutorFactory},
    op_revm, EvmEnv, EvmFactory,
};
use alloy_op_evm::{block::OpAlloyReceiptBuilder, OpBlockExecutionCtx, OpEvmFactory};
use alloy_primitives::{keccak256, U256};
use kona_executor::TrieDB;
use kona_genesis::RollupConfig;
use kona_mpt::NoopTrieHinter;
use op_alloy_consensus::OpTxEnvelope;
use op_alloy_rpc_types_engine::OpPayloadAttributes;
use revm::{
    context::BlockEnv,
    database::{states::bundle_state::BundleRetention, BundleState, State},
};

use crate::{
    chunk_plan::ChunkTile, missing_witness_class, DiskTrieNodeProvider, Result, WorkloadError,
};

#[cfg(test)]
use crate::chunk_plan::ChunkPlan;

const HOLOCENE_EXTRA_DATA_VERSION: u8 = 0;

type RecoveredOpTx = WithEncoded<Recovered<OpTxEnvelope>>;

#[derive(Debug, Clone)]
pub(crate) struct ChunkExecutionCheckpoint {
    tx_cursor: usize,
    gas_used_so_far: u64,
    #[allow(dead_code)] // checkpoint contract field — used for debugging and later serialization
    last_executed_tx_hash: Option<String>,
    #[allow(dead_code)]
    pending_state_root: String,
    #[allow(dead_code)]
    pending_header_hash: String,
    witness_bundle_ref: String,
    bundle: BundleState,
    trie_db: TrieDB<DiskTrieNodeProvider, NoopTrieHinter>,
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChunkDriverResult {
    pub(crate) final_state_root: String,
    pub(crate) final_gas_used: u64,
}

pub(crate) struct ChunkDriver {
    rollup_config: RollupConfig,
    payload: OpPayloadAttributes,
    recovered_transactions: Vec<RecoveredOpTx>,
    initial_trie_db: TrieDB<DiskTrieNodeProvider, NoopTrieHinter>,
    witness_bundle_ref: String,
}

impl ChunkDriver {
    pub(crate) fn new(
        rollup_config: RollupConfig,
        payload: OpPayloadAttributes,
        parent_header: Sealed<Header>,
        provider: DiskTrieNodeProvider,
        witness_bundle_ref: String,
    ) -> Result<Self> {
        let recovered_transactions = payload
            .recovered_transactions_with_encoded()
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|error| {
                WorkloadError::InvalidFixture(format!(
                    "failed to recover execution transactions: {error}"
                ))
            })?;

        Ok(Self {
            rollup_config,
            payload,
            recovered_transactions,
            initial_trie_db: TrieDB::new(parent_header, provider, NoopTrieHinter),
            witness_bundle_ref,
        })
    }

    pub(crate) fn initial_checkpoint(&self) -> ChunkExecutionCheckpoint {
        let parent_header = self.initial_trie_db.parent_block_header();
        let pending_state_root = format!("{:#x}", parent_header.state_root);
        let pending_header_hash = format!("{:#x}", parent_header.hash());

        ChunkExecutionCheckpoint {
            tx_cursor: 0,
            gas_used_so_far: 0,
            last_executed_tx_hash: None,
            pending_state_root,
            pending_header_hash,
            witness_bundle_ref: self.witness_bundle_ref.clone(),
            bundle: BundleState::default(),
            trie_db: self.initial_trie_db.clone(),
        }
    }

    pub(crate) fn execute_tile(
        &self,
        checkpoint: ChunkExecutionCheckpoint,
        tile: &ChunkTile,
    ) -> Result<ChunkTileExecutionOutcome> {
        if checkpoint.tx_cursor != tile.start_tx_index() {
            return Err(WorkloadError::InvalidChunkResume(format!(
                "chunk checkpoint cursor {} does not match tile {} start {}",
                checkpoint.tx_cursor,
                tile.tile_index(),
                tile.start_tx_index()
            )));
        }

        let mut state = State::builder()
            .with_database(checkpoint.trie_db)
            .with_bundle_prestate(checkpoint.bundle.clone())
            .with_bundle_update()
            .without_state_clear()
            .build();

        let parent_header = state.database.parent_block_header().clone();
        let base_fee_params =
            active_base_fee_params(&self.rollup_config, &parent_header, &self.payload)?;
        let evm_env = evm_env(
            &self.rollup_config,
            &parent_header,
            &self.payload,
            &base_fee_params,
        )?;
        let parent_hash = parent_header.seal();
        let evm = OpEvmFactory::default().create_evm(&mut state, evm_env);
        let factory = alloy_op_evm::block::OpBlockExecutorFactory::new(
            OpAlloyReceiptBuilder::default(),
            self.rollup_config.clone(),
            OpEvmFactory::default(),
        );
        let mut executor = factory.create_executor(
            evm,
            OpBlockExecutionCtx {
                parent_hash,
                parent_beacon_block_root: self.payload.payload_attributes.parent_beacon_block_root,
                extra_data: Default::default(),
            },
        );

        if checkpoint.tx_cursor == 0 {
            executor
                .apply_pre_execution_changes()
                .map_err(|error| map_chunk_error(0, "pre_execution", error.to_string()))?;
        }

        let mut gas_used_so_far = checkpoint.gas_used_so_far;

        for (offset, tx) in self.recovered_transactions
            [tile.start_tx_index()..tile.end_tx_index_exclusive()]
            .iter()
            .enumerate()
        {
            let step_index = tile.start_tx_index() + offset;
            let tx_id = tile.tx_ids()[offset].as_str();
            let gas_used = executor
                .execute_transaction(tx)
                .map_err(|error| map_chunk_error(step_index, tx_id, error.to_string()))?;
            gas_used_so_far += gas_used;
        }

        if tile.seals_block() {
            let (evm, _execution_result) = executor.finish().map_err(|error| {
                map_chunk_error(
                    tile.end_tx_index_exclusive() - 1,
                    "finalize",
                    error.to_string(),
                )
            })?;
            drop(evm);

            state.merge_transitions(BundleRetention::Reverts);
            let bundle = state.take_bundle();
            let final_state_root = state
                .database
                .clone()
                .state_root(&bundle)
                .map_err(|error| WorkloadError::KonaExecution {
                    tx_id: format!("tile-{}", tile.tile_index()),
                    reason: error.to_string(),
                })?;

            return Ok(ChunkTileExecutionOutcome::Finalized(ChunkFinalizedResult {
                final_state_root: format!("{final_state_root:#x}"),
                final_gas_used: gas_used_so_far,
            }));
        }

        drop(executor);
        state.merge_transitions(BundleRetention::Reverts);
        let bundle = state.take_bundle();
        let pending_state_root = state
            .database
            .clone()
            .state_root(&bundle)
            .map_err(|error| WorkloadError::KonaExecution {
                tx_id: format!("tile-{}", tile.tile_index()),
                reason: error.to_string(),
            })?;
        let last_executed_tx_hash = tile.tx_hashes().last().cloned().ok_or_else(|| {
            WorkloadError::InvalidFixture(
                "chunk tile must include at least one transaction".to_string(),
            )
        })?;
        let next_tx_cursor = tile.end_tx_index_exclusive();
        let pending_header_hash = hash_checkpoint(
            &state.database.parent_block_header().hash().to_string(),
            &format!("{pending_state_root:#x}"),
            next_tx_cursor,
            gas_used_so_far,
            &last_executed_tx_hash,
            &checkpoint.witness_bundle_ref,
        );

        Ok(ChunkTileExecutionOutcome::Checkpointed(Box::new(
            ChunkExecutionCheckpoint {
                tx_cursor: next_tx_cursor,
                gas_used_so_far,
                last_executed_tx_hash: Some(last_executed_tx_hash),
                pending_state_root: format!("{pending_state_root:#x}"),
                pending_header_hash,
                witness_bundle_ref: checkpoint.witness_bundle_ref,
                bundle,
                trie_db: state.database,
            },
        )))
    }

    #[cfg(test)]
    pub(crate) fn execute_plan(&self, plan: &ChunkPlan) -> Result<ChunkDriverResult> {
        let mut checkpoint = self.initial_checkpoint();

        for tile in plan.tiles() {
            match self.execute_tile(checkpoint, tile)? {
                ChunkTileExecutionOutcome::Checkpointed(next_checkpoint) => {
                    checkpoint = *next_checkpoint;
                }
                ChunkTileExecutionOutcome::Finalized(finalized) => {
                    return Ok(ChunkDriverResult {
                        final_state_root: finalized.final_state_root,
                        final_gas_used: finalized.final_gas_used,
                    });
                }
            }
        }

        Err(WorkloadError::InvalidChunkResume(
            "chunk plan did not include a sealing tile".to_string(),
        ))
    }
}

#[cfg(test)]
impl ChunkExecutionCheckpoint {
    pub(crate) fn tx_cursor(&self) -> usize {
        self.tx_cursor
    }
}

pub(crate) enum ChunkTileExecutionOutcome {
    Checkpointed(Box<ChunkExecutionCheckpoint>),
    Finalized(ChunkFinalizedResult),
}

pub(crate) struct ChunkFinalizedResult {
    pub(crate) final_state_root: String,
    pub(crate) final_gas_used: u64,
}

fn map_chunk_error(step_index: usize, tx_id: &str, reason: String) -> WorkloadError {
    if let Some(class) = missing_witness_class(&reason) {
        WorkloadError::MissingWitness {
            step_index,
            tx_id: tx_id.to_string(),
            class,
            detail: reason,
        }
    } else {
        WorkloadError::KonaExecution {
            tx_id: tx_id.to_string(),
            reason,
        }
    }
}

fn hash_checkpoint(
    parent_header_hash: &str,
    pending_state_root: &str,
    tx_cursor: usize,
    gas_used_so_far: u64,
    last_executed_tx_hash: &str,
    witness_bundle_ref: &str,
) -> String {
    let mut material = Vec::new();
    material.extend_from_slice(parent_header_hash.as_bytes());
    material.extend_from_slice(pending_state_root.as_bytes());
    material.extend_from_slice(tx_cursor.to_string().as_bytes());
    material.extend_from_slice(gas_used_so_far.to_string().as_bytes());
    material.extend_from_slice(last_executed_tx_hash.as_bytes());
    material.extend_from_slice(witness_bundle_ref.as_bytes());
    format!("{:#x}", keccak256(material))
}

fn evm_env(
    config: &RollupConfig,
    parent_header: &Header,
    payload_attrs: &OpPayloadAttributes,
    base_fee_params: &BaseFeeParams,
) -> Result<EvmEnv<op_revm::OpSpecId>> {
    let block_env = prepare_block_env(
        config.spec_id(payload_attrs.payload_attributes.timestamp),
        parent_header,
        payload_attrs,
        base_fee_params,
    )?;
    let cfg_env = revm::context::CfgEnv::new()
        .with_chain_id(config.l2_chain_id.id())
        .with_spec(config.spec_id(payload_attrs.payload_attributes.timestamp));
    Ok(EvmEnv::new(cfg_env, block_env))
}

fn prepare_block_env(
    spec_id: op_revm::OpSpecId,
    parent_header: &Header,
    payload_attrs: &OpPayloadAttributes,
    base_fee_params: &BaseFeeParams,
) -> Result<BlockEnv> {
    let (params, fraction) = if spec_id.is_enabled_in(op_revm::OpSpecId::ISTHMUS) {
        (
            Some(alloy_eips::eip7840::BlobParams::prague()),
            revm::primitives::eip4844::BLOB_BASE_FEE_UPDATE_FRACTION_PRAGUE,
        )
    } else if spec_id.is_enabled_in(op_revm::OpSpecId::ECOTONE) {
        (
            Some(alloy_eips::eip7840::BlobParams::cancun()),
            revm::primitives::eip4844::BLOB_BASE_FEE_UPDATE_FRACTION_CANCUN,
        )
    } else {
        (None, 0)
    };

    let blob_excess_gas_and_price = parent_header
        .maybe_next_block_excess_blob_gas(params)
        .or_else(|| {
            spec_id
                .is_enabled_in(op_revm::OpSpecId::ECOTONE)
                .then_some(0)
        })
        .map(|excess| revm::context_interface::block::BlobExcessGasAndPrice::new(excess, fraction));
    let next_block_base_fee = parent_header
        .next_block_base_fee(*base_fee_params)
        .unwrap_or_default();

    Ok(BlockEnv {
        number: U256::from(parent_header.number + 1),
        beneficiary: payload_attrs.payload_attributes.suggested_fee_recipient,
        timestamp: U256::from(payload_attrs.payload_attributes.timestamp),
        gas_limit: payload_attrs
            .gas_limit
            .ok_or(WorkloadError::InvalidFixture(
                "missing gas limit in payload attributes".to_string(),
            ))?,
        basefee: next_block_base_fee,
        prevrandao: Some(payload_attrs.payload_attributes.prev_randao),
        blob_excess_gas_and_price,
        ..Default::default()
    })
}

fn active_base_fee_params(
    config: &RollupConfig,
    parent_header: &Header,
    payload_attrs: &OpPayloadAttributes,
) -> Result<BaseFeeParams> {
    if config.is_holocene_active(payload_attrs.payload_attributes.timestamp) {
        Ok(if config.is_holocene_active(parent_header.timestamp) {
            decode_holocene_eip_1559_params(parent_header)?
        } else {
            config.chain_op_config.as_canyon_base_fee_params()
        })
    } else if config.is_canyon_active(payload_attrs.payload_attributes.timestamp) {
        Ok(config.chain_op_config.as_canyon_base_fee_params())
    } else {
        Ok(config.chain_op_config.as_base_fee_params())
    }
}

fn decode_holocene_eip_1559_params(header: &Header) -> Result<BaseFeeParams> {
    if header.extra_data.len() != 1 + 8 || header.extra_data[0] != HOLOCENE_EXTRA_DATA_VERSION {
        return Err(WorkloadError::InvalidFixture(
            "invalid Holocene extra data in parent header".to_string(),
        ));
    }

    let data = &header.extra_data[1..];
    let denominator = u32::from_be_bytes(data[..4].try_into().map_err(|_| {
        WorkloadError::InvalidFixture("invalid Holocene denominator bytes".to_string())
    })?) as u128;
    let elasticity = u32::from_be_bytes(data[4..].try_into().map_err(|_| {
        WorkloadError::InvalidFixture("invalid Holocene elasticity bytes".to_string())
    })?) as u128;

    if denominator == 0 {
        return Err(WorkloadError::InvalidFixture(
            "Holocene max_change_denominator must be non-zero".to_string(),
        ));
    }

    Ok(BaseFeeParams {
        elasticity_multiplier: elasticity,
        max_change_denominator: denominator,
    })
}
