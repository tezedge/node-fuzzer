mod generator;
use clap::{App, Arg};
use generator::Generator;
use serde::{Deserialize, Serialize};
use slog::Drain;
use tezos_messages::p2p::encoding::block_header::GetBlockHeadersMessage;
//use tezos_messages::p2p::encoding::operation::GetOperationsMessage;
use tezos_messages::p2p::encoding::operations_for_blocks::GetOperationsForBlocksMessage;
use std::collections::HashSet;
use std::io;
use std::iter::FromIterator;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tezedge_state::DefaultEffects;
use tezedge_state::ShellCompatibilityVersion;
use tezos_messages::p2p::encoding::{
    block_header, current_branch, operations_for_blocks,
    peer::{PeerMessage, PeerMessageResponse},
};

use tla_sm::Acceptor;

use crypto::{
    crypto_box::{PublicKey, SecretKey},
    hash::CryptoboxPublicKeyHash,
    proof_of_work::ProofOfWork,
};
use hex::FromHex;
use tezedge_state::proposals::ExtendPotentialPeersProposal;
use tezedge_state::{PeerAddress, TezedgeConfig, TezedgeState};
use tezos_identity::Identity;

use tezedge_state::proposer::mio_manager::{MioEvents, MioManager};
use tezedge_state::proposer::{Notification, TezedgeProposer, TezedgeProposerConfig};

#[derive(Clone, Serialize, Deserialize)]
struct Credentials {
    peer_id: String,
    public_key: String,
    private_key: String,
    pow: String,
}

#[derive(Clone, Serialize, Deserialize)]
struct Profile {
    id: Credentials,
    peer: String,
    prng_seed: u64,
    push_messages: Vec<String>, // send messages to target regardless of requests
    push_throughput: u64,       // number of messages send in burst
    blocks_limit: usize,          // max block-header vector elements
    reply_to_get_current_branch: bool, // reply to GetCurrentBranch requests
    reply_to_get_block_headers: bool, // reply to GetBlockHeaders requests
    reply_to_get_operations_for_blocks: bool, // reply to GetOperationsForBlocks requests
}

const SERVER_PORT: u16 = 13632;
static DEFAULT_PROFILE: &'static str = r#"
{
    "id": {
        "peer_id": "idrdoT9g6YwELhUQyshCcHwAzBS9zA",
        "public_key": "94498d9416140fbc458495333daac1b4c87e419f5726717a54f9b6c67476ae1c",
        "private_key": "ac7acf3afed7637be10f8fc76a2eb6b3359c78adb1d813b41cbab3fae954f4b1",
        "pow": "bbc2300149249e1ccc84a54362236c3cbbc2cc2ffbd3b6ea"
    },
    "peer": "127.0.0.1:9732",
    "prng_seed": 1234567890,
    "push_messages": [
        "Advertise",
        "SwapRequest",
        "SwapAck",
        "GetCurrentBranch",
        "CurrentBranch",
        "Deactivate",
        "GetCurrentHead",
        "CurrentHead",
        "GetBlockHeaders",
        "BlockHeader",
        "GetOperations",
        "Operation",
        "GetProtocols",
        "Protocol",
        "GetOperationsForBlocks",
        "OperationsForBlocks"    
    ],
    "push_throughput": 65536,
    "blocks_limit": 500,
    "reply_to_get_current_branch": true,
    "reply_to_get_block_headers": true,
    "reply_to_get_operations_for_blocks": true
}"#;

struct BadNode {
    profile: Profile,
    generator: generator::RandomState,
    peer: Option<PeerAddress>,
    proposer: TezedgeProposer<MioEvents, DefaultEffects, MioManager>,
}

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

