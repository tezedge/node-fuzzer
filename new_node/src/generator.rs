use std::rc::Rc;
use std::cell::RefCell;
use rand::rngs::SmallRng;
use rand::{Rng, SeedableRng};
use rand::distributions::{
    Standard,
    Distribution,
    uniform::{SampleRange, SampleUniform},
};
use std::{iter, net};

use tezos_messages::p2p::binary_message::BinaryWrite;
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

use std::marker::PhantomData;

#[derive(Clone)]
struct RandomState<T> {
    rng: Rc<RefCell<SmallRng>>,
    _marker: PhantomData<T>
}

impl<T> RandomState<T> {
    pub fn new(seed: [u8; 32]) -> Self {
        RandomState {
            rng:Rc::new(RefCell::new(SmallRng::from_seed(seed))),
            _marker: PhantomData
        }
    }

    pub fn from<T2>(rs: &RandomState<T2>) -> Self {
        RandomState {
            rng: Rc::clone(&rs.rng),
            _marker: PhantomData
        }
    }

    pub fn gen<T2>(&mut self) -> T2
    where Standard: Distribution<T2> {
        self.rng.borrow_mut().gen::<T2>()
    }

    pub fn gen_range<T2, R>(&mut self, range: R) -> T2 
    where
        T2: SampleUniform,
        R: SampleRange<T2> {
        self.rng.borrow_mut().gen_range::<T2, R>(range)
    }

    fn gen_vec<T2>(&mut self, size: usize) -> Vec<T2>
    where Standard: Distribution<T2> {
        (0..size).map(|_| { self.gen::<T2>() }).collect()
    }
    
    fn gen_string(&mut self, max_size_bytes: usize) -> String {
        let mut string: String = (0..max_size_bytes)
        .map(|_| { self.gen::<char>() }).collect();
        
        for i in (0..max_size_bytes).rev() {
            if string.is_char_boundary(i) {
                string.truncate(i);
                break;
            }
        }

        string
    }
}

pub trait Generator {
    type Item;
    fn make(&mut self) -> Self::Item;
}

pub struct IterWrap<T: Generator>(T);

pub fn iterable<T: Generator>(generator: T) -> IterWrap<T> {
    IterWrap::<T>{ 0: generator }
}

impl<T: Generator> Iterator for IterWrap<T> {
    type Item = T::Item;
    fn next(&mut self) -> Option<Self::Item> {
        Some(self.0.make())
    }
}

type IpAddr = RandomState<net::IpAddr>;
impl Generator for IpAddr {
    type Item = net::IpAddr;
    fn make(&mut self) -> Self::Item {
        let [a, b, c, d] = self.gen::<[u8;4]>();
        Self::Item::V4(net::Ipv4Addr::new(a, b, c, d))
    }
}

type SocketAddr = RandomState<net::SocketAddr>;
impl Generator for SocketAddr {
    type Item = net::SocketAddr;
    fn make(&mut self) -> Self::Item {
        Self::Item::new(IpAddr::from(self).make(), self.gen::<u16>())
    }
}

type AdvertiseMessage = RandomState<advertise::AdvertiseMessage>;
impl Generator for AdvertiseMessage {
    type Item = advertise::AdvertiseMessage;
    fn make(&mut self) -> Self::Item {
        let size = self.gen_range(1..limits::ADVERTISE_ID_LIST_MAX_LENGTH);
        let addresses = iterable(SocketAddr::from(self));
        Self::Item::new(
             &addresses.take(size).collect::<Vec<net::SocketAddr>>()
        )
    }
}

struct Hash<T: HashTrait>(RandomState<T>);
impl<T: HashTrait> Generator for Hash<T> {
    type Item = T;
    fn make(&mut self) -> Self::Item {
        let bytes = self.0.gen_vec::<u8>(T::hash_type().size());
        Self::Item::try_from_bytes(&bytes[..]).unwrap()
    }
}

