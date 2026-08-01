use std::collections::{BTreeMap, BTreeSet, VecDeque};

use bincode::Options;

use crate::{
    NetworkError, NetworkMessage, NetworkPacket, NetworkRole, NetworkTransport, PeerId,
    NETWORK_PROTOCOL_VERSION,
};

const MAX_PEERS: usize = 256;
const MAX_RECEIVES_PER_POLL: usize = 512;
const MAX_PENDING_RELIABLE: usize = 256;
const MAX_RELIABLE_ATTEMPTS: u8 = 8;
const RELIABLE_RETRY_SECONDS: f64 = 0.25;
const REPLAY_WINDOW: u64 = 1_024;

#[derive(Clone, Debug)]
pub struct PeerState<A> {
    pub address: A,
    pub display_name: String,
    pub last_received_sequence: u64,
    pub last_seen_seconds: f64,
    recent_sequences: BTreeSet<u64>,
}

struct PendingReliable<A> {
    address: A,
    bytes: Vec<u8>,
    last_sent_seconds: f64,
    attempts: u8,
}

pub struct NetworkSession<T: NetworkTransport> {
    role: NetworkRole,
    local_peer: PeerId,
    session_id: u64,
    transport: T,
    server_address: Option<T::Address>,
    peers: BTreeMap<PeerId, PeerState<T::Address>>,
    addresses: BTreeMap<T::Address, PeerId>,
    incoming: VecDeque<(PeerId, NetworkMessage)>,
    next_sequence: u64,
    next_peer_id: u64,
    pending_reliable: BTreeMap<(T::Address, u64), PendingReliable<T::Address>>,
    clock_seconds: f64,
}

impl<T: NetworkTransport> NetworkSession<T> {
    pub fn host(transport: T, role: NetworkRole, session_id: u64) -> Result<Self, NetworkError> {
        if !role.is_authority() {
            return Err(NetworkError::AuthorityRequired);
        }
        Ok(Self::new(transport, role, PeerId(1), session_id, None))
    }

    pub fn client(transport: T, server: T::Address) -> Self {
        Self::new(transport, NetworkRole::Client, PeerId(0), 0, Some(server))
    }

    fn new(
        transport: T,
        role: NetworkRole,
        local_peer: PeerId,
        session_id: u64,
        server_address: Option<T::Address>,
    ) -> Self {
        Self {
            role,
            local_peer,
            session_id,
            transport,
            server_address,
            peers: BTreeMap::new(),
            addresses: BTreeMap::new(),
            incoming: VecDeque::new(),
            next_sequence: 1,
            next_peer_id: 2,
            pending_reliable: BTreeMap::new(),
            clock_seconds: 0.0,
        }
    }

    pub fn role(&self) -> NetworkRole {
        self.role
    }

    pub fn local_peer(&self) -> PeerId {
        self.local_peer
    }

    pub fn session_id(&self) -> u64 {
        self.session_id
    }

    pub fn peers(&self) -> &BTreeMap<PeerId, PeerState<T::Address>> {
        &self.peers
    }

    pub fn begin_connect(&mut self, display_name: impl Into<String>) -> Result<(), NetworkError> {
        let Some(server) = self.server_address.clone() else {
            return Err(NetworkError::InvalidPacket(
                "session has no server address".into(),
            ));
        };
        self.send_to_address(
            &server,
            NetworkMessage::Hello {
                protocol: NETWORK_PROTOCOL_VERSION,
                display_name: display_name.into(),
            },
        )
    }

    pub fn send_to_peer(
        &mut self,
        peer: PeerId,
        message: NetworkMessage,
    ) -> Result<(), NetworkError> {
        let address = self
            .peers
            .get(&peer)
            .map(|state| state.address.clone())
            .ok_or(NetworkError::UnknownPeer(peer))?;
        self.send_to_address(&address, message)
    }

    pub fn broadcast(&mut self, message: NetworkMessage) -> Result<(), NetworkError> {
        let addresses = self
            .peers
            .values()
            .map(|state| state.address.clone())
            .collect::<Vec<_>>();
        for address in addresses {
            self.send_to_address(&address, message.clone())?;
        }
        Ok(())
    }

