use std::{thread, time};
use std::ops::Range;
pub use generator::{RandomState, Generator};
use std::net::{SocketAddr, TcpListener, TcpStream};
use tester::{handshake, Message, ChunkBuffer};
//use tester::{RandomState, Generator};
use crypto::nonce::{NoncePair, Nonce, generate_nonces};

use tezos_messages::p2p::encoding::{
    metadata::MetadataMessage,
    ack::AckMessage,
    //peer::{PeerMessage, PeerMessageResponse},
};

fn try_handshake(mut generator: RandomState) -> bool
{
    let addr = SocketAddr::from(([127, 0, 0, 1], 9732));
    let mut stream = TcpStream::connect(addr).unwrap();
    let (ichunk, pkey) = handshake::connect(&mut stream, generator.clone());
    let ret = handshake::recv_connect(&mut stream, pkey);

    if let None = ret {
        return false;
    }

    let (rchunk, key) = ret.unwrap();
    
    let NoncePair { local, remote } = generate_nonces(
        ichunk.raw(), &rchunk.raw(), false).unwrap();

    let mut buffer = ChunkBuffer::default();
    let mut local = local.clone();
    let mut remote = remote.clone();

    //eprintln!("send metadata");
    let metadata: MetadataMessage = generator.gen();
    local = metadata.write_msg(&mut stream, &key, local);

    //eprintln!("recv metadata");
    /*
    match MetadataMessage::read_msg(&mut stream, &mut buffer, &key, remote, false) {
        Some((remote, _msg)) => {*/
            //eprintln!("send ack");
            let ack: AckMessage = generator.gen();
            local = ack.write_msg(&mut stream, &key, local);
    
            /*let (_, _msg) =
                AckMessage::read_msg(&mut stream, &mut buffer, &key, remote, false).unwrap();            
        },
        None => (), 
    }*/
    return true;
}

fn main() {
    let mut generator = RandomState::new(12345, 0);
    loop {
        if try_handshake(generator.clone()) == false {
            break;
        }
        //thread::sleep(time::Duration::from_millis(500));
    }
}
