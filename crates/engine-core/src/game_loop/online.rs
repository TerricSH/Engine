use super::*;

#[cfg(feature = "subsystem-network")]
impl GameLoop {
    pub(super) fn refresh_script_network_context(
        &mut self,
        rpc_events: Vec<engine_network::RpcEnvelope>,
    ) {
        let (active, role, local_peer_id, session_id, peers) = self.network.session().map_or_else(
            || (false, None, None, None, Vec::new()),
            |session| {
                let role = Some(match session.role() {
                    engine_network::NetworkRole::AuthoritativeServer => {
                        engine_script::GameplayNetworkRole::AuthoritativeServer
                    }
                    engine_network::NetworkRole::Client => {
                        engine_script::GameplayNetworkRole::Client
                    }
                    engine_network::NetworkRole::ListenServer => {
                        engine_script::GameplayNetworkRole::ListenServer
                    }
                });
                let peers = session
                    .peers()
                    .iter()
                    .take(256)
                    .map(|(peer, state)| engine_script::GameplayNetworkPeer {
                        peer_id: peer.0,
                        display_name: state.display_name.clone(),
                        last_seen_seconds: state.last_seen_seconds,
                    })
                    .collect();
                (
                    true,
                    role,
                    Some(session.local_peer().0),
                    Some(session.session_id()),
                    peers,
                )
            },
        );
        let lobbies = match self.network.lobby.list() {
            Ok(lobbies) => lobbies.into_iter().take(256).map(gameplay_lobby).collect(),
            Err(error) => {
                tracing::warn!(%error, "script lobby snapshot failed");
                Vec::new()
            }
        };
        let snapshot = engine_script::GameplayNetworkSnapshot {
            active,
            role,
            local_peer_id,
            session_id,
            peers,
            ownership: self
                .network
                .replication
                .ownership_snapshot()
                .into_iter()
                .take(1024)
                .map(
                    |(entity, owner, revision)| engine_script::GameplayNetworkOwnership {
                        network_entity_id: entity.0,
                        owner_peer_id: owner.map(|peer| peer.0),
                        revision,
                    },
                )
                .collect(),
            replicated_states: self
                .network
                .replication
                .snapshot()
                .into_iter()
                .take(1024)
                .map(|state| engine_script::GameplayReplicatedState {
                    network_entity_id: state.entity.0,
                    component: state.component,
                    revision: state.revision,
                    payload: state.payload,
                })
                .collect(),
            rpc_events: rpc_events
                .into_iter()
                .map(|event| engine_script::GameplayRpcEvent {
                    rpc_id: event.rpc_id,
                    sender_peer_id: event.sender.0,
                    target: gameplay_rpc_target(event.target),
                    method: event.method,
                    reliable: event.reliable,
                    payload: event.payload,
                })
                .collect(),
            lobbies,
            friends: self
                .network
                .friends
                .friends()
                .iter()
                .take(1024)
                .map(|(peer, state)| engine_script::GameplayFriend {
                    peer_id: peer.0,
                    display_name: state.display_name.clone(),
                    online: state.online,
                    lobby_id: state.lobby_id.clone(),
                })
                .collect(),
            operation_results: Vec::new(),
        };
        self.runtime.set_script_network_snapshot(snapshot);
    }

    pub(super) fn process_script_network_commands(&mut self) {
        for request in self.runtime.take_pending_network_commands() {
            let operation = request.command.operation_name().to_string();
            let result = self.execute_script_network_command(request.command);
            let (success, value, error) = match result {
                Ok(value) => (true, value, None),
                Err(error) => (false, None, Some(error)),
            };
            self.runtime.push_script_network_result(
                request.owner_entity_id,
                engine_script::GameplayNetworkOperationResult {
                    request_id: request.request_id,
                    operation,
                    success,
                    value,
                    error,
                },
            );
        }
    }

