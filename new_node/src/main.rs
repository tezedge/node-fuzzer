use std::collections::HashSet;
use std::iter::FromIterator;
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::rc::Rc;
use std::cell::RefCell;

use rand::distributions::Standard;
use rand::rngs::SmallRng;
use rand::{Rng, SeedableRng};

use slog::Drain;

use tezedge_state::DefaultEffects;
use tezedge_state::ShellCompatibilityVersion;
use tezos_messages::p2p::encoding::peer::PeerMessageResponse;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use tezos_messages::p2p::encoding::{
    advertise::AdvertiseMessage,
    swap::SwapMessage,
    peer::PeerMessage,
    limits};

use tla_sm::Acceptor;

use crypto::{
    crypto_box::{CryptoKey, PublicKey, SecretKey},
    hash::{CryptoboxPublicKeyHash, HashTrait, HashType},
    proof_of_work::ProofOfWork,
};
use hex::FromHex;
use tezedge_state::proposals::ExtendPotentialPeersProposal;
use tezedge_state::{PeerAddress, TezedgeConfig, TezedgeState};
use tezos_identity::Identity;

use tezedge_state::proposer::mio_manager::{MioEvents, MioManager};
use tezedge_state::proposer::{Notification, Peer, TezedgeProposer, TezedgeProposerConfig};

fn shell_compatibility_version() -> ShellCompatibilityVersion {
    ShellCompatibilityVersion::new("TEZOS_MAINNET".to_owned(), vec![0], vec![0, 1])
}

fn identity(pkh: &[u8], pk: &[u8], sk: &[u8], pow: &[u8]) -> Identity {
    Identity {
        peer_id: CryptoboxPublicKeyHash::try_from_bytes(pkh).unwrap(),
        public_key: PublicKey::from_bytes(pk).unwrap(),
        secret_key: SecretKey::from_bytes(sk).unwrap(),
        proof_of_work_stamp: ProofOfWork::from_hex(hex::encode(pow)).unwrap(),
    }
}

fn identity_1() -> Identity {
    identity(
        &[
            86, 205, 231, 178, 152, 146, 2, 157, 213, 131, 90, 117, 83, 132, 177, 84,
        ],
        &[
            148, 73, 141, 148, 22, 20, 15, 188, 69, 132, 149, 51, 61, 170, 193, 180, 200, 126, 65,
            159, 87, 38, 113, 122, 84, 249, 182, 198, 116, 118, 174, 28,
        ],
        &[
            172, 122, 207, 58, 254, 215, 99, 123, 225, 15, 143, 199, 106, 46, 182, 179, 53, 156,
            120, 173, 177, 216, 19, 180, 28, 186, 179, 250, 233, 84, 244, 177,
        ],
        &[
            187, 194, 48, 1, 73, 36, 158, 28, 204, 132, 165, 67, 98, 35, 108, 60, 187, 194, 204,
            47, 251, 211, 182, 234,
        ],
    )
}

const SERVER_PORT: u16 = 13632;

fn logger(level: slog::Level) -> slog::Logger {
    let drain = Arc::new(
        slog_async::Async::new(
            slog_term::FullFormat::new(slog_term::TermDecorator::new().build())
                .build()
                .fuse(),
        )
        .chan_size(32768)
        .overflow_strategy(slog_async::OverflowStrategy::Block)
        .build(),
    );

    slog::Logger::root(drain.filter_level(level).fuse(), slog::o!())
}