type SwapMessage = RandomState<swap::SwapMessage>;
impl Generator for SwapMessage {
    type Item = swap::SwapMessage;
    fn make(&mut self) -> Self::Item {
        //let point = "127.0.0.1:12345".to_string();
        let size_bytes = self.gen_range(1..limits::P2P_POINT_MAX_LENGTH);
        let pkh = Hash::<crypto::hash::CryptoboxPublicKeyHash> {
            0: RandomState::from(self)
        }.make();
        Self::Item::new(self.gen_string(size_bytes), pkh)
    }
}

type GetCurrentBranchMessage = RandomState<current_branch::GetCurrentBranchMessage>;
impl Generator for GetCurrentBranchMessage {
    type Item = current_branch::GetCurrentBranchMessage;
    fn make(&mut self) -> Self::Item {
        let chain_id = Hash::<crypto::hash::ChainId> {
            0: RandomState::from(self)
        }.make();
        Self::Item::new(chain_id)
    }
}

type BlockHeader = RandomState<block_header::BlockHeader>;
impl Generator for BlockHeader {
    type Item = block_header::BlockHeader;
    fn make(&mut self) -> Self::Item {
        let max_size = limits::BLOCK_HEADER_MAX_SIZE;
        let fitness_len: usize = self.gen_range(1..max_size/0x1000/2);
        let fitness: Vec<usize> = (1..fitness_len)
        .map(|_| {self.gen_range(1..0x1000)} ).collect();
        let data_len = self.gen_range(1..0x1000);

        block_header::BlockHeaderBuilder::default()
        .level(self.gen())
        .proto(self.gen())
        .predecessor(
            Hash::<crypto::hash::BlockHash> { 0: RandomState::from(self) }
            .make()
        )
        .timestamp(self.gen())
        .validation_pass(self.gen())
        .operations_hash(
            Hash::<crypto::hash::OperationListListHash> {
                0: RandomState::from(self)
            }.make()
        )
        .fitness(fitness.iter()
            .map(|x| { self.gen_vec::<u8>(*x) }).collect())
        .context(
            Hash::<crypto::hash::ContextHash> { 0: RandomState::from(self) }
            .make()
        )
        .protocol_data(self.gen_vec::<u8>(data_len))
        .build().unwrap()
    }
}

type CurrentBranch = RandomState<current_branch::CurrentBranch>;
impl Generator for CurrentBranch {
    type Item = current_branch::CurrentBranch;
    fn make(&mut self) -> Self::Item {
        let block_hash = Hash::<crypto::hash::BlockHash> {
            0: RandomState::from(self)
        };
        let history_len = limits::CURRENT_BRANCH_HISTORY_MAX_LENGTH;
        Self::Item::new(
            BlockHeader::from(self).make(),
            iterable(block_hash)
            .take(self.gen_range(1..history_len)).collect()
        )
    }
}

type CurrentBranchMessage = RandomState<current_branch::CurrentBranchMessage>;
impl Generator for CurrentBranchMessage {
    type Item = current_branch::CurrentBranchMessage;
    fn make(&mut self) -> Self::Item {
        let chain_id = Hash::<crypto::hash::ChainId> {
            0: RandomState::from(self)
        }.make();

        Self::Item::new(chain_id, CurrentBranch::from(self).make())
    }
}

type DeactivateMessage = RandomState<deactivate::DeactivateMessage>;
impl Generator for DeactivateMessage {
    type Item = deactivate::DeactivateMessage;
    fn make(&mut self) -> Self::Item {
        let chain_id = Hash::<crypto::hash::ChainId> {
            0: RandomState::from(self)
        }.make();
        Self::Item::new(chain_id)
    }
}

