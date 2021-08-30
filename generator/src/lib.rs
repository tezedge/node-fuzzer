use std::sync::{Arc, Mutex, MutexGuard};
use std::collections::HashMap;
use std::str::FromStr;
use rand::{
    rngs::SmallRng,
    seq::SliceRandom,
    Rng,
    SeedableRng,
    distributions::{
        Standard,
        Distribution,
        uniform::{SampleRange, SampleUniform},
    }
};
use tezos_messages::p2p::encoding::block_header::BlockHeaderMessage;
use tezos_messages::p2p::encoding::operation::OperationMessage;
use tezos_messages::p2p::encoding::operations_for_blocks::OperationsForBlocksMessage;
use tezos_messages::p2p::encoding::prelude::CurrentBranchMessage;
use tezos_messages::p2p::encoding::protocol::ProtocolMessage;
use tezos_messages::p2p::encoding::current_head::CurrentHeadMessage;
use std::net;
use std::convert::TryInto;
use hex::FromHex;

use tezos_messages::p2p::binary_message::{BinaryChunk, BinaryWrite, CONTENT_LENGTH_MAX, MessageHash};
use tezos_messages::p2p::encoding::{
    advertise,
    block_header,
    current_branch,
    current_head,
    deactivate, 
    limits,
    mempool, 
    operation,
    peer,
    swap,
    protocol,
    operations_for_blocks,
    connection,
    version,
    metadata,
    ack
}; 

use crypto::{
    crypto_box::{CryptoKey, PublicKey, SecretKey, PrecomputedKey, CRYPTO_KEY_SIZE},
    hash::*,
    proof_of_work::{ProofOfWork, POW_SIZE},
    nonce::{Nonce, NONCE_SIZE}
};

type BlocksMap = HashMap<BlockHash, block_header::BlockHeader>;

#[derive(Clone)]
pub struct _RandomState {
    rng: SmallRng,
    blocks_limit: usize,
    max_chunk_size: usize,
    fitness_count: u64,
    pub blocks: BlocksMap,
    last_block: BlockHash,
    chain_id: ChainId,
    current_branch: Option<CurrentBranchMessage>,
    current_head: Option<CurrentHeadMessage>,
    block_header: Option<BlockHeaderMessage>,
    operation: Option<OperationMessage>,
    protocol: Option<ProtocolMessage>,
    operations_for_blocks: Option<OperationsForBlocksMessage>
}

impl _RandomState {
    pub fn new(seed: u64, blocks_limit: usize, max_chunk_size: usize) -> Self {
        _RandomState {
            rng: SmallRng::seed_from_u64(seed),
            blocks_limit: blocks_limit,
            max_chunk_size: max_chunk_size,
            fitness_count: 0,
            blocks: BlocksMap::new(),
            last_block: BlockHash::from_base58_check(
                "BLockGenesisGenesisGenesisGenesisGenesisf79b5d1CoW2"
            ).unwrap(),
            chain_id: ChainId::from_str("NetXdQprcVkpaWU").unwrap(),
            current_branch: None,
            current_head: None,
            block_header: None,
            operation: None,
            protocol: None,
            operations_for_blocks: None
        }
    }
}

#[derive(Clone)]
pub struct RandomState(Arc<Mutex<_RandomState>>);

impl RandomState {
    pub fn new(seed: u64, blocks_limit: usize, max_chunk_size: usize) -> Self {
        RandomState {
            0: Arc::new(Mutex::new(_RandomState::new(seed, blocks_limit, max_chunk_size)))
        }
    }

    pub fn gen_fitness(&mut self) -> u64 {
        self.0.lock().unwrap().fitness_count += 1;
        self.0.lock().unwrap().fitness_count
    }

    pub fn gen_range<T, R>(&mut self, range: R) -> T 
    where
        T: SampleUniform,
        R: SampleRange<T> {
        self.0.lock().unwrap().rng.gen_range::<T, R>(range)
    }

    pub fn shuffle<T>(&mut self, vec: &mut Vec<T>) {
        vec.shuffle(&mut self.0.lock().unwrap().rng);
    }

    pub fn gen_chunks(&mut self, bytes: Vec<u8>, key: &PrecomputedKey, nonce: &mut Nonce) -> Vec<BinaryChunk> {
        const MACBYTES: usize = 16;
        let mut start = 0;
        let mut remaining = bytes.len();    
        let mut ret = Vec::new();
        let mut chunk_sizes = Vec::new();
        let max_chunk_size = self.0.lock().unwrap().max_chunk_size;
        let max_size = std::cmp::min(max_chunk_size,CONTENT_LENGTH_MAX - MACBYTES);
    
        loop {
            if remaining <= 1 {
                if remaining == 1 {
                    chunk_sizes.push(1);
                }
                break;
            }
    
            let size = match max_size {
                1 => 1,
                _ => self.gen_range(1..std::cmp::min(remaining, max_size)),
            };
            chunk_sizes.push(size);
            remaining -= size;
        }
    
        self.shuffle::<usize>(&mut chunk_sizes);
    
        for chunk_size in chunk_sizes {
            let bytes = key.encrypt(&bytes[start..start + chunk_size], &nonce).unwrap();
            *nonce = nonce.increment();
            ret.push(BinaryChunk::from_content(&bytes).unwrap());
            start += chunk_size;
        }

        ret    
    }

