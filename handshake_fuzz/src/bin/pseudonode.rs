use clap::{App, Arg};
use serde::{Deserialize, Serialize};
use hex::FromHex;
use generator::{RandomState, Generator};

use std::sync::Arc;
use std::convert::{TryInto, TryFrom};
use tokio::io::{self, AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::{sleep, Duration};
use futures::future::join_all;
use std::io::{Result, Error, ErrorKind};

use crypto::{
    nonce::{Nonce, NoncePair, generate_nonces},
    hash::CryptoboxPublicKeyHash,
    proof_of_work::ProofOfWork,
    crypto_box::{CryptoKey, PrecomputedKey, PublicKey, SecretKey},
};
use tezos_identity::Identity;
use tezos_messages::p2p::{
    binary_message::{BinaryMessage, BinaryChunk, BinaryRead, BinaryWrite},
    encoding::{
        connection::ConnectionMessage,
        version::NetworkVersion,
        metadata::MetadataMessage,
        ack::AckMessage,
        //peer::{PeerMessage, PeerMessageResponse},    
    },
};

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
    threads: u64,
    max_connect_sleep: u64,
    max_connect_msg_sleep: u64,
    max_metadata_msg_sleep: u64,
    max_ack_msg_sleep: u64,
    always_send_connect: bool,
    always_send_metadata: bool,
    always_send_ack: bool,
    fuzz_public_key: bool,
    fuzz_pow: bool,
    fuzz_network_version: bool,
    always_recv_metadata: bool,
    always_recv_ack: bool,
    send_ack: Vec<String>,
}

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
    "threads": 60000,
    "max_connect_sleep": 2000,
    "max_connect_msg_sleep": 2000,
    "max_metadata_msg_sleep": 2000,
    "max_ack_msg_sleep": 2000,
    "always_send_connect": true,
    "always_send_metadata": true,
    "always_send_ack": true,
    "fuzz_public_key": false,
    "fuzz_pow": false,
    "fuzz_network_version": true,
    "always_recv_metadata": false,
    "always_recv_ack": false,
    "send_ack": ["Ack", "NackV0", "Nack"]
}"#;

pub struct ChunkBuffer {
    len: usize,
    data:[u8; 0x10000],
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
            self.len += stream.read(&mut self.data[self.len..]).await?;
           
            if self.len == 0 {
                if retry > 2 {
                    return Err(Error::new(ErrorKind::UnexpectedEof, "zero length read"));
                }
                
                retry += 1;
                sleep(Duration::from_millis(100)).await;                
            }