fn build_tezedge_state() -> TezedgeState {
    // println!("generating identity...");
    // let node_identity = Identity::generate(ProofOfWork::DEFAULT_TARGET).unwrap();
    // dbg!(&node_identity);
    // dbg!(node_identity.secret_key.as_ref().0);

    let node_identity = identity_1();

    // println!("identity generated!");
    let mut tezedge_state = TezedgeState::new(
        logger(slog::Level::Trace),
        TezedgeConfig {
            port: SERVER_PORT,
            disable_mempool: true,
            private_node: false,
            min_connected_peers: 500,
            max_connected_peers: 1000,
            max_pending_peers: 1000,
            max_potential_peers: 100000,
            periodic_react_interval: Duration::from_millis(250),
            peer_blacklist_duration: Duration::from_secs(30 * 60),
            peer_timeout: Duration::from_secs(8),
            pow_target: ProofOfWork::DEFAULT_TARGET,
        },
        node_identity.clone(),
        shell_compatibility_version(),
        Default::default(),
        Instant::now(),
    );

    let peer_addresses = HashSet::<_>::from_iter(
        [
            // Potential peers which state machine will try to connect to.
            vec!["127.0.0.1:9732"]
                .into_iter()
                .map(|x| x.parse().unwrap())
                .collect::<Vec<_>>(),
            // fake peers, just for testing.
            // (0..10000).map(|x| format!(
            //         "{}.{}.{}.{}:12345",
            //         (x / 256 / 256 / 256) % 256,
            //         (x / 256 / 256) % 256,
            //         (x / 256) % 256,
            //         x % 256,
            //     )).collect::<Vec<_>>(),
        ]
        .concat()
        .into_iter(),
    );

    let _ = tezedge_state.accept(ExtendPotentialPeersProposal {
        at: Instant::now(),
        peers: peer_addresses,
    });

    tezedge_state
}

// Two ways of generating IpV4 addresses: random or lineal (index)
enum IpV4Generator {
    Rng(Rc<RefCell<SmallRng>>),
    Index(u32),
}

impl IpV4Generator {
    pub fn from_index(index: u32) -> IpAddr {
        let [a, b, c, d] = index.to_le_bytes();
        IpAddr::V4(Ipv4Addr::new(a, b, c, d))
    }

    pub fn from_rng(rng: &Rc<RefCell<SmallRng>>) -> IpAddr {
        IpV4Generator::from_index(rng.borrow_mut().gen())
    }

    pub fn to_index(addr: Ipv4Addr) -> u32 {
        u32::from_le_bytes(addr.octets())
    }

    pub fn new_random(seed: [u8; 32]) -> Self {
        IpV4Generator::Rng(Rc::new(RefCell::new(SmallRng::from_seed(seed))))
    }
}

impl Iterator for IpV4Generator {
    type Item = IpAddr;

    fn next(&mut self) -> Option<Self::Item> {
        let ret = match self {
            IpV4Generator::Rng(rng) => IpV4Generator::from_rng(rng),
            IpV4Generator::Index(index) => {
                *index = index.wrapping_add(1);
                IpV4Generator::from_index(*index)
            },
        };
        Some(ret)
    }
}

enum PeerAddrGenerator {
    Rng(Rc<RefCell<SmallRng>>),
    Index(u64),
}

impl PeerAddrGenerator {
    pub fn from_index(index: u64) -> SocketAddr {
        let port_index = (index & 0xffff) as u16;
        let ip_index = ((index >> 16) & 0xffffffff) as u32;
        SocketAddr::new(IpV4Generator::from_index(ip_index), port_index)
    }

    pub fn from_rng(rng: &Rc<RefCell<SmallRng>>) -> SocketAddr {
        PeerAddrGenerator::from_index(rng.borrow_mut().gen())
    }

    pub fn new_random(seed: [u8; 32]) -> Self {
        PeerAddrGenerator::Rng(Rc::new(RefCell::new(SmallRng::from_seed(seed))))
    }
}

impl Iterator for PeerAddrGenerator {
    type Item = SocketAddr;

    fn next(&mut self) -> Option<Self::Item> {
        let ret = match self {
            PeerAddrGenerator::Rng(rng) => PeerAddrGenerator::from_rng(rng),
            PeerAddrGenerator::Index(index) => {
                *index = index.wrapping_add(1);
                PeerAddrGenerator::from_index(*index)
            },
        };
        Some(ret)
    }
}