type GetCurrentHeadMessage = RandomState<current_head::GetCurrentHeadMessage>;
impl Generator for GetCurrentHeadMessage {
    type Item = current_head::GetCurrentHeadMessage;
    fn make(&mut self) -> Self::Item {
        let chain_id = Hash::<crypto::hash::ChainId> {
            0: RandomState::from(self)
        }.make();
        Self::Item::new(chain_id)
    }
}

type Mempool = RandomState<mempool::Mempool>;
impl Generator for Mempool {
    type Item = mempool::Mempool;
    fn make(&mut self) -> Self::Item {
        let ops = self.gen_range(1..limits::MEMPOOL_MAX_OPERATIONS);
        let ops_split = self.gen_range(0..ops);
        let op_hash = Hash::<crypto::hash::OperationHash> {
            0: RandomState::from(self)
        };
        let hashes: Vec<crypto::hash::OperationHash> = iterable(op_hash)
        .take(ops).collect();
        Self::Item::new(
            hashes[..ops_split].to_vec(),
            hashes[ops_split..ops].to_vec()
        )
    }
}

type CurrentHeadMessage = RandomState<current_head::CurrentHeadMessage>;
impl Generator for CurrentHeadMessage {
    type Item = current_head::CurrentHeadMessage;
    fn make(&mut self) -> Self::Item {
        let chain_id = Hash::<crypto::hash::ChainId> {
            0: RandomState::from(self)
        }.make();
        Self::Item::new(
            chain_id,
            BlockHeader::from(self).make(),
            Mempool::from(self).make()
        )
    }
}

type GetBlockHeadersMessage = RandomState<block_header::GetBlockHeadersMessage>;
impl Generator for GetBlockHeadersMessage {
    type Item = block_header::GetBlockHeadersMessage;
    fn make(&mut self) -> Self::Item {
        let max_elements = limits::GET_BLOCK_HEADERS_MAX_LENGTH;
        let block_hash = Hash::<crypto::hash::BlockHash> {
            0: RandomState::from(self)
        };
        Self::Item::new(
            iterable(block_hash)
            .take(self.gen_range(1..max_elements)).collect()
        )
    }
}

type BlockHeaderMessage = RandomState<block_header::BlockHeaderMessage>;
impl Generator for BlockHeaderMessage {
    type Item = block_header::BlockHeaderMessage;
    fn make(&mut self) -> Self::Item {
        Self::Item::from(BlockHeader::from(self).make())
    }
}

type GetOperationsMessage = RandomState<operation::GetOperationsMessage>;
impl Generator for GetOperationsMessage {
    type Item = operation::GetOperationsMessage;
    fn make(&mut self) -> Self::Item {
        let max_elements = limits::GET_OPERATIONS_MAX_LENGTH;
        let operation_hash = Hash::<crypto::hash::OperationHash> {
            0: RandomState::from(self)
        };
        Self::Item::new(
            iterable(operation_hash)
            .take(self.gen_range(1..max_elements)).collect()
        )
    }
}


type Operation = RandomState<operation::Operation>;
impl Generator for Operation {
    type Item = operation::Operation;
    fn make(&mut self) -> Self::Item {
        let max_size = limits::OPERATION_MAX_SIZE;
        let max_data_size = max_size - crypto::hash::BlockHash::hash_size();
        let data_size = self.gen_range(1..max_data_size);
        let block_hash = Hash::<crypto::hash::BlockHash> {
            0: RandomState::from(self)
        }.make();
        Self::Item::new(block_hash, self.gen_vec::<u8>(data_size))
    }
}

type OperationMessage = RandomState<operation::OperationMessage>;
impl Generator for OperationMessage {
    type Item = operation::OperationMessage;
    fn make(&mut self) -> Self::Item {
        Self::Item::from(Operation::from(self).make())
    }
}