    pub fn broadcast_except(
        &mut self,
        excluded: PeerId,
        message: NetworkMessage,
    ) -> Result<(), NetworkError> {
        let addresses = self
            .peers
            .iter()
            .filter(|(peer, _)| **peer != excluded)
            .map(|(_, state)| state.address.clone())
            .collect::<Vec<_>>();
        for address in addresses {
            self.send_to_address(&address, message.clone())?;
        }
        Ok(())
    }

    pub fn remove_peer(&mut self, peer: PeerId) -> bool {
        let Some(state) = self.peers.remove(&peer) else {
            return false;
        };
        self.addresses.remove(&state.address);
        self.pending_reliable
            .retain(|(address, _), _| address != &state.address);
        true
    }

    pub fn pending_reliable_count(&self) -> usize {
        self.pending_reliable.len()
    }

    pub fn poll(&mut self, now_seconds: f64) -> Result<usize, NetworkError> {
        if !now_seconds.is_finite() {
            return Err(NetworkError::InvalidPacket(
                "network clock must be finite".into(),
            ));
        }
        self.clock_seconds = now_seconds;
        let mut received = 0;
        while received < MAX_RECEIVES_PER_POLL {
            let Some((address, bytes)) = self.transport.receive()? else {
                break;
            };
            received += 1;
            let Ok(packet) = options().deserialize::<NetworkPacket>(&bytes) else {
                // UDP is untrusted input. A malformed datagram must not abort
                // the authoritative simulation tick.
                continue;
            };
            if packet.protocol != NETWORK_PROTOCOL_VERSION {
                continue;
            }
            if self.role.is_authority() {
                self.accept_or_update_peer(&address, &packet, now_seconds)?;
            } else {
                self.accept_server_packet(&address, &packet, now_seconds)?;
            }
        }
        self.retransmit_due(now_seconds)?;
        Ok(received)
    }

    pub fn drain_messages(&mut self, limit: usize) -> Vec<(PeerId, NetworkMessage)> {
        let count = limit.max(1).min(self.incoming.len());
        self.incoming.drain(..count).collect()
    }

    pub fn disconnect_timed_out(&mut self, now_seconds: f64, timeout_seconds: f64) -> Vec<PeerId> {
        let timed_out = self
            .peers
            .iter()
            .filter(|(_, state)| now_seconds - state.last_seen_seconds > timeout_seconds)
            .map(|(peer, _)| *peer)
            .collect::<Vec<_>>();
        for peer in &timed_out {
            self.remove_peer(*peer);
        }
        timed_out
    }

    fn accept_or_update_peer(
        &mut self,
        address: &T::Address,
        packet: &NetworkPacket,
        now_seconds: f64,
    ) -> Result<(), NetworkError> {
        let peer = if let Some(peer) = self.addresses.get(address).copied() {
            if packet.session_id != self.session_id {
                return Ok(());
            }
            peer
        } else {
            let NetworkMessage::Hello {
                protocol,
                display_name,
            } = &packet.message
            else {
                return Ok(());
            };
            if *protocol != NETWORK_PROTOCOL_VERSION {
                return Err(NetworkError::ProtocolMismatch {
                    expected: NETWORK_PROTOCOL_VERSION,
                    received: *protocol,
                });
            }
            if packet.session_id != 0 {
                return Ok(());
            }
            if self.peers.len() >= MAX_PEERS {
                return Err(NetworkError::LimitExceeded("session peers"));
            }
            let peer = PeerId(self.next_peer_id);
            self.next_peer_id = self.next_peer_id.saturating_add(1);
            self.addresses.insert(address.clone(), peer);
            self.peers.insert(
                peer,
                PeerState {
                    address: address.clone(),
                    display_name: display_name.chars().take(128).collect(),
                    last_received_sequence: packet.sequence.saturating_sub(1),
                    last_seen_seconds: now_seconds,
                    recent_sequences: BTreeSet::new(),
                },
            );
            self.send_to_address(
                address,
                NetworkMessage::Welcome {
                    peer,
                    session_id: self.session_id,
                },
            )?;
            peer
        };
        self.acknowledge_pending(address, packet.acknowledged_sequence);
        if message_requires_ack(&packet.message) {
            self.send_ack(address, packet.sequence)?;
        }
        if matches!(packet.message, NetworkMessage::Ack) {
            return Ok(());
        }
        if let Some(state) = self.peers.get_mut(&peer) {
            if !accept_sequence(state, packet.sequence) {
                return Ok(());
            }
            state.last_seen_seconds = now_seconds;
        }
        self.incoming.push_back((peer, packet.message.clone()));
        Ok(())
    }

