use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{NetworkError, PeerId};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LobbyInfo {
    pub id: String,
    pub owner: PeerId,
    pub name: String,
    pub max_members: u16,
    pub members: BTreeSet<PeerId>,
    pub joinable: bool,
    pub metadata: BTreeMap<String, String>,
}

pub trait LobbyBackend: Send {
    fn list(&mut self) -> Result<Vec<LobbyInfo>, NetworkError>;
    fn create(&mut self, lobby: LobbyInfo) -> Result<(), NetworkError>;
    fn join(&mut self, lobby_id: &str, peer: PeerId) -> Result<LobbyInfo, NetworkError>;
    fn leave(&mut self, lobby_id: &str, peer: PeerId) -> Result<LobbyInfo, NetworkError>;
    fn update(&mut self, lobby: LobbyInfo) -> Result<(), NetworkError>;
    fn remove(&mut self, lobby_id: &str) -> Result<(), NetworkError>;
}

#[derive(Default)]
pub struct InMemoryLobbyBackend {
    lobbies: BTreeMap<String, LobbyInfo>,
}

impl LobbyBackend for InMemoryLobbyBackend {
    fn list(&mut self) -> Result<Vec<LobbyInfo>, NetworkError> {
        Ok(self.lobbies.values().cloned().collect())
    }

    fn create(&mut self, lobby: LobbyInfo) -> Result<(), NetworkError> {
        validate_lobby(&lobby)?;
        if self.lobbies.contains_key(&lobby.id) {
            return Err(NetworkError::InvalidPacket(
                "lobby ID already exists".into(),
            ));
        }
        self.lobbies.insert(lobby.id.clone(), lobby);
        Ok(())
    }

    fn join(&mut self, lobby_id: &str, peer: PeerId) -> Result<LobbyInfo, NetworkError> {
        let lobby = self
            .lobbies
            .get_mut(lobby_id)
            .ok_or_else(|| NetworkError::InvalidPacket("unknown lobby".into()))?;
        if !lobby.joinable && !lobby.members.contains(&peer) {
            return Err(NetworkError::InvalidPacket("lobby is not joinable".into()));
        }
        if lobby.members.len() >= usize::from(lobby.max_members) && !lobby.members.contains(&peer) {
            return Err(NetworkError::LimitExceeded("lobby members"));
        }
        lobby.members.insert(peer);
        Ok(lobby.clone())
    }

    fn leave(&mut self, lobby_id: &str, peer: PeerId) -> Result<LobbyInfo, NetworkError> {
        let lobby = self
            .lobbies
            .get_mut(lobby_id)
            .ok_or_else(|| NetworkError::InvalidPacket("unknown lobby".into()))?;
        lobby.members.remove(&peer);
        Ok(lobby.clone())
    }

    fn update(&mut self, lobby: LobbyInfo) -> Result<(), NetworkError> {
        validate_lobby(&lobby)?;
        if !self.lobbies.contains_key(&lobby.id) {
            return Err(NetworkError::InvalidPacket("unknown lobby".into()));
        }
        self.lobbies.insert(lobby.id.clone(), lobby);
        Ok(())
    }

    fn remove(&mut self, lobby_id: &str) -> Result<(), NetworkError> {
        self.lobbies.remove(lobby_id);
        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
pub struct FriendPresence {
    friends: BTreeMap<PeerId, FriendState>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FriendState {
    pub display_name: String,
    pub online: bool,
    pub lobby_id: Option<String>,
}

impl FriendPresence {
    pub fn update(&mut self, peer: PeerId, state: FriendState) {
        self.friends.insert(peer, state);
    }

    pub fn remove(&mut self, peer: PeerId) {
        self.friends.remove(&peer);
    }

    pub fn friends(&self) -> &BTreeMap<PeerId, FriendState> {
        &self.friends
    }
}

fn validate_lobby(lobby: &LobbyInfo) -> Result<(), NetworkError> {
    if lobby.id.is_empty()
        || lobby.id.len() > 128
        || lobby.name.is_empty()
        || lobby.name.len() > 128
    {
        return Err(NetworkError::LimitExceeded("lobby ID or name"));
    }
    if lobby.max_members == 0 || lobby.members.len() > usize::from(lobby.max_members) {
        return Err(NetworkError::LimitExceeded("lobby members"));
    }
    if lobby.metadata.len() > 64
        || lobby
            .metadata
            .iter()
            .any(|(key, value)| key.len() > 128 || value.len() > 1024)
    {
        return Err(NetworkError::LimitExceeded("lobby metadata"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lobby() -> LobbyInfo {
        LobbyInfo {
            id: "nexus-1".into(),
            owner: PeerId(1),
            name: "Nexus".into(),
            max_members: 2,
            members: BTreeSet::from([PeerId(1)]),
            joinable: true,
            metadata: BTreeMap::new(),
        }
    }

    #[test]
    fn join_and_leave_enforce_capacity() {
        let mut backend = InMemoryLobbyBackend::default();
        backend.create(lobby()).unwrap();
        assert_eq!(backend.join("nexus-1", PeerId(2)).unwrap().members.len(), 2);
        assert!(matches!(
            backend.join("nexus-1", PeerId(3)),
            Err(NetworkError::LimitExceeded("lobby members"))
        ));
        let lobby = backend.leave("nexus-1", PeerId(2)).unwrap();
        assert_eq!(lobby.members, BTreeSet::from([PeerId(1)]));
    }
}
