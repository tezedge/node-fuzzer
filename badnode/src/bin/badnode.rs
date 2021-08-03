use clap::{App, Arg};
use generator::{Generator, RandomState};
use hex::FromHex;
use serde::{Deserialize, Serialize};
use tezos_messages::p2p::binary_message;
use tezos_messages::p2p::encoding::{block_header, current_branch, operations_for_blocks};
use tokio::net::unix::SocketAddr;
use tokio::runtime::Handle;

use futures::future::join_all;
use std::borrow::Borrow;
use std::convert::{TryFrom, TryInto};
use std::io::{Error, ErrorKind, Read, Result};
use std::net::{IpAddr, Ipv4Addr, SocketAddrV4};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpSocket, TcpStream};
use tokio::time::{sleep, Duration};

use crypto::{
    crypto_box::{CryptoKey, PrecomputedKey, PublicKey, SecretKey},
    hash::CryptoboxPublicKeyHash,
    nonce::{generate_nonces, Nonce, NoncePair},
    proof_of_work::ProofOfWork,
};
use tezos_identity::Identity;
use tezos_messages::p2p::{
    binary_message::{BinaryChunk, BinaryMessage, BinaryRead, BinaryWrite},
    encoding::{
        ack::AckMessage,
        connection::ConnectionMessage,
        current_branch::GetCurrentBranchMessage,
        metadata::MetadataMessage,
        peer::{PeerMessage, PeerMessageResponse},
        version::NetworkVersion,
    },
};

#[derive(Clone, Serialize, Deserialize)]
struct Credentials {
    peer_id: String,
    public_key: String,
    secret_key: String,
    proof_of_work_stamp: String,
}

#[derive(Clone, Serialize, Deserialize)]
struct ChunkOptions {
    split_chunks: bool,         // split a message into multiple chunks
    split_bytes: bool,          // split sending of chunk bytes into multiple slices
    incomplete_send: bool,      // randomly abort in the middle of sending bytes slices
    max_write_bytes_sleep: u64, // randomly sleep between sending byte slices
}

#[derive(Clone, Serialize, Deserialize)]
struct HandshakeOptions {
    chunk_options: ChunkOptions,
    // connect message options
    abort_before_connect: bool, // randomly abort before sending connect message
    max_connect_sleep: u64,     // max random sleep between TCP connect and connect message
    max_connect_msg_sleep: u64, // max random sleep after connect message
    fuzz_public_key: bool,      // enable/disable public key fuzzing
    fuzz_pow: bool,             // enable/disable Proof Of Work fuzzing
    fuzz_network_version: bool, // enable/disable protocol version fuzzing
    // metadata message options
    abort_before_metadata: bool, // randomly abort before sending metadata message
    max_metadata_msg_sleep: u64, // max random sleep after metadata message
    always_recv_metadata: bool,  // attempt to recv metadata message from peer
    fuzz_metadata: bool,         // enable/disable metadata fuzzing
    // ack message options
    abort_before_ack: bool,   // randomly abort before sending ack message
    max_ack_msg_sleep: u64,   // max random sleep after ack message
    ack_replies: Vec<String>, // possible replies to generate randomly: Ack, NackV0, Nack
    always_recv_ack: bool,    // attempt to recv ack message from peer
}

#[derive(Clone, Serialize, Deserialize)]
struct PeerMessageReplyOptions {
    get_current_branch: bool,        // reply to GetCurrentBranch requests
    get_block_headers: bool,         // reply to GetBlockHeaders requests
    get_operations_for_blocks: bool, // reply to GetOperationsForBlocks requests
}