    fn accept_server_packet(
        &mut self,
        address: &T::Address,
        packet: &NetworkPacket,
        now_seconds: f64,
    ) -> Result<(), NetworkError> {
        if self.server_address.as_ref() != Some(address) {
            return Ok(());
        }
        if self.local_peer == PeerId(0) {
            if matches!(packet.message, NetworkMessage::Ack) {
                self.acknowledge_pending(address, packet.acknowledged_sequence);
                return Ok(());
            }
            let NetworkMessage::Welcome { peer, session_id } = packet.message else {
                return Ok(());
            };
            if session_id == 0 || packet.session_id != session_id {
                return Ok(());
            }
            self.local_peer = peer;
            self.session_id = session_id;
        } else if packet.session_id != self.session_id {
            return Ok(());
        }
        self.acknowledge_pending(address, packet.acknowledged_sequence);
        if message_requires_ack(&packet.message) {
            self.send_ack(address, packet.sequence)?;
        }
        if matches!(packet.message, NetworkMessage::Ack) {
            return Ok(());
        }
        let server = PeerId(1);
        let state = self.peers.entry(server).or_insert_with(|| PeerState {
            address: address.clone(),
            display_name: "server".into(),
            last_received_sequence: 0,
            last_seen_seconds: now_seconds,
            recent_sequences: BTreeSet::new(),
        });
        if !accept_sequence(state, packet.sequence) {
            return Ok(());
        }
        state.last_seen_seconds = now_seconds;
        self.incoming.push_back((server, packet.message.clone()));
        Ok(())
    }

    fn send_to_address(
        &mut self,
        address: &T::Address,
        message: NetworkMessage,
    ) -> Result<(), NetworkError> {
        let reliable = message_requires_ack(&message);
        if reliable && self.pending_reliable.len() >= MAX_PENDING_RELIABLE {
            return Err(NetworkError::LimitExceeded("pending reliable packets"));
        }
        let sequence = self.next_sequence;
        let packet = NetworkPacket {
            protocol: NETWORK_PROTOCOL_VERSION,
            session_id: self.session_id,
            sender: self.local_peer,
            sequence,
            acknowledged_sequence: 0,
            message,
        };
        self.next_sequence = self.next_sequence.saturating_add(1);
        let bytes = options()
            .serialize(&packet)
            .map_err(|error| NetworkError::InvalidPacket(error.to_string()))?;
        self.transport.send(address, &bytes)?;
        if reliable {
            self.pending_reliable.insert(
                (address.clone(), sequence),
                PendingReliable {
                    address: address.clone(),
                    bytes,
                    last_sent_seconds: self.clock_seconds,
                    attempts: 1,
                },
            );
        }
        Ok(())
    }

    fn send_ack(
        &mut self,
        address: &T::Address,
        acknowledged_sequence: u64,
    ) -> Result<(), NetworkError> {
        let packet = NetworkPacket {
            protocol: NETWORK_PROTOCOL_VERSION,
            session_id: self.session_id,
            sender: self.local_peer,
            sequence: self.next_sequence,
            acknowledged_sequence,
            message: NetworkMessage::Ack,
        };
        self.next_sequence = self.next_sequence.saturating_add(1);
        let bytes = options()
            .serialize(&packet)
            .map_err(|error| NetworkError::InvalidPacket(error.to_string()))?;
        self.transport.send(address, &bytes)
    }

