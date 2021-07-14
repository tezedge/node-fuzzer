use std::rc::Rc;
use std::cell::RefCell;
use std::collections::HashMap;
use std::str::FromStr;
use rand::rngs::SmallRng;
use rand::{Rng, SeedableRng};
use rand::distributions::{
    Standard,
    Distribution,
    uniform::{SampleRange, SampleUniform},
};
use std::{iter, net};

use tezos_messages::p2p::binary_message::{BinaryWrite, MessageHash};
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
}; 

use crypto::{
    crypto_box::{CryptoKey, PublicKey, SecretKey},
    hash::*,
    proof_of_work::ProofOfWork,
};

type BlocksMap = HashMap<crypto::hash::BlockHash, block_header::BlockHeader>;

#[derive(Clone)]
pub struct _RandomState {
    rng: SmallRng,
    messages: Vec<String>,
    pub blocks: BlocksMap,
    last_block: crypto::hash::BlockHash,
    chain_id: ChainId,
}

impl _RandomState {
    pub fn new(seed: u64, messages: Vec<String>) -> Self {
        _RandomState {
            rng: SmallRng::seed_from_u64(seed),
            messages: messages,
            blocks: BlocksMap::new(),
            last_block: BlockHash::from_base58_check(
                "BLockGenesisGenesisGenesisGenesisGenesisf79b5d1CoW2"
            ).unwrap(),
            chain_id: ChainId::from_str("NetXdQprcVkpaWU").unwrap(),          
        }
    }
}

#[derive(Clone)]
pub struct RandomState(Rc<RefCell<_RandomState>>);

impl RandomState {
    pub fn new(seed: u64, messages: Vec<String>) -> Self {
        RandomState {
            0: Rc::new(RefCell::new(_RandomState::new(seed, messages)))
        }
    }

    pub fn gen_range<T, R>(&mut self, range: R) -> T 
    where
        T: SampleUniform,
        R: SampleRange<T> {
        self.0.borrow_mut().rng.gen_range::<T, R>(range)
    }

    pub fn gen_t<T>(&mut self) -> T where Standard: Distribution<T> {
        self.0.borrow_mut().rng.gen::<T>()
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
        self.0.borrow_mut().chain_id.clone()
    }

    pub fn set_chain_id(&mut self, chain_id: ChainId) {
        self.0.borrow_mut().chain_id = chain_id
    }

    fn block_hash(bh: &block_header::BlockHeader) -> crypto::hash::BlockHash {
        let hash = bh.message_hash().unwrap();
        crypto::hash::BlockHash::try_from_bytes(&hash[..]).unwrap()
    }

    pub fn state(&mut self) -> std::cell::RefMut<_RandomState> {
        self.0.borrow_mut()
    }

    fn set_last_block_hash(&mut self, hash: crypto::hash::BlockHash) {
        self.state().last_block = hash;
    }
}

pub trait Generator<T> {
    fn gen(&mut self) -> T;
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
        let peer_id: HashKind<crypto::hash::CryptoboxPublicKeyHash> = self.gen();
        swap::SwapMessage::new(self.gen_string(size_bytes), peer_id.0)
    }
}

impl Generator<current_branch::GetCurrentBranchMessage> for RandomState {
    fn gen(&mut self) -> current_branch::GetCurrentBranchMessage {
        let chain_id: HashKind<crypto::hash::ChainId> = self.gen();
        current_branch::GetCurrentBranchMessage::new(chain_id.0)
    }
}

