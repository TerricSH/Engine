use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const NETWORK_PROTOCOL_VERSION: u16 = 1;
pub const MAX_NETWORK_PACKET_BYTES: usize = 60 * 1024;
pub const MAX_RPC_PAYLOAD_BYTES: usize = 48 * 1024;
pub const MAX_REPLICATION_PAYLOAD_BYTES: usize = 48 * 1024;

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct PeerId(pub u64);

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct NetworkEntityId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NetworkRole {
    AuthoritativeServer,
    Client,
    ListenServer,
}

impl NetworkRole {
    pub fn is_authority(self) -> bool {
        matches!(self, Self::AuthoritativeServer | Self::ListenServer)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplicationUpdate {
    pub entity: NetworkEntityId,
    pub component: String,
    pub revision: u64,
    pub payload: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcEnvelope {
    pub rpc_id: u64,
    pub sender: PeerId,
    pub target: RpcTarget,
    pub method: String,
    pub reliable: bool,
    pub payload: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RpcTarget {
    Server,
    Peer(PeerId),
    All,
    Others,
    Owner(NetworkEntityId),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NetworkMessage {
    Hello {
        protocol: u16,
        display_name: String,
    },
    Welcome {
        peer: PeerId,
        session_id: u64,
    },
    Disconnect {
        reason: String,
    },
    Ownership {
        entity: NetworkEntityId,
        owner: Option<PeerId>,
        revision: u64,
    },
    Replication(Vec<ReplicationUpdate>),
    Rpc(RpcEnvelope),
    Ping {
        nonce: u64,
    },
    Pong {
        nonce: u64,
    },
    /// Transport-level acknowledgement. `acknowledged_sequence` in the
    /// containing packet identifies the reliable datagram being confirmed.
    Ack,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkPacket {
    pub protocol: u16,
    pub session_id: u64,
    pub sender: PeerId,
    pub sequence: u64,
    pub acknowledged_sequence: u64,
    pub message: NetworkMessage,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum NetworkError {
    #[error("network transport error: {0}")]
    Transport(String),
    #[error("network packet exceeds {MAX_NETWORK_PACKET_BYTES} bytes")]
    PacketTooLarge,
    #[error("network packet is invalid: {0}")]
    InvalidPacket(String),
    #[error("network protocol mismatch: expected {expected}, received {received}")]
    ProtocolMismatch { expected: u16, received: u16 },
    #[error("network operation requires server authority")]
    AuthorityRequired,
    #[error("unknown network peer {0:?}")]
    UnknownPeer(PeerId),
    #[error("network entity {0:?} is owned by another peer")]
    OwnershipDenied(NetworkEntityId),
    #[error("network resource limit exceeded: {0}")]
    LimitExceeded(&'static str),
    #[error("reliable network packet {sequence} was not acknowledged after bounded retries")]
    ReliableDeliveryFailed { sequence: u64 },
}