    fn acknowledge_pending(&mut self, address: &T::Address, sequence: u64) {
        if sequence != 0 {
            self.pending_reliable.remove(&(address.clone(), sequence));
        }
    }

    fn retransmit_due(&mut self, now_seconds: f64) -> Result<(), NetworkError> {
        let due = self
            .pending_reliable
            .iter()
            .filter(|(_, pending)| {
                now_seconds - pending.last_sent_seconds >= RELIABLE_RETRY_SECONDS
            })
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        for key in due {
            let Some(pending) = self.pending_reliable.get_mut(&key) else {
                continue;
            };
            if pending.attempts >= MAX_RELIABLE_ATTEMPTS {
                self.pending_reliable.remove(&key);
                return Err(NetworkError::ReliableDeliveryFailed { sequence: key.1 });
            }
            self.transport.send(&pending.address, &pending.bytes)?;
            pending.last_sent_seconds = now_seconds;
            pending.attempts += 1;
        }
        Ok(())
    }
}

fn message_requires_ack(message: &NetworkMessage) -> bool {
    !matches!(
        message,
        NetworkMessage::Ack
            | NetworkMessage::Ping { .. }
            | NetworkMessage::Pong { .. }
            | NetworkMessage::Rpc(crate::RpcEnvelope {
                reliable: false,
                ..
            })
    )
}

fn accept_sequence<A>(state: &mut PeerState<A>, sequence: u64) -> bool {
    if sequence == 0
        || state.recent_sequences.contains(&sequence)
        || sequence.saturating_add(REPLAY_WINDOW) < state.last_received_sequence
    {
        return false;
    }
    state.recent_sequences.insert(sequence);
    state.last_received_sequence = state.last_received_sequence.max(sequence);
    let oldest = state.last_received_sequence.saturating_sub(REPLAY_WINDOW);
    state
        .recent_sequences
        .retain(|received| *received >= oldest);
    true
}