type GetProtocolsMessage = RandomState<protocol::GetProtocolsMessage>;
impl Generator for GetProtocolsMessage {
    type Item = protocol::GetProtocolsMessage;
    fn make(&mut self) -> Self::Item {
        let max_elements = limits::GET_PROTOCOLS_MAX_LENGTH;
        let protocol_hash = Hash::<crypto::hash::ProtocolHash> {
            0: RandomState::from(self)
        };
        Self::Item::new(
            iterable(protocol_hash)
            .take(self.gen_range(1..max_elements)).collect()
        )
    }
}

type Component = RandomState<protocol::Component>;
impl Generator for Component {
    type Item = protocol::Component;
    fn make(&mut self) -> Self::Item {
        let mut remaining_size = limits::PROTOCOL_COMPONENT_MAX_SIZE;
        let name_size = self.gen_range(0..remaining_size);
        let name = self.gen_string(name_size);
        remaining_size -= name.len();
        let interface = match self.gen::<bool>() {
            true => {
                let interface_size = self.gen_range(0..remaining_size);
                let iface = self.gen_string(interface_size);
                remaining_size -= iface.len();
                Some(iface)
            },
            false => None,
        };
        let implementation_size = self.gen_range(0..remaining_size);
        let implementation = self.gen_string(implementation_size);
        Self::Item::new(name, interface, implementation)
    }
}

type Protocol = RandomState<protocol::Protocol>;
impl Generator for Protocol {
    type Item = protocol::Protocol;
    fn make(&mut self) -> Self::Item {
        let mut components: Vec<protocol::Component> = Vec::new();
        let mut remaining_size = limits::PROTOCOL_COMPONENT_MAX_SIZE;

        for component in iterable(Component::from(self)) {
            let component_size = component.as_bytes().unwrap().len();
            
            if component_size > remaining_size {
                break;
            }
            
            remaining_size -= component_size;
            components.push(component);
        }

        Self::Item::new(self.gen(), components)
    }
}


type OperationsForBlock = RandomState<operations_for_blocks::OperationsForBlock>;
impl Generator for OperationsForBlock {
    type Item = operations_for_blocks::OperationsForBlock;
    fn make(&mut self) -> Self::Item {
        let block_hash = Hash::<crypto::hash::BlockHash> {
            0: RandomState::from(self)
        }.make();
        Self::Item::new(block_hash, self.gen())
    }
}

type GetOperationsForBlocksMessage = RandomState<operations_for_blocks::GetOperationsForBlocksMessage>;
impl Generator for GetOperationsForBlocksMessage {
    type Item = operations_for_blocks::GetOperationsForBlocksMessage;
    fn make(&mut self) -> Self::Item {
        let max_ops = limits::GET_OPERATIONS_FOR_BLOCKS_MAX_LENGTH;
        let operations = iterable(OperationsForBlock::from(self));
        Self::Item::new(operations.take(self.gen_range(0..max_ops)).collect())
    }
}

type ProtocolMessage = RandomState<protocol::ProtocolMessage>;
impl Generator for ProtocolMessage {
    type Item = protocol::ProtocolMessage;
    fn make(&mut self) -> Self::Item {
        Self::Item::new(Protocol::from(self).make())
    }
}

type PathItem = RandomState<operations_for_blocks::PathItem>;
impl Generator for PathItem {
    type Item = operations_for_blocks::PathItem;
    fn make(&mut self) -> Self::Item {
        let size = self.gen_range(0..0x1000);
        let hash = self.gen_vec::<u8>(size);
        
        match self.gen::<bool>() {
            true => Self::Item::right(hash),
            false => Self::Item::left(hash),
        }
    }
}

type Path = RandomState<operations_for_blocks::Path>;
impl Generator for Path {
    type Item = operations_for_blocks::Path;
    fn make(&mut self) -> Self::Item {
        let max_depth = operations_for_blocks::MAX_PASS_MERKLE_DEPTH.unwrap();
        let items = iterable(PathItem::from(self));        
        Self::Item::new(items.take(self.gen_range(0..max_depth)).collect())
    }
}

