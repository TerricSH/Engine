use std::net::SocketAddr;

use crate::{
    FriendPresence, InMemoryLobbyBackend, LobbyBackend, NetworkEntityId, NetworkError,
    NetworkMessage, NetworkRole, NetworkSession, PeerId, ReplicationRegistry, RpcRouter, RpcTarget,
    UdpTransport,
};

/// Engine-facing aggregate that owns transport, replication and RPC queues.
pub struct NetworkRuntime {
    session: Option<NetworkSession<UdpTransport>>,
    pub replication: ReplicationRegistry,
    pub rpc: RpcRouter,
    pub lobby: Box<dyn LobbyBackend>,
    pub friends: FriendPresence,
}

impl Default for NetworkRuntime {
    fn default() -> Self {
        Self {
            session: None,
            replication: ReplicationRegistry::new(NetworkRole::Client, PeerId(0)),
            rpc: RpcRouter::default(),
            lobby: Box::<InMemoryLobbyBackend>::default(),
            friends: FriendPresence::default(),
        }
    }
}

impl NetworkRuntime {
    pub fn host(
        &mut self,
        bind: SocketAddr,
        session_id: u64,
        listen_server: bool,
    ) -> Result<SocketAddr, NetworkError> {
        let transport = UdpTransport::bind(bind)?;
        let local = crate::NetworkTransport::local_address(&transport)?;
        let role = if listen_server {
            NetworkRole::ListenServer
        } else {
            NetworkRole::AuthoritativeServer
        };
        self.session = Some(NetworkSession::host(transport, role, session_id)?);
        self.replication = ReplicationRegistry::new(role, PeerId(1));
        Ok(local)
    }

    pub fn connect(
        &mut self,
        bind: SocketAddr,
        server: SocketAddr,
        display_name: impl Into<String>,
    ) -> Result<SocketAddr, NetworkError> {
        let transport = UdpTransport::bind(bind)?;
        let local = crate::NetworkTransport::local_address(&transport)?;
        let mut session = NetworkSession::client(transport, server);
        session.begin_connect(display_name)?;
        self.session = Some(session);
        self.replication = ReplicationRegistry::new(NetworkRole::Client, PeerId(0));
        Ok(local)
    }

    pub fn session(&self) -> Option<&NetworkSession<UdpTransport>> {
        self.session.as_ref()
    }

    pub fn disconnect(&mut self) {
        self.session = None;
    }

    pub fn set_lobby_backend(&mut self, backend: Box<dyn LobbyBackend>) {
        self.lobby = backend;
    }

    /// Assign authority over an entity and publish the newer ownership
    /// revision to connected peers.
    pub fn assign_owner(
        &mut self,
        entity: NetworkEntityId,
        owner: Option<PeerId>,
    ) -> Result<u64, NetworkError> {
        let revision = self.replication.set_owner(entity, owner)?;
        if let Some(session) = self.session.as_mut() {
            session.broadcast(NetworkMessage::Ownership {
                entity,
                owner,
                revision,
            })?;
        }
        Ok(revision)
    }