    pub fn gen_byte_chunks(&mut self, bytes: Vec<u8>) -> Vec<Vec<u8>> {
        let mut start = 0;
        let mut remaining = bytes.len();    
        let mut ret = Vec::new();
        let mut chunk_sizes = Vec::new();
    
        loop {
            if remaining <= 1 {
                if remaining == 1 {
                    chunk_sizes.push(1);
                }
                break;
            }
    
            let size = self.gen_range(1..remaining);
            chunk_sizes.push(size);
            remaining -= size;
        }
    
        self.shuffle::<usize>(&mut chunk_sizes);
    
        for chunk_size in chunk_sizes {
            ret.push(Vec::from(&bytes[start..start + chunk_size]));
            start += chunk_size;
        }

        ret    
    }

    pub fn gen_bool(&mut self, p: f64) -> bool {
        self.0.lock().unwrap().rng.gen_bool(p)
    }

    pub fn gen_t<T>(&mut self) -> T where Standard: Distribution<T> {
        self.0.lock().unwrap().rng.gen::<T>()
    }

    fn gen_vec<T>(&mut self, size: usize) -> Vec<T>
    where Standard: Distribution<T> {
        (0..size).map(|_| { self.gen_t::<T>() }).collect()
    }
    
    fn gen_string(&mut self, max_size_bytes: usize) -> String {
        let mut string: String = (0..max_size_bytes)
        .map(|_| { self.gen_t::<char>() }).collect();
        
        for i in (0..max_size_bytes).rev() {
            if string.is_char_boundary(i) {
                string.truncate(i);
                break;
            }
        }

        string
    }

    pub fn get_chain_id(&mut self) -> ChainId {
        self.0.lock().unwrap().chain_id.clone()
    }

    pub fn set_chain_id(&mut self, chain_id: ChainId) {
        self.0.lock().unwrap().chain_id = chain_id
    }

    pub fn get_current_branch(&mut self) -> Option<CurrentBranchMessage> {
        self.0.lock().unwrap().current_branch.clone()
    }

    pub fn set_current_branch(&mut self, current_branch: CurrentBranchMessage) {
        let fitness = current_branch.current_branch().current_head().fitness();
        if fitness.len() > 1 {
            self.0.lock().unwrap().fitness_count = u64::from_be_bytes(fitness[1].clone().try_into().unwrap());
            self.0.lock().unwrap().current_branch = Some(current_branch);    
        }
    }

    pub fn get_current_head(&mut self) -> Option<CurrentHeadMessage> {
        self.0.lock().unwrap().current_head.clone()
    }

    pub fn set_current_head(&mut self, current_head: CurrentHeadMessage) {
        let fitness = current_head.current_block_header().fitness();
        if fitness.len() > 1 {
            self.0.lock().unwrap().fitness_count = u64::from_be_bytes(fitness[1].clone().try_into().unwrap());
            self.0.lock().unwrap().current_head = Some(current_head);
        }
    }

    pub fn get_block_header(&mut self) -> Option<BlockHeaderMessage> {
        self.0.lock().unwrap().block_header.clone()
    }

    pub fn set_block_header(&mut self, block_header: BlockHeaderMessage) {
        self.0.lock().unwrap().block_header = Some(block_header)
    }

    pub fn get_operation(&mut self) -> Option<OperationMessage> {
        self.0.lock().unwrap().operation.clone()
    }

    pub fn set_operation(&mut self, operation: OperationMessage) {
        self.0.lock().unwrap().operation = Some(operation)
    }

    pub fn get_protocol(&mut self) -> Option<ProtocolMessage> {
        self.0.lock().unwrap().protocol.clone()
    }

    pub fn set_protocol(&mut self, protocol: ProtocolMessage) {
        self.0.lock().unwrap().protocol = Some(protocol)
    }

    pub fn get_operations_for_blocks(&mut self) -> Option<OperationsForBlocksMessage> {
        self.0.lock().unwrap().operations_for_blocks.clone()
    }

    pub fn set_operations_for_blocks(&mut self, operations_for_blocks: OperationsForBlocksMessage) {
        self.0.lock().unwrap().operations_for_blocks = Some(operations_for_blocks)
    }