    fn execute_script_network_command(
        &mut self,
        command: engine_script::GameplayNetworkCommand,
    ) -> Result<Option<String>, String> {
        use engine_script::GameplayNetworkCommand as Command;
        match command {
            Command::Host {
                bind_address,
                session_id,
                listen_server,
            } => {
                let bind = parse_socket_address(&bind_address, "bind address")?;
                self.network
                    .host(bind, session_id, listen_server)
                    .map(|address| Some(address.to_string()))
                    .map_err(|error| error.to_string())
            }
            Command::Connect {
                bind_address,
                server_address,
                display_name,
            } => {
                let bind = parse_socket_address(&bind_address, "bind address")?;
                let server = parse_socket_address(&server_address, "server address")?;
                self.network
                    .connect(bind, server, display_name)
                    .map(|address| Some(address.to_string()))
                    .map_err(|error| error.to_string())
            }
            Command::Disconnect => {
                self.network.disconnect();
                Ok(None)
            }
            Command::AssignOwner {
                network_entity_id,
                owner_peer_id,
            } => self
                .network
                .assign_owner(
                    engine_network::NetworkEntityId(network_entity_id),
                    owner_peer_id.map(engine_network::PeerId),
                )
                .map(|revision| Some(revision.to_string()))
                .map_err(|error| error.to_string()),
            Command::WriteComponent {
                network_entity_id,
                component,
                payload,
            } => self
                .network
                .replication
                .write_component(
                    engine_network::NetworkEntityId(network_entity_id),
                    component,
                    payload,
                )
                .map(|revision| Some(revision.to_string()))
                .map_err(|error| error.to_string()),
            Command::SendRpc {
                target,
                method,
                reliable,
                payload,
            } => self
                .network
                .send_rpc(network_rpc_target(target), method, reliable, payload)
                .map(|rpc_id| Some(rpc_id.to_string()))
                .map_err(|error| error.to_string()),
            Command::CreateLobby {
                lobby_id,
                name,
                max_members,
                joinable,
                metadata,
            } => {
                let local = self.local_network_peer()?;
                self.network
                    .lobby
                    .create(engine_network::LobbyInfo {
                        id: lobby_id.clone(),
                        owner: local,
                        name,
                        max_members,
                        members: std::collections::BTreeSet::from([local]),
                        joinable,
                        metadata,
                    })
                    .map(|()| Some(lobby_id))
                    .map_err(|error| error.to_string())
            }
            Command::JoinLobby { lobby_id } => {
                let local = self.local_network_peer()?;
                self.network
                    .lobby
                    .join(&lobby_id, local)
                    .map(|lobby| Some(lobby.id))
                    .map_err(|error| error.to_string())
            }
            Command::LeaveLobby { lobby_id } => {
                let local = self.local_network_peer()?;
                self.network
                    .lobby
                    .leave(&lobby_id, local)
                    .map(|lobby| Some(lobby.id))
                    .map_err(|error| error.to_string())
            }
            Command::RemoveLobby { lobby_id } => {
                let local = self.local_network_peer()?;
                let owned = self
                    .network
                    .lobby
                    .list()
                    .map_err(|error| error.to_string())?
                    .into_iter()
                    .any(|lobby| lobby.id == lobby_id && lobby.owner == local);
                if !owned {
                    return Err("only the local lobby owner may remove a lobby".into());
                }
                self.network
                    .lobby
                    .remove(&lobby_id)
                    .map(|()| Some(lobby_id))
                    .map_err(|error| error.to_string())
            }
            Command::UpdateFriend { friend } => {
                self.network.friends.update(
                    engine_network::PeerId(friend.peer_id),
                    engine_network::FriendState {
                        display_name: friend.display_name,
                        online: friend.online,
                        lobby_id: friend.lobby_id,
                    },
                );
                Ok(None)
            }
            Command::RemoveFriend { peer_id } => {
                self.network.friends.remove(engine_network::PeerId(peer_id));
                Ok(None)
            }
        }
    }

    fn local_network_peer(&self) -> Result<engine_network::PeerId, String> {
        self.network
            .session()
            .map(engine_network::NetworkSession::local_peer)
            .filter(|peer| peer.0 != 0)
            .ok_or_else(|| {
                "network session is not active or has not completed its handshake".into()
            })
    }
}

#[cfg(not(feature = "subsystem-network"))]
impl GameLoop {
    pub(super) fn refresh_script_network_context(&mut self) {
        self.runtime
            .set_script_network_snapshot(engine_script::GameplayNetworkSnapshot::default());
    }

    pub(super) fn process_script_network_commands(&mut self) {
        for request in self.runtime.take_pending_network_commands() {
            self.runtime.push_script_network_result(
                request.owner_entity_id,
                engine_script::GameplayNetworkOperationResult {
                    request_id: request.request_id,
                    operation: request.command.operation_name().to_string(),
                    success: false,
                    value: None,
                    error: Some("this engine build does not include subsystem-network".into()),
                },
            );
        }
    }
}

