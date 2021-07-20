use std::net::TcpStream;
pub use generator::{RandomState, Generator};


use crypto::{
    nonce::{NoncePair, Nonce, generate_nonces},
    crypto_box::{CryptoKey, PrecomputedKey, PublicKey, SecretKey},
};
use tezos_messages::p2p::{
    binary_message::{BinaryChunk, BinaryRead, BinaryWrite},
    encoding::{
        connection::ConnectionMessage,
        version::NetworkVersion,
    },
};
use super::buffer::ChunkBuffer;

pub fn connect(stream: &mut TcpStream, mut generator: RandomState) -> (BinaryChunk, SecretKey) {
    use tezos_identity::Identity;
    use std::io::Write;

    let identity = Identity::from_json(include_str!("../identity_i.json")).unwrap();
    let version = match true /*generator.gen_t::<bool>()*/ {
        true => NetworkVersion::new("TEZOS_MAINNET".to_string(), 0, 1),
        false => generator.gen(),
    };

    let connection_message: ConnectionMessage = ConnectionMessage::try_new(
        generator.gen_t(),
        &identity.public_key,
        &identity.proof_of_work_stamp,
        Nonce::random(),
        version,
    ).unwrap();

    let temp = connection_message.as_bytes().unwrap();
    let initiator_chunk = BinaryChunk::from_content(&temp).unwrap();
    //eprintln!("sending creds");
    stream.write_all(initiator_chunk.raw()).unwrap();
    (initiator_chunk, identity.secret_key)
}

pub fn recv_connect(stream: &mut TcpStream, sk: SecretKey) -> Option<(BinaryChunk, PrecomputedKey)>
{
    eprintln!("read_chunk");
    let responder_chunk = ChunkBuffer::default().read_chunk(stream)?;
    eprintln!("read_chunk done");

    let connection_message = ConnectionMessage::from_bytes(responder_chunk.content()).unwrap();
    let pk = PublicKey::from_bytes(connection_message.public_key()).unwrap();
    let key = PrecomputedKey::precompute(&pk, &sk);
    Some((responder_chunk, key))
}
