use std::time::{Instant, Duration};
use std::io::{self, Read, Write};
use std::fmt::{self, Debug};
use bytes::Buf;

use crypto::hash::CryptoboxPublicKeyHash;
use tla_sm::{Acceptor, GetRequests};

use tezos_messages::p2p::encoding::peer::{PeerMessage, PeerMessageResponse};
use tezos_messages::p2p::binary_message::{BinaryMessage, BinaryChunk, BinaryChunkError, CONTENT_LENGTH_FIELD_BYTES};
use tezos_messages::p2p::encoding::prelude::{
    ConnectionMessage, MetadataMessage, AckMessage, NetworkVersion,
};
use crate::{Effects, PeerAddress, PeerCrypto, TezedgeRequest, TezedgeState, TezedgeStateWrapper};
use crate::proposals::{
    TickProposal,
    NewPeerConnectProposal,
    PeerReadableProposal,
    PeerWritableProposal,
    SendPeerMessageProposal,
    PeerDisconnectProposal,
    PeerDisconnectedProposal,
    PeerBlacklistProposal,
    PendingRequestProposal, PendingRequestMsg,
};

pub mod mio_manager;
// pub mod sync_manager;

#[derive(Debug, Clone)]
pub enum Notification {
    PeerDisconnected { peer: PeerAddress },
    PeerBlacklisted { peer: PeerAddress },
    MessageReceived { peer: PeerAddress, message: PeerMessageResponse },
    HandshakeSuccessful {
        peer_address: PeerAddress,
        peer_public_key_hash: CryptoboxPublicKeyHash,
        metadata: MetadataMessage,
        network_version: NetworkVersion,
    },
}

#[derive(Debug, Clone)]
pub enum Event<NetE> {
    Tick(Instant),
    Network(NetE),
}

impl<NetE> Event<NetE> {
    pub fn as_event_ref<'a>(&'a self) -> EventRef<'a, NetE> {
        match self {
            Self::Tick(e) => EventRef::Tick(*e),
            Self::Network(e) => EventRef::Network(e),
        }
    }
}

impl<NetE: NetworkEvent> Event<NetE> {
    pub fn time(&self) -> Instant {
        match self {
            Self::Tick(t) => t.clone(),
            Self::Network(e) => e.time(),
        }
    }
}

impl<NetE> From<NetE> for Event<NetE>
    where NetE: NetworkEvent,
{
    fn from(event: NetE) -> Self {
        Self::Network(event)
    }
}

pub type EventRef<'a, NetE> = Event<&'a NetE>;

pub trait NetworkEvent {
    fn is_server_event(&self) -> bool;

    fn is_readable(&self) -> bool;
    fn is_writable(&self) -> bool;

    fn is_read_closed(&self) -> bool;
    fn is_write_closed(&self) -> bool;

    fn time(&self) -> Instant {
        Instant::now()
    }
}

pub trait Events {
    fn set_limit(&mut self, limit: usize);
}

pub struct Peer<S> {
    address: PeerAddress,
    pub stream: S,
}

impl<S> Peer<S> {
    pub fn new(address: PeerAddress, stream: S) -> Self {
        Self {
            address,
            stream,
        }
    }

    pub fn address(&self) -> &PeerAddress {
        &self.address
    }
}

impl<S: Clone> Clone for Peer<S> {
    fn clone(&self) -> Self {
        Self {
            address: self.address.clone(),
            stream: self.stream.clone(),
        }
    }
}

impl<S: Debug> Debug for Peer<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Peer")
            .field("address", &self.address)
            .field("stream", &self.stream)
            .finish()
    }
}

pub trait Manager {
    type Stream: Read + Write;
    type NetworkEvent: NetworkEvent;
    type Events;

    fn start_listening_to_server_events(&mut self);
    fn stop_listening_to_server_events(&mut self);

    fn accept_connection(&mut self, event: &Self::NetworkEvent) -> Option<&mut Peer<Self::Stream>>;

    fn wait_for_events(&mut self, events_container: &mut Self::Events, timeout: Option<Duration>);