    fn block_hash(bh: &block_header::BlockHeader) -> BlockHash {
        let hash = bh.message_hash().unwrap();
        BlockHash::try_from_bytes(&hash[..]).unwrap()
    }

    pub fn state(&mut self) -> MutexGuard<_RandomState> {
        self.0.lock().unwrap()
    }

    pub fn get_block(&mut self, block_hash: &BlockHash) -> Option<block_header::BlockHeaderMessage> {
        let state = self.state();
        let bh = state.blocks.get(block_hash);

        if bh.is_some() {
            Some(block_header::BlockHeaderMessage::from(bh.unwrap().clone()))
        }
        else {
            None
        }        
    }

    fn set_last_block_hash(&mut self, hash: BlockHash) {
        self.state().last_block = hash;
    }
}

pub trait Generator<T> {
    fn gen(&mut self) -> T;
}
pub trait Mutator<T> {
    fn mutate(&mut self, data: T) -> T;
} 

impl Generator<net::IpAddr> for RandomState {
    fn gen(&mut self) -> net::IpAddr {
        let [a, b, c, d] = self.gen_t::<[u8;4]>();
        net::IpAddr::V4(net::Ipv4Addr::new(a, b, c, d))
    }
}

impl Generator<net::SocketAddr> for RandomState {
    fn gen(&mut self) -> net::SocketAddr {
        net::SocketAddr::new(self.gen(), self.gen_t())
    }
}

struct HashKind<T>(T);

impl<T> Generator<HashKind<T>> for RandomState where T: HashTrait {
    fn gen(&mut self) -> HashKind<T> {
        let bytes = self.gen_vec::<u8>(T::hash_type().size());
        HashKind{0: T::try_from_bytes(&bytes[..]).unwrap()}
     }
}

impl<T> Mutator<HashKind<T>> for RandomState where T: HashTrait {
    fn mutate(&mut self, data: HashKind<T>) -> HashKind<T> {
        // No point in mutating hashes
        data
    }
}

impl Generator<advertise::AdvertiseMessage> for RandomState {
    fn gen(&mut self) -> advertise::AdvertiseMessage {
        let size = self.gen_range(0..limits::ADVERTISE_ID_LIST_MAX_LENGTH);
        let addresses = (0..size).map(|_| {self.gen()})
        .collect::<Vec<net::SocketAddr>>();    
        advertise::AdvertiseMessage::new(&addresses)
    }
}

impl Generator<swap::SwapMessage> for RandomState {
    fn gen(&mut self) -> swap::SwapMessage {
        //let point = "127.0.0.1:12345".to_string();
        let size_bytes = self.gen_range(0..limits::P2P_POINT_MAX_LENGTH);
        let peer_id: HashKind<CryptoboxPublicKeyHash> = self.gen();
        swap::SwapMessage::new(self.gen_string(size_bytes), peer_id.0)
    }
}

impl Generator<current_branch::GetCurrentBranchMessage> for RandomState {
    fn gen(&mut self) -> current_branch::GetCurrentBranchMessage {
        let chain_id: HashKind<ChainId> = if self.gen_bool(0.5) {
            self.gen()
        }
        else {
            HashKind{ 0: self.get_chain_id() }
        };
        current_branch::GetCurrentBranchMessage::new(chain_id.0)
    }
}

impl Generator<block_header::BlockHeaderBuilder> for RandomState {
    fn gen(&mut self) -> block_header::BlockHeaderBuilder {
        let max_size = limits::BLOCK_HEADER_MAX_SIZE;
        let mut remaining: usize = max_size - 0x1000;
        let mut fitness: Vec<Vec<u8>> = Vec::new();
        let mut remaining_fitness_size = 28;
        /*
        fitness.push(vec![0 as u8, '1' as u8]);
        fitness.push(self.gen_fitness().to_be_bytes().to_vec());
        remaining -= 2 + 8;
        */
        loop {
            let element_size = self.gen_range(1..28);
            let element = self.gen_vec::<u8>(element_size);

            if element_size + 4 > remaining_fitness_size {
                break;
            }

            fitness.push(element);
            remaining_fitness_size -= element_size + 4;
            remaining -= element_size + 4;
        }
        
        let data_len = self.gen_range(1..remaining);
        let predecessor: HashKind<BlockHash> = self.gen();
        let operations_hash: HashKind<OperationListListHash> = self.gen();
        let context: HashKind<ContextHash> = self.gen();

        let mut b = block_header::BlockHeaderBuilder::default();
        b.level(self.gen_t())
        .proto(self.gen_t())
        .predecessor(predecessor.0)
        .timestamp(self.gen_t())
        .validation_pass(self.gen_t())
        .operations_hash(operations_hash.0)
        .fitness(fitness)
        .context(context.0)
        .protocol_data(self.gen_vec::<u8>(data_len)).clone()
    }
}