type OperationsForBlocksMessage = RandomState<operations_for_blocks::OperationsForBlocksMessage>;
impl Generator for OperationsForBlocksMessage {
    type Item = operations_for_blocks::OperationsForBlocksMessage;
    fn make(&mut self) -> Self::Item {
        let mut remaining_size = limits::OPERATION_LIST_MAX_SIZE;
        let mut operations = Vec::<operation::Operation>::new(); 

        for operation in iterable(Operation::from(self)) {
            let operation_size = operation.as_bytes().unwrap().len();
            
            if operation_size > remaining_size {
                break;
            }
            
            remaining_size -= operation_size;
            operations.push(operation);
        }

        Self::Item::new(
            OperationsForBlock::from(self).make(),
            Path::from(self).make(),
            operations
        )
    }
}

struct PeerAdvertise(RandomState<peer::PeerMessage>);
impl Generator for PeerAdvertise {
    type Item = peer::PeerMessage;
    fn make(&mut self) -> Self::Item {
        Self::Item::Advertise(AdvertiseMessage::from(&self.0).make())
    }
}
struct PeerSwapRequest(RandomState<peer::PeerMessage>);
impl Generator for PeerSwapRequest {
    type Item = peer::PeerMessage;
    fn make(&mut self) -> Self::Item {
        Self::Item::SwapRequest(SwapMessage::from(&self.0).make())
    }
}
struct PeerSwapAck(RandomState<peer::PeerMessage>);
impl Generator for PeerSwapAck {
    type Item = peer::PeerMessage;
    fn make(&mut self) -> Self::Item {
        Self::Item::SwapAck(SwapMessage::from(&self.0).make())
    }
}
struct PeerGetCurrentBranch(RandomState<peer::PeerMessage>);
impl Generator for PeerGetCurrentBranch {
    type Item = peer::PeerMessage;
    fn make(&mut self) -> Self::Item {
        Self::Item::GetCurrentBranch(
            GetCurrentBranchMessage::from(&self.0).make()
        )
    }
}

struct PeerCurrentBranch(RandomState<peer::PeerMessage>);
impl Generator for PeerCurrentBranch {
    type Item = peer::PeerMessage;
    fn make(&mut self) -> Self::Item {
        Self::Item::CurrentBranch(
            CurrentBranchMessage::from(&self.0).make()
        )
    }
}
struct PeerDeactivate(RandomState<peer::PeerMessage>);
impl Generator for PeerDeactivate {
    type Item = peer::PeerMessage;
    fn make(&mut self) -> Self::Item {
        Self::Item::Deactivate(
            DeactivateMessage::from(&self.0).make()
        )
    }
}
struct PeerGetCurrentHead(RandomState<peer::PeerMessage>);
impl Generator for PeerGetCurrentHead {
    type Item = peer::PeerMessage;
    fn make(&mut self) -> Self::Item {
        Self::Item::GetCurrentHead(
            GetCurrentHeadMessage::from(&self.0).make()
        )
    }
}
struct PeerCurrentHead(RandomState<peer::PeerMessage>);
impl Generator for PeerCurrentHead {
    type Item = peer::PeerMessage;
    fn make(&mut self) -> Self::Item {
        Self::Item::CurrentHead(
            CurrentHeadMessage::from(&self.0).make()
        )
    }
}

struct PeerGetBlockHeaders(RandomState<peer::PeerMessage>);
impl Generator for PeerGetBlockHeaders {
    type Item = peer::PeerMessage;
    fn make(&mut self) -> Self::Item {
        Self::Item::GetBlockHeaders(
            GetBlockHeadersMessage::from(&self.0).make()
        )
    }
}

struct PeerBlockHeader(RandomState<peer::PeerMessage>);
impl Generator for PeerBlockHeader {
    type Item = peer::PeerMessage;
    fn make(&mut self) -> Self::Item {
        Self::Item::BlockHeader(
            BlockHeaderMessage::from(&self.0).make()
        )
    }
}