    fn get_peer(&mut self, address: &PeerAddress) -> Option<&mut Peer<Self::Stream>>;
    fn get_peer_or_connect_mut(&mut self, address: &PeerAddress) -> io::Result<&mut Peer<Self::Stream>>;
    fn get_peer_for_event_mut(&mut self, event: &Self::NetworkEvent) -> Option<&mut Peer<Self::Stream>>;

    fn disconnect_peer(&mut self, peer: &PeerAddress);
}

#[derive(Clone)]
pub struct TezedgeProposerConfig {
    pub wait_for_events_timeout: Option<Duration>,
    pub events_limit: usize,
}

#[derive(Clone)]
pub struct TezedgeProposer<Es, Efs, M> {
    config: TezedgeProposerConfig,
    requests: Vec<TezedgeRequest>,
    notifications: Vec<Notification>,
    pub state: TezedgeStateWrapper<Efs>,
    pub events: Es,
    pub manager: M,
}

impl<Es, Efs, M> TezedgeProposer<Es, Efs, M>
    where Es: Events,
{
    pub fn new<S>(
        config: TezedgeProposerConfig,
        state: S,
        mut events: Es,
        manager: M,
    ) -> Self
        where S: Into<TezedgeStateWrapper<Efs>>,
    {
        events.set_limit(config.events_limit);
        Self {
            config,
            requests: vec![],
            notifications: vec![],
            state: state.into(),
            events,
            manager,
        }
    }
}