impl Generator<block_header::BlockHeader> for RandomState {
    fn gen(&mut self) -> block_header::BlockHeader {
        let builder: block_header::BlockHeaderBuilder = self.gen();
        builder.build().unwrap()
    }
}

impl Mutator<block_header::BlockHeader> for RandomState {
    fn mutate(&mut self, data: block_header::BlockHeader) -> block_header::BlockHeader {
        let max_size = limits::BLOCK_HEADER_MAX_SIZE;
        let mut remaining: usize = max_size - 0x1000;

        let level = if self.gen_bool(0.9) { data.level() + 1 } else { self.gen_t::<i32>() };
        let proto = if self.gen_bool(0.9) { data.proto() } else { self.gen_t::<u8>() };
        let predecessor = match self.get_current_head() {
                Some(head) => Self::block_hash(&head.current_block_header()),
                _ => {
                    let mut blockb: block_header::BlockHeaderBuilder = self.gen();
                    let predecessor = self.state().last_block.clone();
                    let new_block = blockb.predecessor(predecessor).build().unwrap();
                    Self::block_hash(&new_block)
                }
            };
        let timestamp = if self.gen_bool(0.9) {
            data.timestamp() + self.gen_range(0..60*60)
        }
        else {
            self.gen_t::<i64>()
        };
        let validation_pass = if self.gen_bool(0.9) { data.validation_pass() + 1 } else { self.gen_t::<u8>() };
        // TODO: generate from operations
        let operations_hash: HashKind<OperationListListHash> = self.gen();
        let mut fitness: Vec<Vec<u8>> = Vec::new();
        let mut remaining_fitness_size = 28;
        
        if self.gen_bool(0.9) {
            fitness.push(data.fitness()[0].clone());
            remaining -= fitness[0].len();
            fitness.push(self.gen_fitness().to_be_bytes().to_vec());
            remaining -= fitness[1].len();
        }
        else {
            loop {
                let element_size = self.gen_range(1..28);
                let element = self.gen_vec::<u8>(element_size);
    
                if element_size + 4 > remaining_fitness_size {
                    break;
                }
    
                fitness.push(element);
                remaining_fitness_size -= element_size + 4;
                remaining -= element_size + 4;
            }
        }
        // TODO: generate from new context
        let context: HashKind<ContextHash> = self.gen();
        let bytes = data.protocol_data();
        let bytes_len = bytes.len();
        
        let protocol_data = if self.gen_bool(0.9) && bytes_len > 0 {
            let mut remaining_flips = self.gen_range(0 .. bytes_len) >> 3;
            let mut mutated = Vec::new();
            
            for b in bytes {
                if remaining_flips > 0 && self.gen_bool(0.1) {
                    remaining_flips -= 1;
                    mutated.push(*b ^ (1 << self.gen_range(0 .. 8)));
                }
                else {
                    mutated.push(*b);
                }
            }
            mutated
        }
        else {
            let data_len = self.gen_range(1..remaining);
            self.gen_vec::<u8>(data_len)
        };

        let mut b = block_header::BlockHeaderBuilder::default();
        b.level(level)
        .proto(proto)
        .predecessor(predecessor)
        .timestamp(timestamp)
        .validation_pass(validation_pass)
        .operations_hash(operations_hash.0)
        .fitness(fitness)
        .context(context.0)
        .protocol_data(protocol_data).build().unwrap()
    }
}


impl Generator<current_branch::CurrentBranch> for RandomState {
    fn gen(&mut self) -> current_branch::CurrentBranch {
        let history_len = self.gen_range(0..limits::CURRENT_BRANCH_HISTORY_MAX_LENGTH);
        let history: Vec<BlockHash> = (1..history_len).map(|_|{
            let HashKind::<BlockHash>{0: block_hash} = self.gen();
            block_hash
        }).collect();
        
        let mut blockb: block_header::BlockHeaderBuilder = self.gen();
        let predecessor = self.state().last_block.clone();
        let new_block = blockb.predecessor(predecessor).build().unwrap();
        let block_hash = Self::block_hash(&new_block);
        let state = self.state().clone();

        if state.blocks_limit != 0 {
            /* remove arbitrary block if map is full */
            if state.blocks.len() == state.blocks_limit {
                let arbitrary_key: Vec<&BlockHash> = state.blocks.keys().take(1).collect();
                eprintln!("removing {} from blocks map", arbitrary_key[0].to_base58_check());
                self.state().blocks.remove(arbitrary_key[0]);
            }
            eprintln!(
                "New block hash {}, predecessor {}",
                block_hash.clone().to_base58_check(),
                new_block.predecessor().to_base58_check()
            );    
            self.state().blocks.insert(block_hash.clone(), new_block.clone());
        }

        self.set_last_block_hash(block_hash);
        current_branch::CurrentBranch::new(new_block, history)
    }
}