impl Generator<block_header::BlockHeaderBuilder> for RandomState {
    fn gen(&mut self) -> block_header::BlockHeaderBuilder {
        let max_size = limits::BLOCK_HEADER_MAX_SIZE;
        let mut remaining: usize = max_size - 0x1000;
        let mut fitness: Vec<Vec<u8>> = Vec::new();
        
        loop {
            let element_size = self.gen_range(0..max_size);
            let element = self.gen_vec::<u8>(element_size);

            if element_size > remaining {
                break;
            }

            fitness.push(element);
            remaining -= element_size;
        }

        let data_len = self.gen_range(1..remaining);
        let predecessor: HashKind<crypto::hash::BlockHash> = self.gen();
        let operations_hash: HashKind<crypto::hash::OperationListListHash> = self.gen();
        let context: HashKind<crypto::hash::ContextHash> = self.gen();

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

impl Generator<current_branch::CurrentBranch> for RandomState {
    fn gen(&mut self) -> current_branch::CurrentBranch {
        /* Current OOM bug is triggered when there is no history */
        let history_len = 0; //self.gen_range(0..limits::CURRENT_BRANCH_HISTORY_MAX_LENGTH);
        let history: Vec<crypto::hash::BlockHash> = (0..history_len).map(|_|{
            let HashKind::<crypto::hash::BlockHash>{0: block_hash} = self.gen();
            block_hash
        }).collect();
        
        let mut blockb: block_header::BlockHeaderBuilder = self.gen();
        let predecessor = self.state().last_block.clone();
        let new_block = blockb.predecessor(predecessor).build().unwrap();
        let block_hash = Self::block_hash(&new_block);
        eprintln!(
            "New block hash {}, predecessor {}",
            block_hash.clone().to_base58_check(),
            new_block.predecessor().to_base58_check()
        );
        /* FIXME: add upper boundary for blocks map */
        self.state().blocks.insert(block_hash.clone(), new_block.clone());
        self.set_last_block_hash(block_hash);
        current_branch::CurrentBranch::new(new_block, history)
    }
}

impl Generator<current_branch::CurrentBranchMessage> for RandomState {
    fn gen(&mut self) -> current_branch::CurrentBranchMessage {
        current_branch::CurrentBranchMessage::new(
            self.get_chain_id(), self.gen()
        )
    }
}

impl Generator<deactivate::DeactivateMessage> for RandomState {
    fn gen(&mut self) -> deactivate::DeactivateMessage {
        // TODO: randomly use self.chain_id() too?
        let chain_id: HashKind<crypto::hash::ChainId> = self.gen();
        deactivate::DeactivateMessage::new(chain_id.0)
    }
}

impl Generator<current_head::GetCurrentHeadMessage> for RandomState {
    fn gen(&mut self) -> current_head::GetCurrentHeadMessage {
        // TODO: ask about existing chains?
        let chain_id: HashKind<crypto::hash::ChainId> = self.gen();
        current_head::GetCurrentHeadMessage::new(chain_id.0)
    }
}

impl Generator<mempool::Mempool> for RandomState {
    fn gen(&mut self) -> mempool::Mempool {
        let max_count = limits::MEMPOOL_MAX_OPERATIONS;
        let pending_num = self.gen_range(1..max_count);
        let known_valid_num = self.gen_range(0..max_count-pending_num);
        let pending: Vec<crypto::hash::OperationHash> = (0..pending_num).map(|_|{
            let HashKind::<crypto::hash::OperationHash>{0: op_hash} = self.gen();
            op_hash
        }).collect();
        let known_valid: Vec<crypto::hash::OperationHash> = (0..known_valid_num).map(|_|{
            let HashKind::<crypto::hash::OperationHash>{0: op_hash} = self.gen();
            op_hash
        }).collect();
        mempool::Mempool::new(known_valid, pending)
    }
}

impl Generator<current_head::CurrentHeadMessage> for RandomState {
    fn gen(&mut self) -> current_head::CurrentHeadMessage {
        current_head::CurrentHeadMessage::new(
            self.get_chain_id(),
            self.gen(),
            self.gen()
        )
    }
}

impl Generator<block_header::GetBlockHeadersMessage> for RandomState {
    fn gen(&mut self) -> block_header::GetBlockHeadersMessage {
        let count = self.gen_range(0..limits::GET_BLOCK_HEADERS_MAX_LENGTH);
        let block_hashes: Vec<crypto::hash::BlockHash> = (0..count).map(|_|{
            let HashKind::<crypto::hash::BlockHash>{0: block_hash} = self.gen();
            block_hash
        }).collect();
        block_header::GetBlockHeadersMessage::new(block_hashes)
    }
}

impl Generator<block_header::BlockHeaderMessage> for RandomState {
    fn gen(&mut self) -> block_header::BlockHeaderMessage {
        let block_header: block_header::BlockHeader = self.gen();
        block_header::BlockHeaderMessage::from(block_header)
    }
}

impl Generator<operation::GetOperationsMessage> for RandomState {
    fn gen(&mut self) -> operation::GetOperationsMessage {
        let count = self.gen_range(0..limits::GET_OPERATIONS_MAX_LENGTH);
        let operations: Vec<crypto::hash::OperationHash> = (0..count).map(|_|{
            let HashKind::<crypto::hash::OperationHash>{0: op_hash} = self.gen();
            op_hash
        }).collect();
        operation::GetOperationsMessage::new(operations)
    }
}

impl Generator<operation::Operation> for RandomState {
    fn gen(&mut self) -> operation::Operation {
        let max_size = limits::OPERATION_MAX_SIZE - crypto::hash::BlockHash::hash_size() - 0x20;
        let size = self.gen_range(0..max_size);
        let last_block = self.state().last_block.clone();      
        operation::Operation::new(
            last_block, // TODO: pick random block hashes?
            self.gen_vec(size)
        )
    }
}

impl Generator<operation::OperationMessage> for RandomState {
    fn gen(&mut self) -> operation::OperationMessage {
        let operation: operation::Operation = self.gen();
        operation::OperationMessage::from(operation)
    }
}

impl Generator<protocol::GetProtocolsMessage> for RandomState {
    fn gen(&mut self) -> protocol::GetProtocolsMessage {
        let count = self.gen_range(0..limits::GET_PROTOCOLS_MAX_LENGTH);
        let get_protocols: Vec<crypto::hash::ProtocolHash> = (0..count).map(|_|{
            let HashKind::<crypto::hash::ProtocolHash>{0: protocol_hash} = self.gen();
            protocol_hash
        }).collect();
        protocol::GetProtocolsMessage::new(get_protocols)
    }
}

impl Generator<protocol::Component> for RandomState {
    fn gen(&mut self) -> protocol::Component {
        let mut remaining = limits::PROTOCOL_COMPONENT_MAX_SIZE;
        let size = self.gen_range(0..remaining);
        let name = self.gen_string(size);
        remaining -= name.len();
        let interface = match self.gen_t::<bool>() {
            true => {
                let size = self.gen_range(0..remaining);
                let iface = self.gen_string(size);
                remaining -= iface.len();
                Some(iface)
            },
            false => None,
        };
        let size = self.gen_range(0..remaining);
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

impl Generator<operations_for_blocks::OperationsForBlock> for RandomState {
    fn gen(&mut self) -> operations_for_blocks::OperationsForBlock {
        let block_hash: HashKind<crypto::hash::BlockHash> = self.gen();
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
        protocol::ProtocolMessage::new(self.gen())
    }
}

impl Generator<operations_for_blocks::PathItem> for RandomState {
    fn gen(&mut self) -> operations_for_blocks::PathItem {
        /* FIXME: random sizes serialize but fail to deserialize */
        let size = 32; // self.gen_range(0..0x1000);
        let hash = self.gen_vec::<u8>(size);
        match self.gen_t::<bool>() {
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
        operations_for_blocks::OperationsForBlocksMessage::new(
            self.gen(), // TODO: existing hash
            self.gen(),
            self.gen()
        )
    }
}

impl Generator<peer::PeerMessage> for RandomState {
    fn gen(&mut self) -> peer::PeerMessage {
        let messages = self.state().messages.clone();
        let index = self.gen_range(0..messages.len());

        match &messages.get(index).unwrap()[..] {
            "Advertise" => peer::PeerMessage::Advertise(self.gen()),
            "SwapRequest" => peer::PeerMessage::SwapRequest(self.gen()),
            "SwapAck" => peer::PeerMessage::SwapAck(self.gen()),
            "GetCurrentBranch" => peer::PeerMessage::GetCurrentBranch(self.gen()),
            "CurrentBranch" => peer::PeerMessage::CurrentBranch(self.gen()),
            "Deactivate" => peer::PeerMessage::Deactivate(self.gen()),
            "GetCurrentHead" => peer::PeerMessage::GetCurrentHead(self.gen()),
            "CurrentHead" => peer::PeerMessage::CurrentHead(self.gen()),
            "GetBlockHeaders" => peer::PeerMessage::GetBlockHeaders(self.gen()),
            "BlockHeader" => peer::PeerMessage::BlockHeader(self.gen()),
            "GetOperations" => peer::PeerMessage::GetOperations(self.gen()),
            "Operation" => peer::PeerMessage::Operation(self.gen()),
            "GetProtocols" => peer::PeerMessage::GetProtocols(self.gen()),
            "Protocol" => peer::PeerMessage::Protocol(self.gen()),
            "GetOperationsForBlocks" => peer::PeerMessage::GetOperationsForBlocks(self.gen()),
            _ => peer::PeerMessage::OperationsForBlocks(self.gen())
        }
    }
}