impl<S, NetE, Es, Efs, M> TezedgeProposer<Es, Efs, M>
    where S: Read + Write,
          NetE: NetworkEvent + Debug,
          Efs: Effects,
          M: Manager<Stream = S, NetworkEvent = NetE, Events = Es>,
{
    fn handle_event(
        event: Event<NetE>,
        requests: &mut Vec<TezedgeRequest>,
        notifications: &mut Vec<Notification>,
        state: &mut TezedgeStateWrapper<Efs>,
        manager: &mut M,
    ) {
        match event {
            Event::Tick(at) => {
                state.accept(TickProposal { at });
            }
            Event::Network(event) => {
                Self::handle_network_event(&event, requests, notifications, state, manager);
            }
        }
    }

    fn handle_event_ref<'a>(
        event: EventRef<'a, NetE>,
        requests: &mut Vec<TezedgeRequest>,
        notifications: &mut Vec<Notification>,
        state: &mut TezedgeStateWrapper<Efs>,
        manager: &mut M,
    ) {
        match event {
            Event::Tick(at) => {
                state.accept(TickProposal { at });
            }
            Event::Network(event) => {
                Self::handle_network_event(event, requests, notifications, state, manager);
            }
        }
    }

    fn handle_network_event(
        event: &NetE,
        requests: &mut Vec<TezedgeRequest>,
        notifications: &mut Vec<Notification>,
        state: &mut TezedgeStateWrapper<Efs>,
        manager: &mut M,
    ) {
        if event.is_server_event() {
            // we received event for the server (client opened tcp stream to us).
            loop {
                // as an optimization, execute requests only after 100
                // accepted new connections. We need to execute those
                // requests as they might include command to stop
                // listening for new connections or disconnect new peer,
                // if for example they are blacklisted.
                for _ in 0..100 {
                    match manager.accept_connection(&event) {
                        Some(peer) => {
                            state.accept(NewPeerConnectProposal {
                                at: event.time(),
                                peer: peer.address().clone(),
                            });
                            Self::handle_readiness_event(event, state, peer);
                        }
                        None => return,
                    }
                }
                Self::execute_requests(requests, notifications, state, manager);
            }
        } else {
            match manager.get_peer_for_event_mut(&event) {
                Some(peer) => Self::handle_readiness_event(event, state, peer),
                None => {
                    // TODO: write error log.
                    return;
                }
            }
        };
    }

    fn handle_readiness_event(
        event: &NetE,
        state: &mut TezedgeStateWrapper<Efs>,
        peer: &mut Peer<S>,
    ) {
        if event.is_read_closed() || event.is_write_closed() {
            state.accept(PeerDisconnectedProposal {
                at: event.time(),
                peer: peer.address().clone(),
            });
            return;
        }

        if event.is_readable() {
            state.accept(PeerReadableProposal {
                at: event.time(),
                peer: peer.address().clone(),
                stream: &mut peer.stream,
            });
        }

        if event.is_writable() {
            state.accept(PeerWritableProposal {
                at: event.time(),
                peer: peer.address().clone(),
                stream: &mut peer.stream,
            });
        }
    }

    fn execute_requests(
        requests: &mut Vec<TezedgeRequest>,
        notifications: &mut Vec<Notification>,
        state: &mut TezedgeStateWrapper<Efs>,
        manager: &mut M,
    ) {
        state.get_requests(requests);

        for req in requests.drain(..) {
            match req {
                TezedgeRequest::StartListeningForNewPeers { req_id } => {
                    manager.start_listening_to_server_events();
                    state.accept(PendingRequestProposal {
                        req_id,
                        at: state.newest_time_seen(),
                        message: PendingRequestMsg::StartListeningForNewPeersSuccess,
                    });
                }
                TezedgeRequest::StopListeningForNewPeers { req_id } => {
                    manager.stop_listening_to_server_events();
                    state.accept(PendingRequestProposal {
                        req_id,
                        at: state.newest_time_seen(),
                        message: PendingRequestMsg::StopListeningForNewPeersSuccess,
                    });
                }
                TezedgeRequest::ConnectPeer { req_id, peer } => {
                    match manager.get_peer_or_connect_mut(&peer) {
                        Ok(_) => state.accept(PendingRequestProposal {
                            req_id,
                            at: state.newest_time_seen(),
                            message: PendingRequestMsg::ConnectPeerSuccess,
                        }),
                        Err(_) => state.accept(PendingRequestProposal {
                            req_id,
                            at: state.newest_time_seen(),
                            message: PendingRequestMsg::ConnectPeerError,
                        }),
                    }
                }
                TezedgeRequest::DisconnectPeer { req_id, peer } => {
                    manager.disconnect_peer(&peer);
                    state.accept(PendingRequestProposal {
                        req_id,
                        at: state.newest_time_seen(),
                        message: PendingRequestMsg::DisconnectPeerSuccess,
                    });
                    notifications.push(Notification::PeerDisconnected { peer });
                }
                TezedgeRequest::BlacklistPeer { req_id, peer } => {
                    manager.disconnect_peer(&peer);
                    state.accept(PendingRequestProposal {
                        req_id,
                        at: state.newest_time_seen(),
                        message: PendingRequestMsg::BlacklistPeerSuccess,
                    });
                    notifications.push(Notification::PeerBlacklisted { peer });
                }
                TezedgeRequest::PeerMessageReceived { req_id, peer, message } => {
                    state.accept(PendingRequestProposal {
                        req_id,
                        at: state.newest_time_seen(),
                        message: PendingRequestMsg::PeerMessageReceivedNotified,
                    });
                    notifications.push(Notification::MessageReceived { peer, message });
                }
                TezedgeRequest::NotifyHandshakeSuccessful {
                    req_id, peer_address, peer_public_key_hash, metadata, network_version
                } => {
                    notifications.push(Notification::HandshakeSuccessful {
                        peer_address,
                        peer_public_key_hash,
                        metadata,
                        network_version,
                    });
                    state.accept(PendingRequestProposal {
                        req_id,
                        at: state.newest_time_seen(),
                        message: PendingRequestMsg::HandshakeSuccessfulNotified,
                    });
                }
            }
        }
    }

    fn wait_for_events(&mut self) {
        let wait_for_events_timeout = self.config.wait_for_events_timeout;
        self.manager.wait_for_events(&mut self.events, wait_for_events_timeout)
    }

    pub fn make_progress(&mut self)
        where for<'a> &'a Es: IntoIterator<Item = EventRef<'a, NetE>>,
    {
        self.wait_for_events();

        for event in self.events.into_iter() {
            Self::handle_event_ref(
                event,
                &mut self.requests,
                &mut self.notifications,
                &mut self.state,
                &mut self.manager,
            );
        }

        Self::execute_requests(
            &mut self.requests,
            &mut self.notifications,
            &mut self.state,
            &mut self.manager
        );
    }

    pub fn make_progress_owned(&mut self)
        where for<'a> &'a Es: IntoIterator<Item = Event<NetE>>,
    {
        let time = Instant::now();
        self.wait_for_events();
        eprintln!("waited for events for: {}ms", time.elapsed().as_millis());

        let time = Instant::now();
        let mut count = 0;
        for event in self.events.into_iter() {
            count += 1;
            Self::handle_event(
                event,
                &mut self.requests,
                &mut self.notifications,
                &mut self.state,
                &mut self.manager,
            );
        }
        eprintln!("handled {} events in: {}ms", count, time.elapsed().as_millis());

        let time = Instant::now();
        Self::execute_requests(
            &mut self.requests,
            &mut self.notifications,
            &mut self.state,
            &mut self.manager
        );
        eprintln!("executed requests in: {}ms", time.elapsed().as_millis());
    }

    pub fn disconnect_peer(&mut self, at: Instant, peer: PeerAddress) {
        self.state.accept(PeerDisconnectProposal { at, peer })
    }

    pub fn blacklist_peer(&mut self, at: Instant, peer: PeerAddress) {
        self.state.accept(PeerBlacklistProposal { at, peer })
    }

    // TODO: Everything bellow this line is temporary until everything
    // is handled in TezedgeState.
    // ---------------------------------------------------------------

    pub fn send_message_to_peer_or_queue(
        &mut self,
        at: Instant,
        addr: PeerAddress,
        message: PeerMessage,
    ) -> Result<(), &'static str> {
        if let Some(peer) = self.manager.get_peer(&addr) {
            self.state.accept(SendPeerMessageProposal {
                at,
                peer: addr,
                message,
            });
            // issue writable proposal in case stream is writable,
            // otherwise we would have to wait till we receive
            // writable event from mio.
            self.state.accept(PeerWritableProposal {
                at,
                peer: addr,
                stream: &mut peer.stream,
            });
            Ok(())
        } else {
            Err("queueing send message failed since peer not found!")
        }
    }

    #[cfg(feature = "blocking")]
    pub fn blocking_send(
        &mut self,
        at: Instant,
        addr: PeerAddress,
        message: PeerMessage,
    ) -> io::Result<()>
    {
        let peer = match self.manager.get_peer(&addr) {
            Some(peer) => peer,
            None => return Err(io::Error::new(io::ErrorKind::NotFound, "peer not found!")),
        };

        self.state.accept(SendPeerMessageProposal {
            at,
            peer: addr,
            message,
        });
        let mut send_buf = WriteOnlyBuffer::new();
        self.state.accept(PeerWritableProposal {
            at,
            peer: addr,
            stream: &mut send_buf,
        });


        let send_buf = send_buf.take();
        let mut buf = &send_buf[..];

        while buf.len() > 0 {
            match peer.stream.write(buf) {
                Ok(len) => buf = &buf[len..],
                Err(err) => {
                    match err.kind() {
                        io::ErrorKind::WouldBlock => std::thread::yield_now(),
                        _ => return Err(err),
                    };
                }
            }
        }

        Ok(())
    }

    pub fn take_notifications<'a>(&'a mut self) -> std::vec::Drain<'a, Notification> {
        self.notifications.drain(..)
    }
}

#[cfg(feature = "blocking")]
struct WriteOnlyBuffer {
    buf: Vec<u8>,
}

#[cfg(feature = "blocking")]
impl WriteOnlyBuffer {
    fn new() -> Self {
        Self { buf: vec![] }
    }

    fn take(self) -> Vec<u8> {
        self.buf
    }
}

#[cfg(feature = "blocking")]
impl Write for WriteOnlyBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buf.extend(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(feature = "blocking")]
impl Read for WriteOnlyBuffer {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::new(io::ErrorKind::WouldBlock, "write only buffer!"))
    }
}