impl Mutator<current_branch::CurrentBranch> for RandomState {
    fn mutate(&mut self, data: current_branch::CurrentBranch) -> current_branch::CurrentBranch {
        let current_head = self.mutate(data.current_head().clone());
        let history = if self.gen_bool(0.5) {
            data.history().clone()
        }
        else {
            let history_len = self.gen_range(1..limits::CURRENT_BRANCH_HISTORY_MAX_LENGTH);
            (0..history_len).map(|_|{
                let HashKind::<BlockHash>{0: block_hash} = self.gen();
                block_hash
            }).collect()   
        };

        let block_hash = Self::block_hash(&current_head);
        self.set_last_block_hash(block_hash);
        current_branch::CurrentBranch::new(current_head, history)
    }
}

impl Mutator<current_branch::CurrentBranchMessage> for RandomState {
    fn mutate(&mut self, data: current_branch::CurrentBranchMessage) -> current_branch::CurrentBranchMessage {
        let chain_id = HashKind::<ChainId>(data.chain_id().clone());
        current_branch::CurrentBranchMessage::new(
            self.mutate(chain_id).0,
            self.mutate(data.current_branch().clone())
        )
    }
}

impl Generator<current_branch::CurrentBranchMessage> for RandomState {
    fn gen(&mut self) -> current_branch::CurrentBranchMessage {
        let current_branch = self.get_current_branch();

        if current_branch.is_some() {
            self.mutate(current_branch.unwrap())
        }
        else {
            let chain_id: HashKind<ChainId> = if self.gen_bool(0.5) { self.gen() } else { HashKind{ 0: self.get_chain_id() } };
            current_branch::CurrentBranchMessage::new(chain_id.0, self.gen())
        }
    }
}

impl Generator<deactivate::DeactivateMessage> for RandomState {
    fn gen(&mut self) -> deactivate::DeactivateMessage {
        let chain_id: HashKind<ChainId> = if self.gen_bool(0.5) {
            self.gen()
        }
        else {
            HashKind{ 0: self.get_chain_id() }
        };

        deactivate::DeactivateMessage::new(chain_id.0)
    }
}

impl Generator<current_head::GetCurrentHeadMessage> for RandomState {
    fn gen(&mut self) -> current_head::GetCurrentHeadMessage {
        let chain_id: HashKind<ChainId> = if self.gen_bool(0.5) {
            self.gen()
        }
        else {
            HashKind{ 0: self.get_chain_id() }
        };

        current_head::GetCurrentHeadMessage::new(chain_id.0)
    }
}

impl Generator<mempool::Mempool> for RandomState {
    fn gen(&mut self) -> mempool::Mempool {
        match self.get_current_head() {
            Some(current_head) => self.mutate(current_head.current_mempool().clone()),
            _ => {
                let max_count = limits::MEMPOOL_MAX_OPERATIONS;
                let pending_num = self.gen_range(1..max_count-2);
                let known_valid_num = self.gen_range(1..max_count-pending_num);
                let pending: Vec<OperationHash> = (0..pending_num).map(|_|{
                    let HashKind::<OperationHash>{0: op_hash} = self.gen();
                    op_hash
                }).collect();
                let known_valid: Vec<OperationHash> = (0..known_valid_num).map(|_|{
                    let HashKind::<OperationHash>{0: op_hash} = self.gen();
                    op_hash
                }).collect();
                mempool::Mempool::new(known_valid, pending)
            }
        }
    }
}

impl Mutator<mempool::Mempool> for RandomState {
    fn mutate(&mut self, data: mempool::Mempool) -> mempool::Mempool {
        let max_count = limits::MEMPOOL_MAX_OPERATIONS;

        let known_valid = if self.gen_bool(0.5) {
            if self.gen_bool(0.5) {
                data.known_valid().clone()
            }
            else {
                // randomly flip lists
                data.pending().clone()
            }
        }
        else {
            let known_valid_num = self.gen_range(1..max_count);
            (0..known_valid_num).map(|_|{
                let HashKind::<OperationHash>{0: op_hash} = self.gen();
                op_hash
            }).collect()
        };

        let remaining = max_count - known_valid.len();

        let pending: Vec<OperationHash> = if self.gen_bool(0.5) {
            if self.gen_bool(0.5) { 
                let remaining = std::cmp::min(data.pending().len(), remaining);
                data.pending()[0..remaining].iter().cloned().collect()
            }
            else {
                // randomly flip lists
                let remaining = std::cmp::min(data.known_valid().len(), remaining);
                data.known_valid()[0..remaining].iter().cloned().collect()
            }
        }
        else {
            let pending_num = if remaining > 1 { self.gen_range(1..remaining) } else { remaining };
            (0..pending_num).map(|_|{
                let HashKind::<OperationHash>{0: op_hash} = self.gen();
                op_hash
            }).collect()
        };
        mempool::Mempool::new(known_valid, pending)
    }
}