enum AdvertiseMessageGenerator {
    Rng(Rc<RefCell<SmallRng>>),
    Index(u64),
}

impl AdvertiseMessageGenerator {
    pub fn from_index(index: u64) -> AdvertiseMessage {
        let max_size = limits::ADVERTISE_ID_LIST_MAX_LENGTH;
        let count: usize = std::cmp::min((index & 0xff) as usize, max_size);
        let vec: Vec<SocketAddr> = PeerAddrGenerator::Index(index >> 8).take(count).collect();
        AdvertiseMessage::new(&vec)
    }

    pub fn from_rng(rng: &Rc<RefCell<SmallRng>>) -> AdvertiseMessage {
        let max_size = limits::ADVERTISE_ID_LIST_MAX_LENGTH;
        let count = rng.borrow_mut().gen_range(1 .. max_size);
        let vec: Vec<SocketAddr> = PeerAddrGenerator::Rng(rng.clone()).take(count).collect();
        AdvertiseMessage::new(&vec)
    }

    pub fn new_random(seed: [u8; 32]) -> Self {
        AdvertiseMessageGenerator::Rng(Rc::new(RefCell::new(SmallRng::from_seed(seed))))
    }
}

impl Iterator for AdvertiseMessageGenerator {
    type Item = AdvertiseMessage;

    fn next(&mut self) -> Option<Self::Item> {
        let ret = match self {
            AdvertiseMessageGenerator::Rng(rng) => {
                AdvertiseMessageGenerator::from_rng(rng)
            },
            AdvertiseMessageGenerator::Index(index) => {
                *index = index.wrapping_add(1);
                AdvertiseMessageGenerator::from_index(*index)
            },
        };
        Some(ret)
    }
}


enum SwapMessageGenerator {
    Rng(Rc<RefCell<SmallRng>>),
    Index(u64),
}


fn random_bytes(rng: &Rc<RefCell<SmallRng>>, size: usize) -> Vec<u8> {
    (0..size).map(|_| { rng.borrow_mut().gen() }).collect()
}

fn random_string(rng: &Rc<RefCell<SmallRng>>, size: usize) -> String {
    let mut rng_ref = rng.borrow_mut();
    (0..size).map(|_| { rng_ref.gen::<char>() }).collect()
}

impl SwapMessageGenerator {
    pub fn from_index(index: u64) -> SwapMessage {
        panic!("SwapMessageGenerator::from_index not implemented!");
    }

    pub fn from_rng(rng: &Rc<RefCell<SmallRng>>) -> SwapMessage {
        let max_size = limits::P2P_POINT_MAX_SIZE - 5; // there is a BUG in the encoder
        let count = rng.borrow_mut().gen_range(0..max_size);
        let pkh_bytes = random_bytes(rng, HashType::CryptoboxPublicKeyHash.size());
        let mut point = random_string(rng, count);

        while point.len() >= count {
            point.pop();
        }

        //eprintln!("point len {}", point.len());
        //let point = "127.0.0.1:12345".to_string();
        SwapMessage::new(
            point,
            CryptoboxPublicKeyHash::try_from_bytes(&pkh_bytes[..]).unwrap()
        )
    }

    pub fn new_random(seed: [u8; 32]) -> Self {
        SwapMessageGenerator::Rng(Rc::new(RefCell::new(SmallRng::from_seed(seed))))
    }
}

impl Iterator for SwapMessageGenerator {
    type Item = SwapMessage;

    fn next(&mut self) -> Option<Self::Item> {
        let ret = match self {
            SwapMessageGenerator::Rng(rng) => {
                SwapMessageGenerator::from_rng(rng)
            },
            SwapMessageGenerator::Index(index) => {
                *index = index.wrapping_add(1);
                SwapMessageGenerator::from_index(*index)
            },
        };
        Some(ret)
    }
}