fn build_tezedge_state(profile: &Profile) -> TezedgeState {
    let node_identity = Identity {
        peer_id: CryptoboxPublicKeyHash::from_base58_check(&profile.id.peer_id).unwrap(),
        public_key: PublicKey::from_hex(&profile.id.public_key).unwrap(),
        secret_key: SecretKey::from_hex(&profile.id.private_key).unwrap(),
        proof_of_work_stamp: ProofOfWork::from_hex(&profile.id.pow).unwrap(),
    };

    let mut tezedge_state = TezedgeState::new(
        logger(slog::Level::Trace),
        TezedgeConfig {
            port: SERVER_PORT,
            disable_mempool: true,
            private_node: false,
            min_connected_peers: 1,
            /* WARNING: don't change these values fuzzer might go into to real network! */
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
        ShellCompatibilityVersion::new("TEZOS_MAINNET".to_owned(), vec![0], vec![0, 1]),
        Default::default(),
        Instant::now(),
    );
    tezedge_state.accept(ExtendPotentialPeersProposal {
        at: Instant::now(),
        peers: HashSet::<_>::from_iter(vec![profile.peer.parse().unwrap()].into_iter()),
    });
    tezedge_state
}

impl BadNode {
    pub fn new(profile: Profile, generator: generator::RandomState) -> Self {
        BadNode {
            profile: profile.clone(),
            generator: generator,
            peer: None,
            proposer: TezedgeProposer::new(
                TezedgeProposerConfig {
                    wait_for_events_timeout: Some(Duration::from_millis(250)),
                    events_limit: 1024,
                },
                build_tezedge_state(&profile),
                // capacity is changed by events_limit.
                MioEvents::new(),
                MioManager::new(SERVER_PORT),
            ),
        }
    }

    fn send(
        proposer: &mut TezedgeProposer<MioEvents, DefaultEffects, MioManager>,
        peer: PeerAddress,
        msg: PeerMessage,
    ) -> io::Result<()> {
        proposer.blocking_send(Instant::now(), peer, msg)
    }

    pub fn send_messages(&mut self) -> io::Result<()> {
        let messages = &self.profile.push_messages;
        eprintln!("Pushing {} messages in burst...", self.profile.push_throughput);

        for _ in 0..self.profile.push_throughput {
            let index = self.generator.gen_range(0..messages.len());

            let msg = match &messages.get(index).unwrap()[..] {
                "Advertise" => PeerMessage::Advertise(self.generator.gen()),
                "SwapRequest" => PeerMessage::SwapRequest(self.generator.gen()),
                "SwapAck" => PeerMessage::SwapAck(self.generator.gen()),
                "GetCurrentBranch" => PeerMessage::GetCurrentBranch(self.generator.gen()),
                "CurrentBranch" => PeerMessage::CurrentBranch(self.generator.gen()),
                "Deactivate" => PeerMessage::Deactivate(self.generator.gen()),
                "GetCurrentHead" => PeerMessage::GetCurrentHead(self.generator.gen()),
                "CurrentHead" => PeerMessage::CurrentHead(self.generator.gen()),
                "GetBlockHeaders" => PeerMessage::GetBlockHeaders(self.generator.gen()),
                "BlockHeader" => PeerMessage::BlockHeader(self.generator.gen()),
                "GetOperations" => PeerMessage::GetOperations(self.generator.gen()),
                "Operation" => PeerMessage::Operation(self.generator.gen()),
                "GetProtocols" => PeerMessage::GetProtocols(self.generator.gen()),
                "Protocol" => PeerMessage::Protocol(self.generator.gen()),
                "GetOperationsForBlocks" => PeerMessage::GetOperationsForBlocks(self.generator.gen()),
                "OperationsForBlocks" => PeerMessage::OperationsForBlocks(self.generator.gen()),
                _ => panic!("Invalid message"),
            };
            BadNode::send(&mut self.proposer, self.peer.unwrap(), msg)?;
        }

        Ok(())
    }

    pub fn reply_to_get_current_branch(&mut self, chain_id: crypto::hash::ChainId) -> io::Result<()> {
        self.generator.set_chain_id(chain_id);
        let msg: current_branch::CurrentBranchMessage = self.generator.gen();
        eprintln!("sending CurrentBranch");
        BadNode::send(
            &mut self.proposer,
            self.peer.unwrap(),
            PeerMessage::CurrentBranch(msg),
        )
    }

    pub fn reply_to_get_block_headers(&mut self, bh: GetBlockHeadersMessage) -> io::Result<()> {
        for block_hash in bh.get_block_headers() {
            let state = self.generator.state();
            let bh = state.blocks.get(&block_hash).unwrap();
            let msg = block_header::BlockHeaderMessage::from(bh.clone());
            eprintln!("sending BlockHeader");
            BadNode::send(
                &mut self.proposer,
                self.peer.unwrap(),
                PeerMessage::BlockHeader(msg),
            )?;
        }
        Ok(())
    }

    pub fn reply_to_get_operations_for_blocks(&mut self, ops: GetOperationsForBlocksMessage) -> io::Result<()> {
        for operations_for_block in ops.get_operations_for_blocks() {
            let msg = operations_for_blocks::OperationsForBlocksMessage::new(
                operations_for_block.clone(),
                self.generator.gen(),
                self.generator.gen(),
            );
            eprintln!("sending OperationsForBlocksMessage");
            BadNode::send(
                &mut self.proposer,
                self.peer.unwrap(),
                PeerMessage::OperationsForBlocks(msg),
            )?;
        }
        Ok(())
    }

    pub fn handle_response(&mut self, msg: PeerMessageResponse) -> io::Result<()> {
        eprintln!("received message: {:?}", msg.message);
        match msg.message {
            PeerMessage::GetCurrentBranch(current_branch::GetCurrentBranchMessage { chain_id }) => {
                match self.profile.reply_to_get_current_branch {
                    true => self.reply_to_get_current_branch(chain_id),
                    _ => Ok(())
                }
            },
            PeerMessage::GetBlockHeaders(bh) => {
                match self.profile.reply_to_get_block_headers {
                    true => self.reply_to_get_block_headers(bh),
                    _ => Ok(())
                }
            },
            PeerMessage::GetOperationsForBlocks(ops) => {
                match self.profile.reply_to_get_block_headers {
                    true => self.reply_to_get_operations_for_blocks(ops),
                    _ => Ok(())
                }
            },
            _ => Ok(()),
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
                }
                Notification::MessageReceived { peer, message } => {
                    self.peer = Some(peer);
                    self.handle_response(message)?;
                }
                Notification::PeerDisconnected { .. } => {
                    return Err(io::Error::new(
                        io::ErrorKind::ConnectionAborted,
                        "disconnected",
                    ));
                }
                Notification::PeerBlacklisted { .. } => {
                    return Err(io::Error::new(
                        io::ErrorKind::ConnectionAborted,
                        "Blacklisted",
                    ));
                }
            }
        }
        match self.peer {
            Some(_) => self.send_messages(),
            None => Ok(()),
        }
    }
}

fn run(profile: Profile, generator: generator::RandomState) {
    loop {
        let mut node = BadNode::new(profile.clone(), generator.clone());

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

fn main() {
    let matches = App::new("Bad-Node")
        .arg(
            Arg::with_name("config")
                .short("c")
                .long("config")
                .takes_value(true)
                .help("path to JSON configuration file"),
        )
        .get_matches();

    let config = match matches.value_of("config") {
        Some(file) => std::fs::read_to_string(file).unwrap(),
        None => DEFAULT_PROFILE.to_string(),
    };
    match serde_json::from_str::<Profile>(&config) {
        Ok(profile) => {
            let generator = generator::RandomState::new(
                profile.prng_seed,
                profile.blocks_limit
            );
            run(profile, generator);        
        },
        Err(e) => {
            eprintln!("error {}", e);
        }
    }
}