#[cfg(feature = "subsystem-xr")]
impl GameLoop {
    pub(super) fn refresh_script_xr_context(&mut self) {
        let frame = self
            .xr
            .latest_frame()
            .map(|frame| engine_script::GameplayXrFrame {
                predicted_display_time_nanoseconds: frame.predicted_display_time_nanoseconds,
                should_render: frame.should_render,
                views: frame.views.map(gameplay_xr_view),
                head: gameplay_xr_pose(frame.head),
                left_hand: gameplay_xr_pose(frame.left_hand),
                right_hand: gameplay_xr_pose(frame.right_hand),
            });
        let actions = self
            .xr
            .actions()
            .values
            .iter()
            .map(|(name, value)| {
                let value = match value {
                    engine_xr::XrActionValue::Boolean(value) => {
                        engine_script::GameplayXrActionValue::Boolean(*value)
                    }
                    engine_xr::XrActionValue::Float(value) => {
                        engine_script::GameplayXrActionValue::Float(*value)
                    }
                    engine_xr::XrActionValue::Vector2(value) => {
                        engine_script::GameplayXrActionValue::Vector2(*value)
                    }
                    engine_xr::XrActionValue::Pose(value) => {
                        engine_script::GameplayXrActionValue::Pose(gameplay_xr_pose(*value))
                    }
                };
                (name.clone(), value)
            })
            .collect();
        self.runtime
            .set_script_xr_snapshot(engine_script::GameplayXrSnapshot {
                active: self.xr.is_active(),
                frame,
                actions,
            });
    }
}

#[cfg(not(feature = "subsystem-xr"))]
impl GameLoop {
    pub(super) fn refresh_script_xr_context(&mut self) {
        self.runtime
            .set_script_xr_snapshot(engine_script::GameplayXrSnapshot::default());
    }
}

#[cfg(feature = "subsystem-network")]
fn gameplay_rpc_target(target: engine_network::RpcTarget) -> engine_script::GameplayRpcTarget {
    match target {
        engine_network::RpcTarget::Server => engine_script::GameplayRpcTarget::Server,
        engine_network::RpcTarget::Peer(peer) => engine_script::GameplayRpcTarget::Peer(peer.0),
        engine_network::RpcTarget::All => engine_script::GameplayRpcTarget::All,
        engine_network::RpcTarget::Others => engine_script::GameplayRpcTarget::Others,
        engine_network::RpcTarget::Owner(entity) => {
            engine_script::GameplayRpcTarget::Owner(entity.0)
        }
    }
}

#[cfg(feature = "subsystem-network")]
fn network_rpc_target(target: engine_script::GameplayRpcTarget) -> engine_network::RpcTarget {
    match target {
        engine_script::GameplayRpcTarget::Server => engine_network::RpcTarget::Server,
        engine_script::GameplayRpcTarget::Peer(peer) => {
            engine_network::RpcTarget::Peer(engine_network::PeerId(peer))
        }
        engine_script::GameplayRpcTarget::All => engine_network::RpcTarget::All,
        engine_script::GameplayRpcTarget::Others => engine_network::RpcTarget::Others,
        engine_script::GameplayRpcTarget::Owner(entity) => {
            engine_network::RpcTarget::Owner(engine_network::NetworkEntityId(entity))
        }
    }
}

#[cfg(feature = "subsystem-network")]
fn gameplay_lobby(lobby: engine_network::LobbyInfo) -> engine_script::GameplayLobby {
    engine_script::GameplayLobby {
        id: lobby.id,
        owner_peer_id: lobby.owner.0,
        name: lobby.name,
        max_members: lobby.max_members,
        members: lobby.members.into_iter().map(|peer| peer.0).collect(),
        joinable: lobby.joinable,
        metadata: lobby.metadata,
    }
}

#[cfg(feature = "subsystem-network")]
fn parse_socket_address(value: &str, label: &str) -> Result<std::net::SocketAddr, String> {
    value
        .parse()
        .map_err(|error| format!("invalid {label} '{value}': {error}"))
}

#[cfg(feature = "subsystem-xr")]
fn gameplay_xr_pose(pose: engine_xr::XrPose) -> engine_script::GameplayXrPose {
    engine_script::GameplayXrPose {
        orientation: pose.orientation,
        position: pose.position,
        orientation_valid: pose.orientation_valid,
        position_valid: pose.position_valid,
        tracked: pose.tracked,
    }
}

#[cfg(feature = "subsystem-xr")]
fn gameplay_xr_view(view: engine_xr::XrView) -> engine_script::GameplayXrView {
    engine_script::GameplayXrView {
        pose: gameplay_xr_pose(view.pose),
        fov: engine_script::GameplayXrFieldOfView {
            angle_left: view.fov.angle_left,
            angle_right: view.fov.angle_right,
            angle_up: view.fov.angle_up,
            angle_down: view.fov.angle_down,
        },
    }
}
