//! Transport-agnostic multiplayer foundations: authoritative sessions,
//! entity ownership, revisioned state replication, RPC and lobby contracts.

#![forbid(unsafe_code)]

mod lobby;
mod protocol;
mod replication;
mod rpc;
mod runtime;
mod session;
mod transport;

pub use lobby::*;
pub use protocol::*;
pub use replication::*;
pub use rpc::*;
pub use runtime::*;
pub use session::*;
pub use transport::*;