fn options() -> impl Options {
    bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_limit(crate::MAX_NETWORK_PACKET_BYTES as u64)
        .reject_trailing_bytes()
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    type Inbox = BTreeMap<u8, VecDeque<(u8, Vec<u8>)>>;

    struct MemoryTransport {
        address: u8,
        inboxes: Arc<Mutex<Inbox>>,
    }

    impl MemoryTransport {
        fn pair() -> (Self, Self) {
            let inboxes = Arc::new(Mutex::new(BTreeMap::from([
                (1, VecDeque::new()),
                (2, VecDeque::new()),
            ])));
            (
                Self {
                    address: 1,
                    inboxes: Arc::clone(&inboxes),
                },
                Self {
                    address: 2,
                    inboxes,
                },
            )
        }
    }

    impl NetworkTransport for MemoryTransport {
        type Address = u8;

        fn send(
            &mut self,
            destination: &Self::Address,
            payload: &[u8],
        ) -> Result<(), NetworkError> {
            self.inboxes
                .lock()
                .unwrap()
                .get_mut(destination)
                .ok_or_else(|| NetworkError::Transport("unknown memory address".into()))?
                .push_back((self.address, payload.to_vec()));
            Ok(())
        }

        fn receive(
            &mut self,
        ) -> Result<Option<crate::ReceivedPacket<Self::Address>>, NetworkError> {
            Ok(self
                .inboxes
                .lock()
                .unwrap()
                .get_mut(&self.address)
                .and_then(VecDeque::pop_front))
        }

        fn local_address(&self) -> Result<Self::Address, NetworkError> {
            Ok(self.address)
        }
    }

    #[test]
    fn authority_handshake_assigns_peer_and_routes_messages() {
        let (server_transport, client_transport) = MemoryTransport::pair();
        let mut server =
            NetworkSession::host(server_transport, NetworkRole::AuthoritativeServer, 0xCAFE)
                .unwrap();
        let mut client = NetworkSession::client(client_transport, 1);

        client.begin_connect("pilot").unwrap();
        assert_eq!(server.poll(1.0).unwrap(), 1);
        assert_eq!(server.peers().len(), 1);
        assert_eq!(client.poll(1.1).unwrap(), 2);
        assert_eq!(client.local_peer(), PeerId(2));
        assert_eq!(client.session_id(), 0xCAFE);

        client
            .send_to_peer(PeerId(1), NetworkMessage::Ping { nonce: 77 })
            .unwrap();
        assert_eq!(server.poll(1.2).unwrap(), 2);
        assert!(server
            .drain_messages(8)
            .iter()
            .any(|(peer, message)| *peer == PeerId(2)
                && *message == NetworkMessage::Ping { nonce: 77 }));
    }

    #[test]
    fn reliable_rpc_retransmits_after_loss_and_stops_after_ack() {
        let (server_transport, client_transport) = MemoryTransport::pair();
        let inboxes = Arc::clone(&client_transport.inboxes);
        let mut server =
            NetworkSession::host(server_transport, NetworkRole::AuthoritativeServer, 0xCAFE)
                .unwrap();
        let mut client = NetworkSession::client(client_transport, 1);
        client.begin_connect("pilot").unwrap();
        server.poll(1.0).unwrap();
        client.poll(1.1).unwrap();
        server.poll(1.2).unwrap();
        assert_eq!(client.pending_reliable_count(), 0);
        assert_eq!(server.pending_reliable_count(), 0);

        client
            .send_to_peer(
                PeerId(1),
                NetworkMessage::Rpc(crate::RpcEnvelope {
                    rpc_id: 7,
                    sender: PeerId(2),
                    target: crate::RpcTarget::Server,
                    method: "terrain.apply".into(),
                    reliable: true,
                    payload: vec![1, 2, 3],
                }),
            )
            .unwrap();
        assert_eq!(client.pending_reliable_count(), 1);
        // Drop the first datagram, then advance the client's retry clock.
        assert!(inboxes
            .lock()
            .unwrap()
            .get_mut(&1)
            .unwrap()
            .pop_front()
            .is_some());
        client.poll(1.5).unwrap();
        assert_eq!(server.poll(1.6).unwrap(), 1);
        assert!(server.drain_messages(8).iter().any(
            |(_, message)| matches!(message, NetworkMessage::Rpc(envelope) if envelope.rpc_id == 7)
        ));
        client.poll(1.7).unwrap();
        assert_eq!(client.pending_reliable_count(), 0);
    }

    #[test]
    fn replay_window_accepts_reordered_packets_once() {
        let (server_transport, client_transport) = MemoryTransport::pair();
        let inboxes = Arc::clone(&client_transport.inboxes);
        let mut server =
            NetworkSession::host(server_transport, NetworkRole::AuthoritativeServer, 0xCAFE)
                .unwrap();
        let mut client = NetworkSession::client(client_transport, 1);
        client.begin_connect("pilot").unwrap();
        server.poll(1.0).unwrap();
        client.poll(1.1).unwrap();
        server.poll(1.2).unwrap();

        let encode = |sequence, nonce| {
            options()
                .serialize(&NetworkPacket {
                    protocol: NETWORK_PROTOCOL_VERSION,
                    session_id: 0xCAFE,
                    sender: PeerId(1),
                    sequence,
                    acknowledged_sequence: 0,
                    message: NetworkMessage::Pong { nonce },
                })
                .unwrap()
        };
        let mut guard = inboxes.lock().unwrap();
        guard.get_mut(&2).unwrap().push_back((1, encode(100, 100)));
        guard.get_mut(&2).unwrap().push_back((1, encode(99, 99)));
        guard.get_mut(&2).unwrap().push_back((1, encode(99, 99)));
        drop(guard);
        assert_eq!(client.poll(2.0).unwrap(), 3);
        let messages = client.drain_messages(8);
        assert_eq!(
            messages
                .iter()
                .filter(|(_, message)| matches!(message, NetworkMessage::Pong { .. }))
                .count(),
            2
        );
    }
}