struct PeerGetOperations(RandomState<peer::PeerMessage>);
impl Generator for PeerGetOperations {
    type Item = peer::PeerMessage;
    fn make(&mut self) -> Self::Item {
        Self::Item::GetOperations(
            GetOperationsMessage::from(&self.0).make()
        )
    }
}

struct PeerOperation(RandomState<peer::PeerMessage>);
impl Generator for PeerOperation {
    type Item = peer::PeerMessage;
    fn make(&mut self) -> Self::Item {
        Self::Item::Operation(
            OperationMessage::from(&self.0).make()
        )
    }
}

struct PeerGetProtocols(RandomState<peer::PeerMessage>);
impl Generator for PeerGetProtocols {
    type Item = peer::PeerMessage;
    fn make(&mut self) -> Self::Item {
        Self::Item::GetProtocols(
            GetProtocolsMessage::from(&self.0).make()
        )
    }
}
struct PeerProtocol(RandomState<peer::PeerMessage>);
impl Generator for PeerProtocol {
    type Item = peer::PeerMessage;
    fn make(&mut self) -> Self::Item {
        Self::Item::Protocol(
            ProtocolMessage::from(&self.0).make()
        )
    }
}

struct PeerGetOperationsForBlocks(RandomState<peer::PeerMessage>);
impl Generator for PeerGetOperationsForBlocks {
    type Item = peer::PeerMessage;
    fn make(&mut self) -> Self::Item {
        Self::Item::GetOperationsForBlocks(
            GetOperationsForBlocksMessage::from(&self.0).make()
        )
    }
}

struct PeerOperationsForBlocks(RandomState<peer::PeerMessage>);
impl Generator for PeerOperationsForBlocks {
    type Item = peer::PeerMessage;
    fn make(&mut self) -> Self::Item {
        Self::Item::OperationsForBlocks(
            OperationsForBlocksMessage::from(&self.0).make()
        )
    }
}


#[derive(Clone)]
pub struct Message(RandomState<peer::PeerMessage>);
impl Message {
    pub fn new(seed: [u8; 32]) -> Self {
        Message { 0: RandomState::<peer::PeerMessage>::new(seed) }
    }
}

impl Generator for Message {
    type Item = peer::PeerMessage;
    fn make(&mut self) -> Self::Item {
        let n = self.0.gen_range(0..16);
        match n {
            0 => PeerAdvertise{ 0: RandomState::from(&self.0) }.make(),
            1 => PeerSwapRequest{ 0: RandomState::from(&self.0) }.make(),
            2 => PeerSwapAck{ 0: RandomState::from(&self.0) }.make(),
            3 => PeerGetCurrentBranch{ 0: RandomState::from(&self.0) }.make(),
            4 => PeerCurrentBranch{ 0: RandomState::from(&self.0) }.make(),
            5 => PeerDeactivate{ 0: RandomState::from(&self.0) }.make(),
            6 => PeerGetCurrentHead{ 0: RandomState::from(&self.0) }.make(),
            7 => PeerCurrentHead{ 0: RandomState::from(&self.0) }.make(),
            8 => PeerGetBlockHeaders{ 0: RandomState::from(&self.0) }.make(),
            9 => PeerBlockHeader{ 0: RandomState::from(&self.0) }.make(),
            10 => PeerGetOperations{ 0: RandomState::from(&self.0) }.make(),
            11 => PeerOperation{ 0: RandomState::from(&self.0) }.make(),
            12 => PeerGetProtocols{ 0: RandomState::from(&self.0) }.make(),
            13 => PeerProtocol{ 0: RandomState::from(&self.0) }.make(),
            14 => PeerGetOperationsForBlocks{ 0: RandomState::from(&self.0) }.make(),
            _ => PeerOperationsForBlocks{ 0: RandomState::from(&self.0) }.make(),

        }
    }
}
