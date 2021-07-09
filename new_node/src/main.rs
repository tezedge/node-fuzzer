mod generator;

use generator::Generator;
use tezos_messages::p2p::binary_message::MessageHash;
use std::collections::HashSet;
use std::convert::TryFrom;
use std::iter::FromIterator;
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::rc::Rc;
use std::u128;

use slog::Drain;

use tezedge_state::ShellCompatibilityVersion;
use tezos_messages::p2p::encoding::peer::{PeerMessage, PeerMessageResponse};

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use tla_sm::Acceptor;

use crypto::{
    crypto_box::{CryptoKey, PublicKey, SecretKey},
    hash::{CryptoboxPublicKeyHash, HashTrait, HashType, ChainId},
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



struct BadNode {
    throughput: usize,
    generator: generator::Message,
    peer: Option<PeerAddress>,
    proposer: TezedgeProposer<MioEvents, MioManager>,
}

impl BadNode {
    pub fn new() -> Self {
        BadNode {
            throughput: 0x80000, // TODO: fix BrokenPipe error
            generator: generator::Message::new([1; 32]),
            peer: None,
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
        proposer: &mut TezedgeProposer<MioEvents, MioManager>,
        peer: PeerAddress,
        msg: PeerMessage
    ) {
        let result = proposer.send_message_to_peer_or_queue(
            Instant::now(),
            peer,
            msg
        );
        match result {
            Err(e) => {
                eprintln!("ERROR: {}", e);
                std::process::exit(-1);
            },
            Ok(()) => (),
        }
    }

    pub fn send_messages(&mut self) {
        let msg_generator = generator::iterable(self.generator.clone());
        for msg in msg_generator.take(self.throughput) {
            BadNode::send(&mut self.proposer, self.peer.unwrap(), msg);
        }
    }

    pub fn handle_response(&mut self, msg: PeerMessageResponse) {
        eprintln!("received message from {}, contents: {:?}", self.peer.unwrap(), msg.message);
/* 
        match msg.message {
            Pee
        }
        */
        // TODO: use incoming messages as feedback for the fuzzer
    }

    pub fn handle_events(&mut self) -> bool {
        self.proposer.make_progress();

        for n in self.proposer.take_notifications().collect::<Vec<_>>() {
            match n {
                Notification::HandshakeSuccessful { peer_address, .. } => {
                    self.peer = Some(peer_address);
                    eprintln!("handshake from {}", peer_address);
                    BadNode::send(&mut self.proposer, peer_address, PeerMessage::Bootstrap);
                }
                Notification::MessageReceived { peer, message } => {
                    self.peer = Some(peer);
                    self.handle_response(message);
                }
                Notification::PeerDisconnected { peer } => {
                    eprintln!("DISCONNECTED {}", peer);
                    self.peer = None;
                    return false;
                }
                Notification::PeerBlacklisted { peer } => {
                    eprintln!("BLACKLISTED {}", peer);
                    self.peer = None;
                    return false;
                }
                _ => {}
            }
        }
        match self.peer {
            Some(_peer) => self.send_messages(),
            _ => {}
        }
        
        true
    }
}


fn main() {
    let mut node = BadNode::new();

    while node.handle_events() {        
    }
}