#[derive(Clone, Serialize, Deserialize)]
struct PeerMessageOptions {
    chunk_options: ChunkOptions,
    messages_per_chunk: u64, // default: 1, merge more than one message per chunk
    push_messages: Vec<String>, // possible messages send randomly to target: Advertise,
    // SwapRequest, SwapAck, GetCurrentBranch, CurrentBranch,
    // Deactivate, GetCurrentHead, CurrentHead, GetBlockHeaders,
    // BlockHeader, GetOperations, Operation, GetProtocols,
    // Protocol, GetOperationsForBlocks, OperationsForBlocks
    push_throughput: u64,      // number of messages send in burst
    block_header_limit: usize, // max block-header cache size
    reply_options: Option<PeerMessageReplyOptions>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Profile {
    peer: String,
    prng_seed: u64,
    max_chunk_size: usize,
    threads: u64,
    handshake_fuzzing: Option<HandshakeOptions>, // if null generate valid handshakes
    peer_message_fuzzing: Option<PeerMessageOptions>,
}

static DEFAULT_IDENTITY: &'static str = r#"
[{
    "peer_id":"idt3LoYeur2FXAvRXsvmW2Ac9EpEHb",
    "public_key":"f0c7f2fdf2e6effb32fb6d750051dc63512b5bd8d16ac7aa110f4f4fc840552e",
    "proof_of_work_stamp":"d5a8e2140d85615201c0e08b1bd22780b8d766af9be06123",
    "secret_key":"051a6c62253e37577de1031fee4e584487494d6da8186dba608a894ba2834a40"
}]
"#;

static DEFAULT_PROFILE: &'static str = r#"
{
    "peer": "172.28.0.0:9732",
    "prng_seed": 1234567890,
    "max_chunk_size": 65519,
    "threads": 1,
    "handshake_fuzzing": null,
    "peer_message_fuzzing": {
        "chunk_options": {
            "split_chunks": true,
            "split_bytes": false,
            "incomplete_send": false,
            "max_write_bytes_sleep": 0
        },
        "messages_per_chunk": 1,
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
        "block_header_limit": 0,
        "reply_options": null
    }
}"#;

async fn rand_sleep(max_duration: u64, generator: &mut generator::RandomState) {
    if max_duration > 0 {
        sleep(Duration::from_millis(generator.gen_range(0..max_duration))).await;
    }
}

pub struct ChunkBuffer {
    len: usize,
    data: [u8; 0x10000],
}

impl Default for ChunkBuffer {
    fn default() -> Self {
        ChunkBuffer {
            len: 0,
            data: [0; 0x10000],
        }
    }
}

impl ChunkBuffer {
    pub async fn read_chunk(&mut self, stream: &mut TcpStream) -> Result<BinaryChunk> {
        const HEADER_LENGTH: usize = 2;
        let mut retry = 0;
        loop {
            if self.len >= HEADER_LENGTH {
                let chunk_len = u16::from_be_bytes([self.data[0], self.data[1]]) as usize;
                let raw_len = chunk_len + HEADER_LENGTH;

                if self.len >= raw_len {
                    let chunk = self.data[..(raw_len)].to_vec();
                    for i in raw_len..self.len {
                        self.data[(i - raw_len)] = self.data[i];
                    }
                    self.len -= raw_len;

                    return Ok(chunk.try_into().unwrap());
                }
            }

            self.len += stream.read(&mut self.data[self.len..]).await?;

            if self.len == 0 {
                if retry > 2 {
                    return Err(Error::new(ErrorKind::UnexpectedEof, "zero length read"));
                }

                retry += 1;
                sleep(Duration::from_millis(200)).await;
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

async fn write_bytes(
    options: &ChunkOptions,
    generator: &mut generator::RandomState,
    bytes: &Vec<u8>,
    stream: &mut TcpStream,
) -> Result<()> {
    let chunks = match options.split_bytes && generator.gen_t::<bool>() {
        true => generator.gen_byte_chunks(bytes.clone()),
        false => vec![bytes.clone()],
    };

    let incomplete_send = options.incomplete_send && generator.gen_t::<bool>();

    for chunk in chunks {
        stream.write_all(chunk.as_slice()).await?;

        if incomplete_send && generator.gen_t::<bool>() {
            return Ok(());
        }

        rand_sleep(options.max_write_bytes_sleep, generator).await;
    }

    Ok(())
}

async fn write_bytes_encrypted(
    options: &ChunkOptions,
    generator: &mut generator::RandomState,
    bytes: &Vec<u8>,
    stream: &mut TcpStream,
    key: &PrecomputedKey,
    nonce: &mut Nonce,
) -> Result<()> {
    const MACBYTES: usize = 16;
    let chunks = match options.split_chunks && generator.gen_t::<bool>() {
        true => generator.gen_chunks(bytes.clone(), key, nonce),
        false => bytes
            .as_slice()
            .chunks(binary_message::CONTENT_LENGTH_MAX - MACBYTES)
            .map(|x| {
                let bytes = key.encrypt(x, nonce).unwrap();
                *nonce = nonce.increment();
                BinaryChunk::from_content(&bytes).unwrap().clone()
            })
            .collect(),
    };

    for chunk in chunks {
        write_bytes(options, generator, chunk.raw(), stream).await?;
    }

    Ok(())
}

async fn write_msg<M>(
    options: &ChunkOptions,
    generator: &mut generator::RandomState,
    msg: &M,
    stream: &mut TcpStream,
    key: &PrecomputedKey,
    nonce: &mut Nonce,
) -> Result<()>
where
    M: BinaryMessage,
{
    let bytes = msg.as_bytes().unwrap();
    write_bytes_encrypted(options, generator, &bytes, stream, key, nonce).await?;
    Ok(())
}

async fn read_msg<M>(
    stream: &mut TcpStream,
    buffer: &mut ChunkBuffer,
    key: &PrecomputedKey,
    nonce: &mut Nonce,
    peer_message: bool,
) -> Result<M>
where
    M: BinaryMessage,
{
    const HEADER_LENGTH: usize = 4;
    let mut bytes = Vec::new();
    let mut length = 0;

    loop {
        let chunk = buffer.read_chunk(stream).await?;
        let decrypted = key.decrypt(chunk.content(), &nonce);

        if decrypted.is_err() {
            return Err(Error::new(ErrorKind::Other, "decrypt error"));
        }

        bytes.extend_from_slice(&decrypted.unwrap().as_slice());

        if length == 0 && peer_message {
            let b = TryFrom::try_from(&bytes[..HEADER_LENGTH]).unwrap();
            length = u32::from_be_bytes(b) as usize + HEADER_LENGTH;
        }
        *nonce = nonce.increment();

        if bytes.len() == length || !peer_message {
            let m = M::from_bytes(&bytes);

            if !m.is_err() {
                return Ok(m.unwrap());
            }
        }
    }
}

async fn fuzz_peer_messages(
    options: &PeerMessageOptions,
    generator: &mut generator::RandomState,
    stream: &mut TcpStream,
    key: &PrecomputedKey,
    local: &mut Nonce,
) -> Result<()> {
    let messages = &options.push_messages;
    let mut remaining = options.push_throughput;
    eprintln!("Pushing {} messages in burst...", options.push_throughput);

    loop {
        if options.messages_per_chunk > remaining {
            break;
        }

        remaining -= options.messages_per_chunk;
        let mut msg_batch: Vec<u8> = Vec::new();

        for _ in 0..options.messages_per_chunk {
            let index = generator.gen_range(0..messages.len());
            let msg = match &messages.get(index).unwrap()[..] {
                "Advertise" => PeerMessage::Advertise(generator.gen()),
                "SwapRequest" => PeerMessage::SwapRequest(generator.gen()),
                "SwapAck" => PeerMessage::SwapAck(generator.gen()),
                "GetCurrentBranch" => PeerMessage::GetCurrentBranch(generator.gen()),
                "CurrentBranch" => PeerMessage::CurrentBranch(generator.gen()),
                "Deactivate" => PeerMessage::Deactivate(generator.gen()),
                "GetCurrentHead" => PeerMessage::GetCurrentHead(generator.gen()),
                "CurrentHead" => PeerMessage::CurrentHead(generator.gen()),
                "GetBlockHeaders" => PeerMessage::GetBlockHeaders(generator.gen()),
                "BlockHeader" => PeerMessage::BlockHeader(generator.gen()),
                "GetOperations" => PeerMessage::GetOperations(generator.gen()),
                "Operation" => PeerMessage::Operation(generator.gen()),
                "GetProtocols" => PeerMessage::GetProtocols(generator.gen()),
                "Protocol" => PeerMessage::Protocol(generator.gen()),
                "GetOperationsForBlocks" => PeerMessage::GetOperationsForBlocks(generator.gen()),
                "OperationsForBlocks" => PeerMessage::OperationsForBlocks(generator.gen()),
                _ => panic!("Invalid message"),
            };
            //eprintln!("msg {:?}",  msg.get_type());
            msg_batch.append(&mut PeerMessageResponse::from(msg).as_bytes().unwrap());
        }

        write_bytes_encrypted(
            &options.chunk_options,
            generator,
            &msg_batch,
            stream,
            &key,
            local,
        )
        .await?;
    }
    Ok(())
}

async fn handle_response(
    options: &PeerMessageOptions,
    generator: &mut generator::RandomState,
    stream: &mut TcpStream,
    key: &PrecomputedKey,
    local: &mut Nonce,
    remote: &mut Nonce,
    buffer: &mut ChunkBuffer,
) -> Result<()> {
    if let Some(reply_options) = options.reply_options.clone() {
        let m = read_msg::<PeerMessageResponse>(stream, buffer, &key, remote, true).await?;
        eprintln!("recv msg {:?}", m);
        match m.message {
            PeerMessage::GetCurrentBranch(current_branch::GetCurrentBranchMessage { chain_id }) => {
                if reply_options.get_current_branch {
                    generator.set_chain_id(chain_id);
                    let msg: current_branch::CurrentBranchMessage = generator.gen();
                    eprintln!("sending CurrentBranch");
                    write_msg(
                        &options.chunk_options,
                        generator,
                        &PeerMessageResponse::from(msg),
                        stream,
                        &key,
                        local,
                    )
                    .await?;
                }
            }
            PeerMessage::GetBlockHeaders(bh) => {
                if reply_options.get_block_headers {
                    for block_hash in bh.get_block_headers() {
                        let msg = generator.get_block(block_hash);
                        eprintln!("sending BlockHeader");
                        write_msg(
                            &options.chunk_options,
                            generator,
                            &PeerMessageResponse::from(msg),
                            stream,
                            &key,
                            local,
                        )
                        .await?;
                    }
                }
            }
            PeerMessage::GetOperationsForBlocks(ops) => {
                if reply_options.get_operations_for_blocks {
                    for operations_for_block in ops.get_operations_for_blocks() {
                        let msg = operations_for_blocks::OperationsForBlocksMessage::new(
                            operations_for_block.clone(),
                            generator.gen(),
                            generator.gen(),
                        );
                        eprintln!("sending OperationsForBlocksMessage");
                        write_msg(
                            &options.chunk_options,
                            generator,
                            &PeerMessageResponse::from(msg),
                            stream,
                            &key,
                            local,
                        )
                        .await?;
                    }
                }
            }
            _ => (),
        }
    }

    Ok(())
}

async fn fuzz(
    identities: &Vec<Identity>,
    profile: &Profile,
    generator: &mut generator::RandomState
) -> Result<()> {
    let options = match profile.handshake_fuzzing.borrow() {
        None => HandshakeOptions {
            chunk_options: ChunkOptions {
                split_chunks: false,
                split_bytes: false,
                incomplete_send: false,
                max_write_bytes_sleep: 0,
            },
            abort_before_connect: false,
            max_connect_sleep: 0,
            max_connect_msg_sleep: 0,
            fuzz_public_key: false,
            fuzz_pow: false,
            fuzz_network_version: false,
            abort_before_metadata: false,
            max_metadata_msg_sleep: 0,
            always_recv_metadata: true,
            fuzz_metadata: false,
            abort_before_ack: false,
            max_ack_msg_sleep: 0,
            ack_replies: vec![String::from("Ack")],
            always_recv_ack: true,
        },
        Some(opt) => opt.clone(),
    };

    
    //eprintln!("connected");

    let socket = TcpSocket::new_v4()?;
    socket.set_reuseaddr(true)?;
    socket.set_reuseport(true)?;

    let addr = SocketAddrV4::new(
            // Target should be 172.28.50.0
            Ipv4Addr::new(
                172, 
                28, 
                generator.gen_range(0..255), 
                generator.gen_range(2..255)),
            generator.gen_range(2000 as u16..0xffff)
        );
    

    socket.bind(addr.try_into().unwrap())?;
    let mut stream = socket.connect(profile.peer.clone().parse().unwrap()).await?;

    let identity = identities.get(generator.gen_range(0..identities.len())).unwrap();

    if options.abort_before_connect && generator.gen_t::<bool>() {
        return Ok(());
    }

    rand_sleep(options.max_connect_sleep, generator).await;

    let version = match options.fuzz_network_version && generator.gen_t::<bool>() {
        false => NetworkVersion::new("TEZOS_MAINNET".to_string(), 0, 1),
        true => generator.gen(),
    };

    let public_key = match options.fuzz_public_key && generator.gen_t::<bool>() {
        false => identity.public_key.clone(),
        true => generator.gen(),
    };

    let pow = match options.fuzz_pow && generator.gen_t::<bool>() {
        false => identity.proof_of_work_stamp.clone(),
        true => generator.gen(),
    };

    let connection_message: ConnectionMessage = ConnectionMessage::try_new(
        generator.gen_t(),
        &public_key,
        &pow,
        generator.gen(),
        version,
    )
    .unwrap();

    let initiator_chunk =
        BinaryChunk::from_content(&connection_message.as_bytes().unwrap()).unwrap();
    write_bytes(
        &options.chunk_options,
        generator,
        &initiator_chunk.raw(),
        &mut stream,
    )
    .await?;

    if options.abort_before_metadata && generator.gen_t::<bool>() {
        return Ok(());
    }

    rand_sleep(options.max_connect_msg_sleep, generator).await;

    let mut buffer = ChunkBuffer::default();
    let responder_chunk = buffer.read_chunk(&mut stream).await?;
    let connection_message = ConnectionMessage::from_bytes(responder_chunk.content()).unwrap();

    let key = PrecomputedKey::precompute(
        &PublicKey::from_bytes(connection_message.public_key()).unwrap(),
        &identity.secret_key,
    );

    let NoncePair {
        mut local,
        mut remote,
    } = generate_nonces(&initiator_chunk.raw(), &responder_chunk.raw(), false).unwrap();

    let metadata = match options.fuzz_metadata && generator.gen_t::<bool>() {
        false => MetadataMessage::new(false, false),
        true => generator.gen(),
    };

    write_msg(
        &options.chunk_options,
        generator,
        &metadata,
        &mut stream,
        &key,
        &mut local,
    )
    .await?;

    if options.always_recv_metadata || generator.gen_t::<bool>() {
        let m =
            read_msg::<MetadataMessage>(&mut stream, &mut buffer, &key, &mut remote, false).await?;
        //eprintln!("metadata {:?}", m);
    }

    if options.abort_before_ack && generator.gen_t::<bool>() {
        return Ok(());
    }

    rand_sleep(options.max_metadata_msg_sleep, generator).await;

    let index = generator.gen_range(0..options.ack_replies.len());
    let ack: AckMessage = match &options.ack_replies.get(index).unwrap()[..] {
        "Ack" => AckMessage::Ack,
        "NackV0" => AckMessage::NackV0,
        _ => AckMessage::Nack(generator.gen()),
    };
    //eprintln!("send ack");
    write_msg(
        &options.chunk_options,
        generator,
        &ack,
        &mut stream,
        &key,
        &mut local,
    )
    .await?;
    rand_sleep(options.max_ack_msg_sleep, generator).await;

    if (options.always_recv_metadata && options.always_recv_ack) || generator.gen_t::<bool>() {
        //eprintln!("recv ack");
        read_msg::<AckMessage>(&mut stream, &mut buffer, &key, &mut remote, false).await?;
    }

    if let Some(peer_message_options) = profile.peer_message_fuzzing.borrow() {
        handle_response(
            &peer_message_options,
            generator,
            &mut stream,
            &key,
            &mut local,
            &mut remote,
            &mut buffer,
        )
        .await?;
        fuzz_peer_messages(
            &peer_message_options,
            generator,
            &mut stream,
            &key,
            &mut local,
        )
        .await?;
    }

    Ok(())
}

async fn worker(identities: Arc<Vec<Identity>>, profile: Arc<Profile>, task_id: u64) {
    let mut generator = RandomState::new(
        profile.prng_seed * task_id, 
        0, 
        profile.max_chunk_size
    );

    loop {
        match fuzz(&identities, &profile, &mut generator).await {
            Err(e) => {
                eprintln!("ERROR: {}", e);
                //break
            }
            _ => (),
        }
    }
}

#[tokio::main]
async fn main() {
    let matches = App::new("badnode")
        .arg(
            Arg::with_name("config")
                .short("c")
                .long("config")
                .takes_value(true)
                .help("path to JSON configuration file"),
        )
        .arg(
            Arg::with_name("identities")
                .short("i")
                .long("identities")
                .takes_value(true)
                .help("path to JSON identities file"),
        )
        .get_matches();

    let config = match matches.value_of("config") {
        Some(file) => std::fs::read_to_string(file).unwrap(),
        None => DEFAULT_PROFILE.to_string(),
    };

    let identities = serde_json::from_str::<Vec<Credentials>>(&
        match matches.value_of("identities") {
            Some(file) => std::fs::read_to_string(file).unwrap(),
            None => DEFAULT_IDENTITY.to_string(),
    }).unwrap().iter().map(|x| {
        Identity {
            peer_id: CryptoboxPublicKeyHash::from_base58_check(&x.peer_id).unwrap(),
            public_key: PublicKey::from_hex(&x.public_key).unwrap(),
            secret_key: SecretKey::from_hex(&x.secret_key).unwrap(),
            proof_of_work_stamp: ProofOfWork::from_hex(&x.proof_of_work_stamp).unwrap(),
        }
    }).collect::<Vec<Identity>>();

    let max_fd = fdlimit::raise_fd_limit().unwrap();
    eprintln!("current fd limit {}", max_fd);

    let mut profile = serde_json::from_str::<Profile>(&config).unwrap();
    profile.threads = std::cmp::min(profile.threads, max_fd);

    let mut tasks = Vec::new();
    let identities = Arc::new(identities);
    let profile = Arc::new(profile);

    for n in 0..profile.threads {
        let identities = Arc::clone(&identities);   
        let profile = Arc::clone(&profile);
        tasks.push(
            tokio::spawn(async move {
                worker(identities, profile, n).await
            })
        );
    }

    join_all(tasks).await;
}