impl Generator<current_head::CurrentHeadMessage> for RandomState {
    fn gen(&mut self) -> current_head::CurrentHeadMessage {
        match self.get_current_head() {
            Some(current_head) => self.mutate(current_head),
            _ => {
                let chain_id: HashKind<ChainId> = if self.gen_bool(0.5) {
                    self.gen()
                }
                else {
                    HashKind{ 0: self.get_chain_id() }
                };
        
                current_head::CurrentHeadMessage::new(
                    chain_id.0,
                    self.gen(),
                    self.gen()
                )        
            }
        }
    }
}

impl Mutator<current_head::CurrentHeadMessage> for RandomState {
    fn mutate(&mut self, data: current_head::CurrentHeadMessage) -> current_head::CurrentHeadMessage {
        let chain_id = HashKind::<ChainId>(data.chain_id().clone());
        current_head::CurrentHeadMessage::new(
            self.mutate(chain_id).0,
            self.mutate(data.current_block_header().clone()),
            self.mutate(data.current_mempool().clone())
        )
    }
}

// TODO: get history
impl Generator<block_header::GetBlockHeadersMessage> for RandomState {
    fn gen(&mut self) -> block_header::GetBlockHeadersMessage {
        let count = self.gen_range(0..limits::GET_BLOCK_HEADERS_MAX_LENGTH);
        let block_hashes: Vec<BlockHash> = (0..count).map(|_|{
            let HashKind::<BlockHash>{0: block_hash} = self.gen();
            block_hash
        }).collect();
        block_header::GetBlockHeadersMessage::new(block_hashes)
    }
}

impl Generator<block_header::BlockHeaderMessage> for RandomState {
    fn gen(&mut self) -> block_header::BlockHeaderMessage {
        match self.get_block_header() {
            Some(block_header) => self.mutate(block_header),
            _ => {
                let block_header: block_header::BlockHeader = self.gen();
                block_header::BlockHeaderMessage::from(block_header)    
            }
        }
    }
}

impl Mutator<block_header::BlockHeaderMessage> for RandomState {
    fn mutate(&mut self, data: block_header::BlockHeaderMessage) -> block_header::BlockHeaderMessage {
        block_header::BlockHeaderMessage::from(self.mutate(data.block_header().clone()))
    }
}

// TODO: get hashes
impl Generator<operation::GetOperationsMessage> for RandomState {
    fn gen(&mut self) -> operation::GetOperationsMessage {
        let count = self.gen_range(0..limits::GET_OPERATIONS_MAX_LENGTH);
        let operations: Vec<OperationHash> = (0..count).map(|_|{
            let HashKind::<OperationHash>{0: op_hash} = self.gen();
            op_hash
        }).collect();
        operation::GetOperationsMessage::new(operations)
    }
}

impl Generator<operation::Operation> for RandomState {
    fn gen(&mut self) -> operation::Operation {
        let max_size = limits::OPERATION_MAX_SIZE - BlockHash::hash_size() - 0x20;
        let size = max_size; //self.gen_range(0..max_size);
        let last_block = match self.get_current_head() {
            Some(head) => Self::block_hash(&head.current_block_header()),
            _ => if self.gen_bool(0.5) {
                self.state().last_block.clone()
            }
            else {
                let bh: HashKind<BlockHash> = self.gen();
                bh.0
            }
        };

        operation::Operation::new(
            last_block,
            self.gen_vec(size) // TODO: Operation Message data generator!!
        )
    }
}

impl Mutator<operation::Operation> for RandomState {
    fn mutate(&mut self, data: operation::Operation) -> operation::Operation {
        let branch = if self.gen_bool(0.5) {
            data.branch().clone()
        }
        else {
            match self.get_current_head() {
                Some(head) => Self::block_hash(&head.current_block_header()),
                _ => if self.gen_bool(0.5) {
                    self.state().last_block.clone()
                }
                else {
                    let bh: HashKind<BlockHash> = self.gen();
                    bh.0
                }
            }    
        };
        let data = if self.gen_bool(0.9) {
            let bytes = data.data().clone();
            let bytes_len = bytes.len();
            let mut remaining_flips = if bytes_len > 0 { self.gen_range(0 .. bytes_len) >> 3 } else { bytes_len };
            let mut mutated = Vec::new();
            
            for b in bytes {
                if remaining_flips > 0 && self.gen_bool(0.1) {
                    remaining_flips -= 1;
                    mutated.push(b ^ (1 << self.gen_range(0 .. 8)));
                }
                else {
                    mutated.push(b);
                }
            }
            mutated
        }
        else {
            let data_len = data.data().len();
            let data_len = if data_len > 1 { self.gen_range(1..data_len) } else { data_len };
            self.gen_vec::<u8>(data_len)
        };
        operation::Operation::new(branch, data)
    }
}


