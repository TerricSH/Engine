# Networking runtime

`engine-network` is the transport-agnostic multiplayer foundation. It owns a
bounded, versioned wire protocol, authoritative sessions, stable peer and
entity identities, revisioned state replication, RPC dispatch, and lobby /
friend-presence contracts. `engine-core/subsystem-network` installs and ticks a
`NetworkRuntime`; game rules remain outside the transport crate.

`NetworkSession<T>` accepts any `NetworkTransport`. The built-in
`UdpTransport` is non-blocking and supports dedicated server, listen-server,
and client roles. A client begins with `Hello`; the authority assigns a
`PeerId` and session ID in `Welcome`. Packet sizes, protocol versions, peer
counts, display-name lengths, receive work and command queues are bounded.
Per-peer sequence windows reject replays while still accepting legitimate UDP
reordering. Control messages, ownership, replication and RPCs marked reliable
use bounded acknowledgements and retransmission; retry count and queued packet
memory are capped, and exhaustion is an explicit error. Idle peers can be
removed with an explicit timeout policy.

`ReplicationRegistry` is the authority boundary. It tracks entity ownership,
component payload revisions and dirty state. Only the authority may assign
owners; client-authored component changes are accepted only for entities owned
by that peer, then revision-checked and rebroadcast. New peers receive an
ownership and component snapshot, one bounded component per packet. Send
failure restores unsent dirty state instead of losing it. Clients accept newer
revisions and ignore stale ones.

`RpcRouter` registers named handlers and queues validated RPC envelopes with
explicit target and transport-derived sender identities. Client calls always
pass through the authority, which routes `Server`, `Peer`, `All`, `Others` and
entity `Owner` targets rather than trusting a forged sender field. `LobbyBackend`
defines create/join/leave/list operations plus friend presence;
`InMemoryLobbyBackend` is the deterministic local/test implementation, while
online services implement the same interface without changing sessions or
gameplay code.

The protocol intentionally transports opaque component/RPC bytes. Schema,
prediction, reconciliation, interest management and persistence policies are
registered by the game or a higher-level engine subsystem rather than being
hard-coded into UDP transport.
