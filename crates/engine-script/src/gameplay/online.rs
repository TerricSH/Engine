use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::validation::validate_entity_id;

pub const MAX_PENDING_NETWORK_COMMANDS: usize = 256;
pub const MAX_SCRIPT_NETWORK_PAYLOAD_BYTES: usize = 48 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GameplayNetworkRole {
    AuthoritativeServer,
    Client,
    ListenServer,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GameplayNetworkSnapshot {
    pub active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<GameplayNetworkRole>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_peer_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<u64>,
    #[serde(default)]
    pub peers: Vec<GameplayNetworkPeer>,
    #[serde(default)]
    pub ownership: Vec<GameplayNetworkOwnership>,
    #[serde(default)]
    pub replicated_states: Vec<GameplayReplicatedState>,
    #[serde(default)]
    pub rpc_events: Vec<GameplayRpcEvent>,
    #[serde(default)]
    pub lobbies: Vec<GameplayLobby>,
    #[serde(default)]
    pub friends: Vec<GameplayFriend>,
    #[serde(default)]
    pub operation_results: Vec<GameplayNetworkOperationResult>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GameplayNetworkPeer {
    pub peer_id: u64,
    pub display_name: String,
    pub last_seen_seconds: f64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameplayNetworkOwnership {
    pub network_entity_id: u64,
    pub owner_peer_id: Option<u64>,
    pub revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameplayReplicatedState {
    pub network_entity_id: u64,
    pub component: String,
    pub revision: u64,
    pub payload: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum GameplayRpcTarget {
    Server,
    Peer(u64),
    All,
    Others,
    Owner(u64),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameplayRpcEvent {
    pub rpc_id: u64,
    pub sender_peer_id: u64,
    pub target: GameplayRpcTarget,
    pub method: String,
    pub reliable: bool,
    pub payload: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameplayLobby {
    pub id: String,
    pub owner_peer_id: u64,
    pub name: String,
    pub max_members: u16,
    pub members: Vec<u64>,
    pub joinable: bool,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameplayFriend {
    pub peer_id: u64,
    pub display_name: String,
    pub online: bool,
    pub lobby_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameplayNetworkOperationResult {
    pub request_id: u32,
    pub operation: String,
    pub success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GameplayNetworkCommand {
    Host {
        bind_address: String,
        session_id: u64,
        #[serde(default)]
        listen_server: bool,
    },
    Connect {
        bind_address: String,
        server_address: String,
        display_name: String,
    },
    Disconnect,
    AssignOwner {
        network_entity_id: u64,
        owner_peer_id: Option<u64>,
    },
    WriteComponent {
        network_entity_id: u64,
        component: String,
        payload: Vec<u8>,
    },
    SendRpc {
        target: GameplayRpcTarget,
        method: String,
        #[serde(default)]
        reliable: bool,
        payload: Vec<u8>,
    },
    CreateLobby {
        lobby_id: String,
        name: String,
        max_members: u16,
        #[serde(default = "default_true")]
        joinable: bool,
        #[serde(default)]
        metadata: BTreeMap<String, String>,
    },
    JoinLobby {
        lobby_id: String,
    },
    LeaveLobby {
        lobby_id: String,
    },
    RemoveLobby {
        lobby_id: String,
    },
    UpdateFriend {
        friend: GameplayFriend,
    },
    RemoveFriend {
        peer_id: u64,
    },
}

impl GameplayNetworkCommand {
    pub fn operation_name(&self) -> &'static str {
        match self {
            Self::Host { .. } => "host",
            Self::Connect { .. } => "connect",
            Self::Disconnect => "disconnect",
            Self::AssignOwner { .. } => "assign_owner",
            Self::WriteComponent { .. } => "write_component",
            Self::SendRpc { .. } => "send_rpc",
            Self::CreateLobby { .. } => "create_lobby",
            Self::JoinLobby { .. } => "join_lobby",
            Self::LeaveLobby { .. } => "leave_lobby",
            Self::RemoveLobby { .. } => "remove_lobby",
            Self::UpdateFriend { .. } => "update_friend",
            Self::RemoveFriend { .. } => "remove_friend",
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Host { bind_address, .. } => validate_text(bind_address, "bind address", 256),
            Self::Connect {
                bind_address,
                server_address,
                display_name,
            } => {
                validate_text(bind_address, "bind address", 256)?;
                validate_text(server_address, "server address", 256)?;
                validate_text(display_name, "display name", 128)
            }
            Self::Disconnect | Self::AssignOwner { .. } | Self::RemoveFriend { .. } => Ok(()),
            Self::WriteComponent {
                component, payload, ..
            } => {
                validate_entity_id(component)?;
                validate_payload(payload)
            }
            Self::SendRpc {
                method, payload, ..
            } => {
                validate_entity_id(method)?;
                validate_payload(payload)
            }
            Self::CreateLobby {
                lobby_id,
                name,
                max_members,
                metadata,
                ..
            } => {
                validate_text(lobby_id, "lobby ID", 128)?;
                validate_text(name, "lobby name", 128)?;
                if *max_members == 0 {
                    return Err("lobby capacity must be greater than zero".into());
                }
                validate_metadata(metadata)
            }
            Self::JoinLobby { lobby_id }
            | Self::LeaveLobby { lobby_id }
            | Self::RemoveLobby { lobby_id } => validate_text(lobby_id, "lobby ID", 128),
            Self::UpdateFriend { friend } => {
                validate_text(&friend.display_name, "friend display name", 128)?;
                if let Some(lobby_id) = &friend.lobby_id {
                    validate_text(lobby_id, "friend lobby ID", 128)?;
                }
                Ok(())
            }
        }
    }
}

fn validate_payload(payload: &[u8]) -> Result<(), String> {
    if payload.len() <= MAX_SCRIPT_NETWORK_PAYLOAD_BYTES {
        Ok(())
    } else {
        Err(format!(
            "network payload exceeds {MAX_SCRIPT_NETWORK_PAYLOAD_BYTES} bytes"
        ))
    }
}

fn validate_metadata(metadata: &BTreeMap<String, String>) -> Result<(), String> {
    if metadata.len() > 64 {
        return Err("lobby metadata has more than 64 entries".into());
    }
    for (key, value) in metadata {
        validate_text(key, "lobby metadata key", 128)?;
        if value.len() > 1024 || value.chars().any(char::is_control) {
            return Err(
                "lobby metadata values must contain at most 1024 bytes and no controls".into(),
            );
        }
    }
    Ok(())
}

fn default_true() -> bool {
    true
}

fn validate_text(value: &str, label: &str, max_bytes: usize) -> Result<(), String> {
    if !value.is_empty() && value.len() <= max_bytes && !value.chars().any(char::is_control) {
        Ok(())
    } else {
        Err(format!(
            "{label} must contain 1 to {max_bytes} bytes and no control characters"
        ))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnedGameplayNetworkCommand {
    pub owner_entity_id: String,
    pub request_id: u32,
    pub command: GameplayNetworkCommand,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_command_wire_contract_is_bounded_and_stable() {
        let command = GameplayNetworkCommand::SendRpc {
            target: GameplayRpcTarget::Owner(42),
            method: "world.edit".into(),
            reliable: true,
            payload: vec![1, 2, 3],
        };
        assert!(command.validate().is_ok());
        assert_eq!(
            serde_json::to_string(&command).unwrap(),
            r#"{"kind":"send_rpc","target":{"kind":"owner","id":42},"method":"world.edit","reliable":true,"payload":[1,2,3]}"#
        );
        assert!(GameplayNetworkCommand::WriteComponent {
            network_entity_id: 1,
            component: "state".into(),
            payload: vec![0; MAX_SCRIPT_NETWORK_PAYLOAD_BYTES + 1],
        }
        .validate()
        .is_err());
    }
}