impl Generator<operation::OperationMessage> for RandomState {
    fn gen(&mut self) -> operation::OperationMessage {
        match self.get_operation() {
            Some(operation) => {
                let operation = operation.operation();
                operation::OperationMessage::from(self.mutate(operation.clone()))    
            },
            _ => {
                let operation: operation::Operation = self.gen();
                operation::OperationMessage::from(operation)    
            }
        }
    }
}

// TODO get hash
impl Generator<protocol::GetProtocolsMessage> for RandomState {
    fn gen(&mut self) -> protocol::GetProtocolsMessage {
        let count = self.gen_range(0..limits::GET_PROTOCOLS_MAX_LENGTH);
        let get_protocols: Vec<ProtocolHash> = (0..count).map(|_|{
            let HashKind::<ProtocolHash>{0: protocol_hash} = self.gen();
            protocol_hash
        }).collect();
        protocol::GetProtocolsMessage::new(get_protocols)
    }
}

impl Generator<protocol::Component> for RandomState {
    fn gen(&mut self) -> protocol::Component {
        let size = self.gen_range(0..1025);
        let name = self.gen_string(size);
        let interface = match self.gen_bool(0.5) {
            true => {
                let size = self.gen_range(0..102401);
                let iface = self.gen_string(size);
                Some(iface)
            },
            false => None,
        };
        let size = self.gen_range(0..102401);
        let implementation = self.gen_string(size);
        protocol::Component::new(name, interface, implementation)
    }
}

impl Generator<protocol::Protocol> for RandomState {
    fn gen(&mut self) -> protocol::Protocol {
        let mut components: Vec<protocol::Component> = Vec::new();
        let mut remaining = limits::PROTOCOL_COMPONENT_MAX_SIZE;

        loop {
            let component: protocol::Component = self.gen();
            let size = component.as_bytes().unwrap().len();

            if size > remaining {
                break;
            }
            
            remaining -= size;
            components.push(component);
        }
        protocol::Protocol::new(self.gen_t(), components)
    }
}

// TODO use mutator
impl Generator<operations_for_blocks::OperationsForBlock> for RandomState {
    fn gen(&mut self) -> operations_for_blocks::OperationsForBlock {
        let block_hash: HashKind<BlockHash> = self.gen();
        operations_for_blocks::OperationsForBlock::new(
            block_hash.0, self.gen_t()
        )
    }
}

impl Generator<operations_for_blocks::GetOperationsForBlocksMessage> for RandomState {
    fn gen(&mut self) -> operations_for_blocks::GetOperationsForBlocksMessage {
        let count = self.gen_range(0..limits::GET_OPERATIONS_FOR_BLOCKS_MAX_LENGTH);
        let operations: Vec<operations_for_blocks::OperationsForBlock> = (0..count)
        .map(|_|{self.gen()}).collect();
        operations_for_blocks::GetOperationsForBlocksMessage::new(operations)
    }
}

impl Generator<protocol::ProtocolMessage> for RandomState {
    fn gen(&mut self) -> protocol::ProtocolMessage {
        let protocol = self.get_protocol();

        if protocol.is_some() && self.gen_bool(0.5) {
            // TODO mutate
            protocol.unwrap()
        }
        else {
            protocol::ProtocolMessage::new(self.gen())
        }
    }
}

impl Generator<operations_for_blocks::PathItem> for RandomState {
    fn gen(&mut self) -> operations_for_blocks::PathItem {
        /* FIXME: random sizes serialize but fail to deserialize */
        let size = 32; // self.gen_range(0..0x1000);
        let hash = self.gen_vec::<u8>(size);
        match self.gen_bool(0.5) {
            true => operations_for_blocks::PathItem::right(hash),
            false => operations_for_blocks::PathItem::left(hash),
        }
    }
}

impl Generator<operations_for_blocks::Path> for RandomState {
    fn gen(&mut self) -> operations_for_blocks::Path {
        let count = self.gen_range(0..operations_for_blocks::MAX_PASS_MERKLE_DEPTH.unwrap());
        operations_for_blocks::Path::new((0..count).map(|_|{self.gen()}).collect())
    }
}