    /// Route an RPC according to its explicit target. Client calls always go
    /// through the authority, which performs final target/ownership routing.
    pub fn send_rpc(
        &mut self,
        target: RpcTarget,
        method: impl Into<String>,
        reliable: bool,
        payload: Vec<u8>,
    ) -> Result<u64, NetworkError> {
        let (role, local_peer) = self
            .session
            .as_ref()
            .map(|session| (session.role(), session.local_peer()))
            .ok_or_else(|| NetworkError::InvalidPacket("network session is not active".into()))?;
        let envelope = self
            .rpc
            .envelope(local_peer, target, method, reliable, payload)?;
        let rpc_id = envelope.rpc_id;
        let session = self
            .session
            .as_mut()
            .ok_or_else(|| NetworkError::InvalidPacket("network session is not active".into()))?;
        if !role.is_authority() {
            session.send_to_peer(PeerId(1), NetworkMessage::Rpc(envelope))?;
            return Ok(rpc_id);
        }
        match target {
            RpcTarget::Server => self.rpc.enqueue(envelope)?,
            RpcTarget::Peer(peer) if peer == local_peer => self.rpc.enqueue(envelope)?,
            RpcTarget::Peer(peer) => session.send_to_peer(peer, NetworkMessage::Rpc(envelope))?,
            RpcTarget::All => {
                self.rpc.enqueue(envelope.clone())?;
                session.broadcast(NetworkMessage::Rpc(envelope))?;
            }
            RpcTarget::Others => session.broadcast(NetworkMessage::Rpc(envelope))?,
            RpcTarget::Owner(entity) => {
                let owner = self
                    .replication
                    .owner(entity)
                    .ok_or_else(|| NetworkError::InvalidPacket("entity has no owner".into()))?;
                if owner == local_peer {
                    self.rpc.enqueue(envelope)?;
                } else {
                    session.send_to_peer(owner, NetworkMessage::Rpc(envelope))?;
                }
            }
        }
        Ok(rpc_id)
    }

    pub fn tick(&mut self, now_seconds: f64) -> Result<usize, NetworkError> {
        let Some(session) = self.session.as_mut() else {
            return Ok(0);
        };
        let count = session.poll(now_seconds)?;
        let messages = session.drain_messages(1024);
        let role = session.role();
        let local_peer = session.local_peer();
        let mut server_disconnected = false;
        for (sender, message) in messages {
            match message {
                NetworkMessage::Ownership {
                    entity,
                    owner,
                    revision,
                } => {
                    if !role.is_authority() {
                        self.replication.apply_owner(entity, owner, revision);
                    }
                }
                NetworkMessage::Replication(updates) => {
                    if role.is_authority() {
                        self.replication.apply_peer_updates(sender, updates);
                    } else {
                        self.replication.apply_updates(updates);
                    }
                }
                NetworkMessage::Rpc(mut envelope) => {
                    envelope.sender = sender;
                    if role.is_authority() {
                        self.route_received_rpc(sender, envelope)?;
                    } else {
                        self.rpc.enqueue(envelope)?;
                    }
                }
                NetworkMessage::Ping { nonce } => {
                    if let Some(session) = self.session.as_mut() {
                        session.send_to_peer(sender, NetworkMessage::Pong { nonce })?;
                    }
                }
                NetworkMessage::Welcome { peer, .. } => {
                    if !role.is_authority() {
                        self.replication = ReplicationRegistry::new(NetworkRole::Client, peer);
                    }
                }
                NetworkMessage::Hello { .. } => {
                    if role.is_authority() {
                        self.send_initial_snapshot(sender)?;
                    }
                }
                NetworkMessage::Disconnect { .. } => {
                    if role.is_authority() {
                        if let Some(session) = self.session.as_mut() {
                            session.remove_peer(sender);
                        }
                    } else if sender == PeerId(1) {
                        server_disconnected = true;
                    }
                }
                NetworkMessage::Pong { .. } | NetworkMessage::Ack => {}
            }
        }
        if server_disconnected {
            self.session = None;
        } else if role.is_authority() && local_peer != PeerId(1) {
            return Err(NetworkError::InvalidPacket(
                "authoritative session has an invalid local peer".into(),
            ));
        }
        Ok(count)
    }

    fn send_initial_snapshot(&mut self, peer: PeerId) -> Result<(), NetworkError> {
        let session = self
            .session
            .as_mut()
            .ok_or_else(|| NetworkError::InvalidPacket("network session is not active".into()))?;
        for (entity, owner, revision) in self.replication.ownership_snapshot() {
            session.send_to_peer(
                peer,
                NetworkMessage::Ownership {
                    entity,
                    owner,
                    revision,
                },
            )?;
        }
        // Send one component per packet so an arbitrary snapshot count cannot
        // exceed the bounded UDP packet size.
        for update in self.replication.snapshot() {
            session.send_to_peer(peer, NetworkMessage::Replication(vec![update]))?;
        }
        Ok(())
    }