            if self.len >= HEADER_LENGTH {
                let chunk_len = u16::from_be_bytes([self.data[0], self.data[1]]) as usize;
                let raw_len = chunk_len + HEADER_LENGTH;
            
                if self.len >= raw_len {
                    let chunk = self.data[..(raw_len)].to_vec();
                    for i in raw_len..self.len {
                        self.data[(i - raw_len)] = self.data[i];
                    }
                    self.len -= raw_len;

                    return Ok(chunk.try_into().unwrap())
                }
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}


async fn write_msg<M>(
    msg: &M,
    stream: &mut TcpStream,
    key: &PrecomputedKey,
    nonce: Nonce
) -> Nonce
where M: BinaryMessage {
    let bytes = msg.as_bytes().unwrap();
    let mut nonce = nonce;

    for bytes in bytes.as_slice().chunks(0xffe0) {
        let temp = key.encrypt(&bytes, &nonce).unwrap();
        let chunk = BinaryChunk::from_content(&temp).unwrap().raw().clone();
        stream.write_all(&chunk).await.unwrap();
        nonce = nonce.increment();
    }

    nonce
}

async fn read_msg<M>(
    stream: &mut TcpStream,
    buffer: &mut ChunkBuffer,
    key: &PrecomputedKey,
    nonce: Nonce,
    peer_message: bool,
) -> Result<(Nonce, M)>
where M: BinaryMessage
{
    const HEADER_LENGTH: usize = 4;
    let mut nonce = nonce;
    let mut bytes = Vec::new();
    let mut length = 0;

    loop {
        let chunk = buffer.read_chunk(stream).await?;
        let decrypted = key.decrypt(chunk.content(), &nonce).unwrap();

        bytes.extend_from_slice(&decrypted.as_slice());

        if length == 0 && peer_message {
            let b = TryFrom::try_from(&bytes[..HEADER_LENGTH]).unwrap();
            length = u32::from_be_bytes(b) as usize + HEADER_LENGTH;
        }
        nonce = nonce.increment();

        if bytes.len() == length || !peer_message {
            return Ok((nonce, M::from_bytes(bytes).unwrap()))
        }
    }
}


async fn fuzz_handshake(profile: &Profile, mut generator: generator::RandomState) -> Result<()>
{
    let mut stream = TcpStream::connect(profile.peer.clone()).await?;
    //eprintln!("connected");
    if profile.max_connect_sleep > 0 {
        sleep(Duration::from_millis(generator.gen_range(0..profile.max_connect_sleep))).await;
    }
    

    let identity = Identity {
        peer_id: CryptoboxPublicKeyHash::from_base58_check(&profile.id.peer_id).unwrap(),
        public_key: PublicKey::from_hex(&profile.id.public_key).unwrap(),
        secret_key: SecretKey::from_hex(&profile.id.private_key).unwrap(),
        proof_of_work_stamp: ProofOfWork::from_hex(&profile.id.pow).unwrap(),
    };

    if !(profile.always_send_connect || generator.gen_t::<bool>()) {
        return Ok(());
    }

    let version = match profile.fuzz_network_version && generator.gen_t::<bool>() {
        false => NetworkVersion::new(
            "TEZOS_MAINNET".to_string(), 
            0, 
            1),
        true => generator.gen(),
    };

    let public_key = match profile.fuzz_public_key && generator.gen_t::<bool>() {
        false => identity.public_key,
        true => generator.gen(),
    };

    let pow = match profile.fuzz_pow && generator.gen_t::<bool>() {
        false => identity.proof_of_work_stamp,
        true => generator.gen(),
    };

    let connection_message: ConnectionMessage = ConnectionMessage::try_new(
        generator.gen_t(),
        &public_key,
        &pow,
        generator.gen(),
        version
    ).unwrap();

    let initiator_chunk = BinaryChunk::from_content(
        &connection_message.as_bytes().unwrap()
    ).unwrap();
    stream.write_all(initiator_chunk.raw()).await?;

    if profile.max_connect_msg_sleep > 0 {
        sleep(Duration::from_millis(generator.gen_range(0..profile.max_connect_msg_sleep))).await;
    }

    if !(profile.always_send_metadata || generator.gen_t::<bool>()) {
        return Ok(());
    }

    let mut buffer = ChunkBuffer::default();
    let responder_chunk = buffer.read_chunk(&mut stream).await?;

    let connection_message = ConnectionMessage::from_bytes(
        responder_chunk.content()
    ).unwrap();

    let key = PrecomputedKey::precompute(
        &PublicKey::from_bytes(connection_message.public_key()).unwrap(),
        &identity.secret_key
    );

    let NoncePair { mut local, mut remote } = generate_nonces(
        &initiator_chunk.raw(),
        &responder_chunk.raw(),
        false
    ).unwrap();

    //eprintln!("send metadata");
    let metadata: MetadataMessage = generator.gen();
    local = write_msg(&metadata, &mut stream, &key, local).await;

    if profile.max_metadata_msg_sleep > 0 {
        sleep(Duration::from_millis(generator.gen_range(0..profile.max_metadata_msg_sleep))).await;
    }

    if !(profile.always_send_ack || generator.gen_t::<bool>()) {
        return Ok(());
    }

    if profile.always_recv_metadata || generator.gen_t::<bool>() {
        //eprintln!("recv metadata");
        let (nonce, _) = read_msg::<MetadataMessage>(
            &mut stream,
            &mut buffer,
            &key,
            remote,
            false).await?;
        remote = nonce;
    }

    let index = generator.gen_range(0..profile.send_ack.len());

    let ack: AckMessage = match &profile.send_ack.get(index).unwrap()[..] {
        "Ack" => AckMessage::Ack,
        "NackV0" => AckMessage::NackV0,
        _ => AckMessage::Nack(generator.gen()),
    };
    write_msg(&ack, &mut stream, &key, local).await;

    if profile.max_ack_msg_sleep > 0 {
        sleep(Duration::from_millis(generator.gen_range(0..profile.max_ack_msg_sleep))).await;
    }

    if (profile.always_recv_metadata && profile.always_recv_ack) || generator.gen_t::<bool>() {
        let (_nonce, _) = read_msg::<AckMessage>(
            &mut stream, 
            &mut buffer, 
            &key, 
            remote, 
            false
        ).await?;
        //remote = _nonce;
    }
    Ok(())
}


async fn worker(profile: Arc<Profile>, task_id: u64) {
    let generator = RandomState::new(profile.prng_seed * task_id, 0);
    loop {
        match fuzz_handshake(&profile, generator.clone()).await {
            Err(e) => {
                //eprintln!("ERROR: {}", e);
                //break
            },
            _ => (),
        }
    }
}

#[tokio::main]
async fn main() {
    let matches = App::new("handshake-fuzzer")
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

    let max_fd = fdlimit::raise_fd_limit().unwrap();
    eprintln!("current fd limit {}", max_fd);
    
    match serde_json::from_str::<Profile>(&config) {
        Ok(mut profile) => {
            profile.threads = std::cmp::min(profile.threads, max_fd);

            let profile = Arc::new(profile);
            let mut tasks = Vec::new();
            
            for n in 0..profile.threads {
                let profile = Arc::clone(&profile);
             
                tasks.push(
                    tokio::spawn( async move {
                        //eprintln!("spawning thread");
                        worker(profile, n).await
                    })
                );
            }

            join_all(tasks).await;
        },
        Err(e) => eprintln!("error {}", e),
    }
}