// TODO use operation mutator
impl Generator<Vec::<operation::Operation>> for RandomState {
    fn gen(&mut self) -> Vec::<operation::Operation> {
        let mut remaining = limits::OPERATION_LIST_MAX_SIZE;
        let mut operations = Vec::<operation::Operation>::new(); 
        loop {
            let operation: operation::Operation = self.gen();
            let operation_size = operation.as_bytes().unwrap().len();
            
            if operation_size > remaining {
                break;
            }
            remaining -= operation_size;
            operations.push(operation);
        }
        operations
    }
}

impl Generator<operations_for_blocks::OperationsForBlocksMessage> for RandomState {
    fn gen(&mut self) -> operations_for_blocks::OperationsForBlocksMessage {
        match self.get_operations_for_blocks() {
            Some(operations_for_blocks_msg) => {
                let operations_for_blocks = if self.gen_bool(0.5) {
                    operations_for_blocks_msg.operations_for_block().clone()
                }
                else {
                    self.gen()
                };
                let operations_hashes_path = if self.gen_bool(0.5) {
                    operations_for_blocks_msg.operation_hashes_path().clone()
                }
                else {
                    self.gen()
                };
                let operations = if self.gen_bool(0.5) {
                    operations_for_blocks_msg.operations().clone()
                }
                else {
                    self.gen()
                };

                operations_for_blocks::OperationsForBlocksMessage::new(
                    operations_for_blocks,
                    operations_hashes_path,
                    operations
                )
            },
            _ => operations_for_blocks::OperationsForBlocksMessage::new(
                self.gen(),
                self.gen(),
                self.gen()
            )
        }
    }
}

impl Generator<version::NetworkVersion> for RandomState {
    fn gen(&mut self) -> version::NetworkVersion {
        let size = self.gen_range(0..limits::CHAIN_NAME_MAX_LENGTH);
        let chain_name = self.gen_string(size);
        version::NetworkVersion::new(chain_name, self.gen_t(), self.gen_t())
    }
}

impl Generator<PublicKey> for RandomState {
    fn gen(&mut self) -> PublicKey {
        let public_key = self.gen_vec::<u8>(CRYPTO_KEY_SIZE);
        PublicKey::from_bytes(public_key).unwrap()
    }
}

impl Generator<ProofOfWork> for RandomState {
    fn gen(&mut self) -> ProofOfWork {
        let pow: [u8;POW_SIZE] = self.gen_vec::<u8>(POW_SIZE)
        .as_slice().try_into().unwrap();
        ProofOfWork::from_hex(hex::encode(pow)).unwrap()
    }
}

impl Generator<Nonce> for RandomState {
    fn gen(&mut self) -> Nonce {
        let nonce: [u8;NONCE_SIZE] = self.gen_vec::<u8>(NONCE_SIZE)
        .as_slice().try_into().unwrap();
        Nonce::new(&nonce)
    }
}

impl Generator<connection::ConnectionMessage> for RandomState {
    fn gen(&mut self) -> connection::ConnectionMessage {
            connection::ConnectionMessage::try_new(
            self.gen_t(),
            &self.gen(),
            &self.gen(),
            self.gen(),
            self.gen()
        ).unwrap()
    }
}

impl Generator<metadata::MetadataMessage> for RandomState {
    fn gen(&mut self) -> metadata::MetadataMessage {
        metadata::MetadataMessage::new(self.gen_t(), self.gen_t())
    }
}

impl Generator<ack::NackInfo> for RandomState {
    fn gen(&mut self) -> ack::NackInfo {
        let motive = match self.gen_range(0..6) {
            0 => ack::NackMotive::NoMotive,
            1 => ack::NackMotive::TooManyConnections,
            2 => ack::NackMotive::UnknownChainName,
            3 => ack::NackMotive::DeprecatedP2pVersion,
            4 => ack::NackMotive::DeprecatedDistributedDbVersion,
            _ => ack::NackMotive::AlreadyConnected,
        };
        let mut remaining = self.gen_range(0..limits::NACK_PEERS_MAX_LENGTH);
        let mut peers: Vec<String> = Vec::new();
        
        loop {
            if remaining == 0 {
                break;
            }
            let size = self.gen_range(0..limits::P2P_POINT_MAX_LENGTH);
            let name = self.gen_string(size);
            remaining -= 1;
            peers.push(name);
        }

        ack::NackInfo::new(motive, &peers)
    }
}

impl Generator<ack::AckMessage> for RandomState {
    fn gen(&mut self) -> ack::AckMessage {
        match self.gen_range(0..3) {
            0 => ack::AckMessage::Ack,
            1 => ack::AckMessage::NackV0,
            _ => ack::AckMessage::Nack(self.gen()),
        }
    }
}
