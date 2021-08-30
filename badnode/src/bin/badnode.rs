// TODO: hashes

use clap::{App, Arg};
use generator::{Generator, RandomState};
use hex::FromHex;
use serde::{Deserialize, Serialize};
use tezos_messages::p2p::binary_message;
use tezos_messages::p2p::encoding::{advertise, block_header, current_branch, operations_for_blocks};
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
    flip_bits: f64,            // probability to randomly flip bits in raw stream
    split_chunks: f64,         // probability to split a message into multiple chunks
    split_bytes: f64,          // probability to split sending of chunk bytes into multiple slices
    incomplete_send: f64,      // probability to randomly abort in the middle of sending bytes slices
    write_bytes_sleep: f64,    // probability to randomly sleep between sending byte slices
    write_bytes_sleep_ms: u64, // sleep milliseconds
}

#[derive(Clone, Serialize, Deserialize)]
struct HandshakeOptions {
    chunk_options: ChunkOptions,
    // connect message options
    abort_before_connect: f64, // probability to randomly abort before sending connect message
    connect_sleep: f64, // probability to randomly sleep between TCP connect and connect message
    connect_sleep_ms: u64,     // sleep milliseconds 
    connect_msg_sleep: f64, // probability to randomly sleep after connect message
    connect_msg_sleep_ms: u64, // sleep milliseconds 
    fuzz_public_key: f64,      // probability to fuzz public key
    fuzz_pow: f64,             // probability to fuzz Proof Of Work fuzzing
    fuzz_network_version: f64, // probability to fuzz protocol version fuzzing
    // metadata message options
    abort_before_metadata: f64, // probability to randomly abort before sending metadata message
    metadata_msg_sleep: f64, // probability yo random sleep after metadata message
    metadata_msg_sleep_ms: u64, 
    recv_metadata: f64,  // probability to attempt to recv metadata message from peer
    fuzz_metadata: f64,         // probability to fuzz metadata
    // ack message options
    abort_before_ack: f64,   // probability to randomly abort before sending ack message
    ack_msg_sleep: f64, // probability to randomly sleep after ack message
    ack_msg_sleep_ms: u64,
    fuzz_ack_replies: f64, // probability to randomly fuzz ack reply
    recv_ack: f64,    // probability to attempt to recv ack message from peer
}