    fn route_received_rpc(
        &mut self,
        sender: PeerId,
        envelope: crate::RpcEnvelope,
    ) -> Result<(), NetworkError> {
        let local_peer = self
            .session
            .as_ref()
            .map(NetworkSession::local_peer)
            .ok_or_else(|| NetworkError::InvalidPacket("network session is not active".into()))?;
        match envelope.target {
            RpcTarget::Server => self.rpc.enqueue(envelope),
            RpcTarget::Peer(peer) if peer == local_peer => self.rpc.enqueue(envelope),
            RpcTarget::Peer(peer) => self
                .session
                .as_mut()
                .ok_or_else(|| NetworkError::InvalidPacket("network session is not active".into()))?
                .send_to_peer(peer, NetworkMessage::Rpc(envelope)),
            RpcTarget::All => {
                self.rpc.enqueue(envelope.clone())?;
                self.session
                    .as_mut()
                    .ok_or_else(|| {
                        NetworkError::InvalidPacket("network session is not active".into())
                    })?
                    .broadcast(NetworkMessage::Rpc(envelope))
            }
            RpcTarget::Others => {
                self.rpc.enqueue(envelope.clone())?;
                self.session
                    .as_mut()
                    .ok_or_else(|| {
                        NetworkError::InvalidPacket("network session is not active".into())
                    })?
                    .broadcast_except(sender, NetworkMessage::Rpc(envelope))
            }
            RpcTarget::Owner(entity) => {
                let owner = self
                    .replication
                    .owner(entity)
                    .ok_or_else(|| NetworkError::InvalidPacket("entity has no owner".into()))?;
                if owner == local_peer {
                    self.rpc.enqueue(envelope)
                } else {
                    self.session
                        .as_mut()
                        .ok_or_else(|| {
                            NetworkError::InvalidPacket("network session is not active".into())
                        })?
                        .send_to_peer(owner, NetworkMessage::Rpc(envelope))
                }
            }
        }
    }

    pub fn flush_replication(&mut self, limit: usize) -> Result<usize, NetworkError> {
        let Some(session) = self.session.as_mut() else {
            return Ok(0);
        };
        let updates = self.replication.drain_dirty(limit);
        if updates.is_empty() {
            return Ok(0);
        }
        let mut sent = 0;
        for (index, update) in updates.iter().cloned().enumerate() {
            if let Err(error) = session.broadcast(NetworkMessage::Replication(vec![update])) {
                self.replication
                    .restore_dirty(updates[index..].iter().cloned());
                return Err(error);
            }
            sent += 1;
        }
        Ok(sent)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    use super::*;

    #[test]
    fn client_rpc_reaches_authoritative_server_handler() {
        let mut server = NetworkRuntime::default();
        let address = server
            .host("127.0.0.1:0".parse().unwrap(), 0x1234, false)
            .unwrap();
        let mut client = NetworkRuntime::default();
        client
            .connect("127.0.0.1:0".parse().unwrap(), address, "pilot")
            .unwrap();
        for step in 0..4 {
            server.tick(f64::from(step) * 0.01).unwrap();
            client.tick(f64::from(step) * 0.01 + 0.005).unwrap();
        }
        assert_eq!(client.session().unwrap().local_peer(), PeerId(2));

        let calls = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&calls);
        server
            .rpc
            .register(
                "world.edit",
                Box::new(move |envelope| {
                    assert_eq!(envelope.sender, PeerId(2));
                    assert_eq!(envelope.payload, [7, 8, 9]);
                    observed.fetch_add(1, Ordering::Relaxed);
                    Ok(())
                }),
            )
            .unwrap();
        client
            .send_rpc(RpcTarget::Server, "world.edit", true, vec![7, 8, 9])
            .unwrap();
        for step in 5..9 {
            server.tick(f64::from(step) * 0.01).unwrap();
            client.tick(f64::from(step) * 0.01 + 0.005).unwrap();
        }
        assert_eq!(server.rpc.dispatch(8).len(), 1);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }
}
