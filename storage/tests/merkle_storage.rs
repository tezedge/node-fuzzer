// Copyright (c) SimpleStaking and Tezedge Contributors
// SPDX-License-Identifier: MIT

use std::convert::TryInto;
use std::error::Error;
use std::collections::VecDeque;
use std::sync::{mpsc, Arc};
use std::thread;
use serde::{Serialize, Deserialize};
use crypto::hash::{HashType, BlockHash};
use rocksdb::{DB, Cache, Options, ColumnFamilyDescriptor};

use storage::*;
use context_action_storage::ContextAction;
use merkle_storage::{MerkleStorage, MerkleError};
use persistent::{PersistentStorage, CommitLogSchema, DbConfiguration, KeyValueSchema, open_cl, open_kv, default_table_options};
use persistent::sequence::Sequences;

fn init_persistent_storage() -> PersistentStorage {
    // Parses config + cli args
    // let env = crate::configuration::Environment::from_args();
    let db_path = "/tmp/tezedge/light-node";

    // create common RocksDB block cache to be shared among column families
    // IMPORTANT: Cache object must live at least as long as DB (returned by open_kv)
    let cache = Cache::new_lru_cache(128 * 1024 * 1024).unwrap(); // 128 MB

    let schemas = vec![
        block_storage::BlockPrimaryIndex::descriptor(&cache),
        block_storage::BlockByLevelIndex::descriptor(&cache),
        block_storage::BlockByContextHashIndex::descriptor(&cache),
        BlockMetaStorage::descriptor(&cache),
        OperationsStorage::descriptor(&cache),
        OperationsMetaStorage::descriptor(&cache),
        context_action_storage::ContextActionByBlockHashIndex::descriptor(&cache),
        context_action_storage::ContextActionByContractIndex::descriptor(&cache),
        context_action_storage::ContextActionByTypeIndex::descriptor(&cache),
        ContextActionStorage::descriptor(&cache),
        SystemStorage::descriptor(&cache),
        Sequences::descriptor(&cache),
        MempoolStorage::descriptor(&cache),
        ChainMetaStorage::descriptor(&cache),
        PredecessorStorage::descriptor(&cache),
        // tmp merkle_storage
        MerkleStorage::descriptor(&cache),
        // original merkle_storage, required by rocksdb
        {
            let cf_opts = default_table_options(&cache);
            ColumnFamilyDescriptor::new("merkle_storage", cf_opts)
        }
    ];

    let opts = DbConfiguration::default();
    let rocks_db = Arc::new(open_kv(db_path, schemas, &opts).unwrap());
    let commit_logs = match open_cl(db_path, vec![BlockStorage::descriptor()]) {
        Ok(commit_logs) => Arc::new(commit_logs),
        Err(e) => panic!(e),
    };

    PersistentStorage::new(rocks_db, commit_logs)
}

struct BlocksIterator {
    block_storage: BlockStorage,
    blocks: std::vec::IntoIter<BlockHeaderWithHash>,
    next_block_chunk_hash: Option<BlockHash>,
    limit: usize,
}

impl BlocksIterator {
    pub fn new(block_storage: BlockStorage, start_block_hash: &BlockHash, limit: usize) -> Self {
        let blocks = Self::get_blocks_after_block(&block_storage, &start_block_hash, limit);
        let mut this = Self { block_storage, limit, next_block_chunk_hash: None, blocks: vec![].into_iter() };

        this.get_and_set_blocks(start_block_hash);
        this
    }

    fn get_and_set_blocks(&mut self, from_block_hash: &BlockHash) -> Result<(), ()> {
        let mut new_blocks = Self::get_blocks_after_block(
            &self.block_storage,
            from_block_hash,
            self.limit,
        );

        if new_blocks.len() == 0 {
            return Err(());
        }
        if new_blocks.len() >= self.limit {
            self.next_block_chunk_hash = Some(new_blocks.pop().unwrap().hash);
        }
        self.blocks = new_blocks.into_iter();

        Ok(())
    }

    fn get_blocks_after_block(
        block_storage: &BlockStorage,
        block_hash: &BlockHash,
        limit: usize
    ) -> Vec<BlockHeaderWithHash> {
        block_storage.get_multiple_without_json(block_hash, limit).unwrap_or(vec![])
    }
}

impl Iterator for BlocksIterator {
    type Item = BlockHeaderWithHash;

    fn next(&mut self) -> Option<Self::Item> {
        match self.blocks.next() {
            Some(block) => Some(block),
            None => {
                if let Some(next_block_chunk_hash) = self.next_block_chunk_hash.take() {
                    if self.get_and_set_blocks(&next_block_chunk_hash).is_ok() {
                        return self.next();
                    }
                }
                None
            },
        }
    }
}

use std::time::Instant;

type BlockAndActions = (BlockHeaderWithHash, Vec<ContextAction>);

fn recv_blocks(
    block_storage: BlockStorage,
    ctx_action_storage: ContextActionStorage,
) -> impl Iterator<Item = BlockAndActions> {
    let genesis_block_hash = HashType::BlockHash.b58check_to_hash("BLockGenesisGenesisGenesisGenesisGenesis355e8bjkYPv").unwrap();
    let (tx, rx) = mpsc::sync_channel(8);

    thread::spawn(move || {
        for block in BlocksIterator::new(block_storage, &genesis_block_hash, 8) {
            let t = Instant::now();
            let mut actions = ctx_action_storage.get_by_block_hash(&block.hash).unwrap();
            println!("fetched actions: {}, duration: {}ms", actions.len(), t.elapsed().as_millis());
            actions.sort_by_key(|x| x.id);
            let actions = actions.into_iter().map(|x| x.action).collect();
            if let Err(_) = tx.send((block, actions)) {
                dbg!("blocks receiver disconnected, shutting down sender thread");
                return;
            }
        }
    });

    rx.into_iter()
}

#[test]
fn test_merkle_storage_gc() {
    let persistent_storage = init_persistent_storage();

    let block_storage = BlockStorage::new(&persistent_storage);
    let ctx_action_storage = ContextActionStorage::new(&persistent_storage);

    for (block, actions) in recv_blocks(block_storage, ctx_action_storage) {
        println!("applying block: {}", block.header.level());

        let merkle_rwlock = persistent_storage.merkle();
        let mut merkle = merkle_rwlock.write().unwrap();
        let t = Instant::now();

        for action in actions.into_iter() {
            merkle.apply_context_action(&action).unwrap();
        }

        println!("applied actions of block: {}, duration: {}ms", block.header.level(), t.elapsed().as_millis());
    }
}