enum PeerMessageGenerator {
    Rng(Rc<RefCell<SmallRng>>),
    Index(u64),
}

impl PeerMessageGenerator {
    pub fn from_index(index: u64) -> PeerMessage {
        panic!("MessageGenerator::from_index not implemented!");
    }

    pub fn from_rng(rng: &Rc<RefCell<SmallRng>>) -> PeerMessage {
        let n = rng.borrow_mut().gen_range(0..2);

        match n {
             0 => PeerMessage::Advertise(AdvertiseMessageGenerator::from_rng(rng)),
             1 => PeerMessage::SwapRequest(SwapMessageGenerator::from_rng(rng)),
             _ => PeerMessage::SwapAck(SwapMessageGenerator::from_rng(rng)),
        }
    }

    pub fn new_random(seed: [u8; 32]) -> Self {
        PeerMessageGenerator::Rng(Rc::new(RefCell::new(SmallRng::from_seed(seed))))
    }
}

impl Iterator for PeerMessageGenerator {
    type Item = PeerMessage;

    fn next(&mut self) -> Option<Self::Item> {
        let ret = match self {
            PeerMessageGenerator::Rng(rng) => {
                PeerMessageGenerator::from_rng(rng)
            },
            PeerMessageGenerator::Index(index) => {
                *index = index.wrapping_add(1);
                PeerMessageGenerator::from_index(*index)
            },
        };
        Some(ret)
    }
}


struct BadNode {
    throughput: usize,
    generator: PeerMessageGenerator,
    proposer: TezedgeProposer<MioEvents, DefaultEffects, MioManager>,
}

impl BadNode {
    pub fn new() -> Self {
        BadNode {
            throughput: 0x100000, // TODO: fix BrokenPipe error
            generator: PeerMessageGenerator::new_random([1; 32]),
            proposer: TezedgeProposer::new(
                TezedgeProposerConfig {
                    wait_for_events_timeout: Some(Duration::from_millis(250)),
                    events_limit: 1024,
                },
                build_tezedge_state(),
                // capacity is changed by events_limit.
                MioEvents::new(),
                MioManager::new(SERVER_PORT),
            )
        }
    }

    fn send(
        proposer: &mut TezedgeProposer<MioEvents, DefaultEffects, MioManager>,
        peer: PeerAddress,
        msg: PeerMessage
    ) {
        match proposer.blocking_send(Instant::now(), peer, msg) {
            Err(e) => {
                eprintln!("ERROR: {}", e);
                std::process::exit(-1);
            },
            Ok(()) => (),
        }
    }

    pub fn handle_response(&mut self, peer: PeerAddress, msg: PeerMessageResponse) {
        eprintln!("received message from {}, contents: {:?}", peer, msg.message);
        //eprintln!("Starting advertise message flood...");

        for (i, msg) in self.generator.by_ref().enumerate() {
            BadNode::send(&mut self.proposer, peer, msg);
            // eprintln!("{} sent", i);

            // if i % self.throughput == 0 {
            //     eprintln!("{} messages", i);
            //     self.proposer.make_progress();
            // }
        }
    }

    pub fn handle_events(&mut self) {
        self.proposer.make_progress();

        for n in self.proposer.take_notifications().collect::<Vec<_>>() {
            match n {
                Notification::HandshakeSuccessful { peer_address, .. } => {
                    eprintln!("handshake from {}", peer_address);
                    BadNode::send(&mut self.proposer, peer_address, PeerMessage::Bootstrap);
                }
                Notification::MessageReceived { peer, message } => {
                    self.handle_response(peer, message);
                }
                Notification::PeerDisconnected { peer } => {
                    eprintln!("DISCONNECTED {}", peer);
                }
                Notification::PeerBlacklisted { peer } => {
                    eprintln!("BLACKLISTED {}", peer);
                }
                _ => {}
            }
        }
    }
}


fn main() {
    let mut node = BadNode::new();

    loop {
        node.handle_events();
    }
}
