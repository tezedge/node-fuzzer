mod generator;
use std::io;
use clap::{Arg, App};
use crypto::hash;
use generator::Generator;
use tezedge_state::DefaultEffects;
use tezos_messages::p2p::binary_message::{BinaryWrite, MessageHash};
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

fn build_tezedge_state(peer: &str) -> TezedgeState {
    let node_identity = identity_1();
    let mut tezedge_state = TezedgeState::new(
        logger(slog::Level::Trace),
        TezedgeConfig {
            port: SERVER_PORT,
            disable_mempool: true,
            private_node: false,
            min_connected_peers: 1,
            /* 
                WARNING: don't change these values
                fuzzer might go into to real network!
             */
            max_connected_peers: 1, 
            max_pending_peers: 1,
            max_potential_peers: 1,
            /* -------------------------------------------- */
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
            vec![peer].into_iter() .map(|x| x.parse().unwrap()) .collect::<Vec<_>>(),
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
    generator: generator::RandomState,
    peer: Option<PeerAddress>,
    proposer: TezedgeProposer<MioEvents, DefaultEffects, MioManager>,
}

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

impl BadNode {
    pub fn new(peer: &str, generator: generator::RandomState) -> Self {
        BadNode {
            throughput: 0x10000,
            generator: generator,
            peer: None,
            proposer: TezedgeProposer::new(
                TezedgeProposerConfig {
                    wait_for_events_timeout: Some(Duration::from_millis(250)),
                    events_limit: 1024,
                },
                build_tezedge_state(peer),
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
    ) -> io::Result<()> {
        proposer.blocking_send(Instant::now(), peer, msg)
    }

    pub fn send_messages(&mut self) -> io::Result<()> {
        for _  in 0..self.throughput {
            let msg = self.generator.gen();
            BadNode::send(&mut self.proposer, self.peer.unwrap(), msg)?;
        }
        Ok(())
    }

    pub fn handle_response(&mut self, msg: PeerMessageResponse) -> io::Result<()> {
        eprintln!("received message from {}, contents: {:?}", self.peer.unwrap(), msg.message);
        match msg.message {
            PeerMessage::GetCurrentBranch(current_branch::GetCurrentBranchMessage{chain_id}) => {
                self.generator.set_chain_id(chain_id);
                let msg: current_branch::CurrentBranchMessage = self.generator.gen();
                eprintln!("sending CurrentBranch");
                BadNode::send(
                    &mut self.proposer,
                    self.peer.unwrap(),
                    PeerMessage::CurrentBranch(msg)
                )
            },
            PeerMessage::GetBlockHeaders(bh) => {
                for block_hash in bh.get_block_headers() {
                    let state = self.generator.state();
                    let bh = state.blocks.get(&block_hash).unwrap();
                    let msg = block_header::BlockHeaderMessage::from(bh.clone());
                    eprintln!("sending BlockHeader");                    
                    BadNode::send(
                        &mut self.proposer,
                        self.peer.unwrap(),
                        PeerMessage::BlockHeader(msg)
                    )?; 
                }
                Ok(())
            },
            PeerMessage::GetOperationsForBlocks(ops) => {
                for operations_for_block in ops.get_operations_for_blocks() {
                    let msg = operations_for_blocks::OperationsForBlocksMessage::new(
                        operations_for_block.clone(),
                        self.generator.gen(),
                        self.generator.gen()
                    );
            
                    eprintln!("sending OperationsForBlocksMessage");
                    BadNode::send(
                        &mut self.proposer,
                        self.peer.unwrap(),
                        PeerMessage::OperationsForBlocks(msg)
                    )?;        
                }
                Ok(())
            },
            _ => Ok(())
        }
    }

    pub fn handle_events(&mut self) -> io::Result<()> {
        self.proposer.make_progress();

        for n in self.proposer.take_notifications().collect::<Vec<_>>() {
            match n {
                Notification::HandshakeSuccessful { peer_address, .. } => {
                    self.peer = Some(peer_address);
                    eprintln!("handshake from {}", peer_address);
                    BadNode::send(&mut self.proposer, peer_address, PeerMessage::Bootstrap)?
                },
                Notification::MessageReceived { peer, message } => {
                    self.peer = Some(peer);
                    self.handle_response(message);
                },
                Notification::PeerDisconnected {..} => {
                    return Err(io::Error::new(io::ErrorKind::ConnectionAborted,"disconnected"));
                },
                Notification::PeerBlacklisted {..} => {
                    return Err(io::Error::new(io::ErrorKind::ConnectionAborted,"Blacklisted"));
                }
            }
        }
        //Ok(())   
        match self.peer {
            Some(_peer) => self.send_messages(),
            _ => Ok(())
        }
    }
}


fn main() {
    let matches = App::new("Bad-Node")
    .arg(Arg::with_name("peer")
             .short("p")
             .long("peer")
             .takes_value(true)
             .help("target peer address")
             .default_value("127.0.0.1:9732")
            )
    .arg(Arg::with_name("seed")
            .short("s")
            .long("seed")
            .takes_value(true)
            .help("PRNG seed")
            .default_value("123456789abcdef0")
           )
    .arg(Arg::with_name("messages")
             .short("m")
             .long("messages")
             .multiple(true)
             .takes_value(true)
             .help("list of possible messages to generate")
             .default_value("all")
            )
    .get_matches();

    let peer = matches.value_of("peer").unwrap();
    let seed = u64::from_str_radix(
        matches.value_of("seed").unwrap(), 16
    ).unwrap();
    let mut messages: Vec<&str> = matches.values_of("messages").unwrap().collect();

    if *messages.first().unwrap() == "all" {
        messages.clear();
        messages = vec![
            "Advertise", "SwapRequest", "SwapAck", "GetCurrentBranch",
            "CurrentBranch", "Deactivate", "GetCurrentHead", "CurrentHead",
            "GetBlockHeaders", "BlockHeader", "GetOperations", "Operation",
            "GetProtocols", "Protocol", "GetOperationsForBlocks", "OperationsForBlocks"
        ];
    }
    let generator = generator::RandomState::new(
        seed,
        messages.iter().map(|x| {String::from(*x)}).collect()
    );

    loop {
        let mut node = BadNode::new(peer, generator.clone());

        loop {
            match node.handle_events() {
                Err(e) => {
                    eprintln!("ERROR: {}", e);
                    //return;
                    break;
                }
                _ => {}
            }
        }
    } 
}