#[derive(Clone, Serialize, Deserialize)]
struct PeerMessageReplyOptions {
    bootstrap: f64,                 // probability to reply to Bootstrap requests
    get_current_branch: f64,        // probability to reply to GetCurrentBranch requests
    get_block_headers: f64,         // probability to reply to GetBlockHeaders requests
    get_operations_for_blocks: f64, // probability to reply to GetOperationsForBlocks requests
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
    max_push_throughput: u64,      // max number of messages send in burst
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

async fn rand_sleep(duration: u64, generator: &mut generator::RandomState) {
    if duration > 0 {
        sleep(Duration::from_millis(duration)).await;
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
    let chunks = match generator.gen_bool(options.split_bytes) {
        true => generator.gen_byte_chunks(bytes.clone()),
        false => vec![bytes.clone()],
    };

    let incomplete_send = generator.gen_bool(options.incomplete_send);

    for chunk in chunks {
        stream.write_all(chunk.as_slice()).await?;

        if incomplete_send {
            return Ok(());
        }

        if generator.gen_bool(options.write_bytes_sleep) {
            rand_sleep(options.write_bytes_sleep_ms, generator).await;
        }
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
    let bytes = if generator.gen_bool(options.flip_bits) {
        let mut remaining_flips = generator.gen_range(0 .. bytes.len()) >> 3;
        let mut mutated = Vec::new();
        
        for b in bytes {
            if remaining_flips > 0 && generator.gen_t::<bool>() {
                remaining_flips -= 1;
                mutated.push(*b ^ (1 << generator.gen_range(0 .. 8)));
            }
            else {
                mutated.push(*b);
            }
        }
        mutated
    }
    else {
        bytes.clone()
    };

    let chunks = match generator.gen_bool(options.split_chunks) {
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
    let mut remaining = generator.gen_range(1..options.max_push_throughput);
    eprintln!("Pushing {} messages in burst...", remaining);

    loop {
        if options.messages_per_chunk > remaining {
            break;
        }

        remaining -= options.messages_per_chunk;
        let mut msg_batch: Vec<u8> = Vec::new();

        for _ in 0..options.messages_per_chunk {
            let index = generator.gen_range(0..messages.len());
            let msg = match &messages.get(index).unwrap()[..] {
                "Disconnect" => PeerMessage::Disconnect,
                "Bootstrap" => PeerMessage::Bootstrap,
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

            if let PeerMessage::BlockHeader(bh) = msg.clone() {
                eprintln!("msg {:?}",  bh.block_header().timestamp());
            }
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
            PeerMessage::Bootstrap => {
                if generator.gen_bool(reply_options.bootstrap) {
                    let msg: advertise::AdvertiseMessage = generator.gen();
                    eprintln!("sending Advertise");
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
            },
            PeerMessage::GetCurrentBranch(current_branch::GetCurrentBranchMessage { chain_id }) => {
                generator.set_chain_id(chain_id);

                if generator.gen_bool(reply_options.get_current_branch) {
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
            },
            PeerMessage::GetBlockHeaders(bh) => {
                if generator.gen_bool(reply_options.get_block_headers) {
                    for block_hash in bh.get_block_headers() {
                        let msg = generator.get_block(block_hash);
                        if msg.is_some() {
                            eprintln!("sending BlockHeader");
                            write_msg(
                                &options.chunk_options,
                                generator,
                                &PeerMessageResponse::from(msg.unwrap()),
                                stream,
                                &key,
                                local,
                            )
                            .await?;    
                        }
                    }
                }
            },
            PeerMessage::GetOperationsForBlocks(ops) => {
                if generator.gen_bool(reply_options.get_operations_for_blocks) {
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
            },
            PeerMessage::CurrentBranch(current_branch) => {
                eprintln!("saving CurrentBranch");
                generator.set_current_branch(current_branch);
            },
            PeerMessage::CurrentHead(current_head) => {
                eprintln!("saving CurrentHead");
                generator.set_chain_id(current_head.chain_id().clone());
                generator.set_current_head(current_head);
            },
            PeerMessage::BlockHeader(block_header) => {
                eprintln!("saving BlockHeader");
                generator.set_block_header(block_header);
            },
            PeerMessage::Operation(operation) => {
                eprintln!("saving Operation");
                generator.set_operation(operation);
            },
            PeerMessage::Protocol(protocol) => {
                eprintln!("saving Protocol");
                generator.set_protocol(protocol);
            },
            PeerMessage::OperationsForBlocks(operations_for_blocks) => {
                eprintln!("saving OperationsForBlocks");
                generator.set_operations_for_blocks(operations_for_blocks);
            },
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
                flip_bits: 0.01,
                split_chunks: 0.01,
                split_bytes: 0.01,
                incomplete_send: 0.01,
                write_bytes_sleep: 0.01, 
                write_bytes_sleep_ms: 20000,
            },
            abort_before_connect: 0.01,
            connect_sleep: 0.01,
            connect_sleep_ms: 20000,
            connect_msg_sleep: 0.01,
            connect_msg_sleep_ms: 20000,
            fuzz_public_key: 0.01,
            fuzz_pow: 0.01,
            fuzz_network_version: 0.01,
            abort_before_metadata: 0.01,
            metadata_msg_sleep: 0.01,
            metadata_msg_sleep_ms: 20000,
            recv_metadata: 0.9,
            fuzz_metadata: 0.01,
            abort_before_ack: 0.01,
            ack_msg_sleep: 0.01,
            ack_msg_sleep_ms: 20000,
            fuzz_ack_replies: 0.01,
            recv_ack: 0.9,
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

    if generator.gen_bool(options.abort_before_connect) {
        return Ok(());
    }

    if generator.gen_bool(options.connect_sleep) {
        rand_sleep(options.connect_sleep_ms, generator).await;
    }

    let version = match generator.gen_bool(options.fuzz_network_version) {
        false => NetworkVersion::new("TEZOS_MAINNET".to_string(), 0, 1),
        true => generator.gen(),
    };

    let public_key = match generator.gen_bool(options.fuzz_public_key) {
        false => identity.public_key.clone(),
        true => generator.gen(),
    };

    let pow = match generator.gen_bool(options.fuzz_pow) {
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

    if generator.gen_bool(options.abort_before_metadata) {
        return Ok(());
    }

    if generator.gen_bool(options.connect_msg_sleep) {
        rand_sleep(options.connect_msg_sleep_ms, generator).await;
    }

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

    let metadata = match generator.gen_bool(options.fuzz_metadata) {
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

    if generator.gen_bool(options.recv_metadata) {
        let _m =
            read_msg::<MetadataMessage>(&mut stream, &mut buffer, &key, &mut remote, false).await?;
        //eprintln!("metadata {:?}", _m);
    }

    if generator.gen_bool(options.abort_before_ack) {
        return Ok(());
    }

    if generator.gen_bool(options.metadata_msg_sleep) {
        rand_sleep(options.metadata_msg_sleep_ms, generator).await;
    }

    let ack = if generator.gen_bool(options.fuzz_ack_replies) {
        match generator.gen_bool(0.5) {
            true => AckMessage::NackV0,
            _ => AckMessage::Nack(generator.gen())
        }   
    }
    else
    {
        AckMessage::Ack
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

    if generator.gen_bool(options.ack_msg_sleep) {
        rand_sleep(options.ack_msg_sleep_ms, generator).await;
    }

    if generator.gen_bool(options.recv_ack) {
        //eprintln!("recv ack");
        read_msg::<AckMessage>(&mut stream, &mut buffer, &key, &mut remote, false).await?;
    }

    // TODO: send get current head/current branch?

    if let Some(peer_message_options) = profile.peer_message_fuzzing.borrow() {
        loop {
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

                if Some(111) == e.raw_os_error() {
                    return;
                } 
                
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

    let config = std::fs::read_to_string(matches.value_of("config").unwrap()).unwrap();
    let identities = serde_json::from_str::<Vec<Credentials>>(&
        std::fs::read_to_string(
            matches.value_of("identities").unwrap()
        )
        .unwrap())
        .unwrap()
        .iter()
        .map(|x| {
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
