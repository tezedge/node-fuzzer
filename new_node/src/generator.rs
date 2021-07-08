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

use tezos_messages::p2p::encoding::{
    limits,
    advertise,
    swap,
    current_branch,
    block_header,
    peer
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
        let size = self.gen_range(0..limits::ADVERTISE_ID_LIST_MAX_LENGTH);
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
        let size_bytes = self.gen_range(0..limits::P2P_POINT_MAX_LENGTH);
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
        let fitness_len: usize = self.gen_range(0..max_size/0x1000/2);
        let fitness: Vec<usize> = (0..fitness_len)
        .map(|_| {self.gen_range(0..0x1000)} ).collect();
        let data_len = self.gen_range(0..0x1000);

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
            .take(self.gen_range(0..history_len)).collect()
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
        let n = self.0.gen_range(0..5);
        match n {
            0 => PeerAdvertise{ 0: RandomState::from(&self.0) }.make(),
            1 => PeerSwapRequest{ 0: RandomState::from(&self.0) }.make(),
            2 => PeerSwapAck{ 0: RandomState::from(&self.0) }.make(),
            3 => PeerGetCurrentBranch{ 0: RandomState::from(&self.0) }.make(),
            _ => PeerCurrentBranch{ 0: RandomState::from(&self.0) }.make(),
        }
    }
}
