use super::*;

impl GameLoop {
    /// Capture a complete live-world checkpoint.
    ///
    /// `custom_state` is the project-owned portion of the save (inventory,
    /// objectives, dialogue flags, and similar rules). The engine adds the
    /// live ECS scene, world origin, game state, and transient rigid-body
    /// state.
    pub fn capture_save_game(
        &self,
        custom_state: std::collections::BTreeMap<String, engine_serialize::Value>,
    ) -> Result<crate::SaveGameSnapshot, crate::SaveGameError> {
        let scene = crate::savegame::capture_live_scene(&self.runtime)?;
        let world_origin = self
            .runtime
            .with_world(|world| world.world_origin())
            .ok_or(crate::SaveGameError::NoWorld)?;

        #[cfg(feature = "subsystem-physics")]
        let physics_bodies = if let Some(physics) = &self.physics {
            let states = physics.runtime_body_states();
            self.runtime
                .with_world(|world| {
                    states
                        .into_iter()
                        .filter_map(|(entity, state)| {
                            Some(crate::SavedPhysicsBody {
                                entity_id: world.persistent_id(entity)?.to_string(),
                                position: state.position,
                                rotation: state.rotation,
                                linear_velocity: state.linear_velocity,
                                angular_velocity: state.angular_velocity,
                                sleeping: state.sleeping,
                            })
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        #[cfg(not(feature = "subsystem-physics"))]
        let physics_bodies = Vec::new();

        #[cfg(feature = "subsystem-gameplay")]
        let game_state = Some(self.state_manager.current().to_u32());
        #[cfg(not(feature = "subsystem-gameplay"))]
        let game_state = None;

        let mut snapshot = crate::SaveGameSnapshot {
            schema_version: crate::SAVE_GAME_SCHEMA_VERSION,
            scene,
            world_origin,
            world_origin_shift_count: self.world_origin_shift_count,
            game_state,
            physics_bodies,
            custom_state,
        };
        snapshot
            .physics_bodies
            .sort_by(|left, right| left.entity_id.cmp(&right.entity_id));
        snapshot.validate()?;
        Ok(snapshot)
    }

    /// Restore a previously decoded checkpoint.
    ///
    /// Scene installation is transactional: validation and ECS construction
    /// finish before the active world is replaced. Missing physics entities
    /// are reported as skips so saves remain forward-compatible with a scene
    /// that intentionally removed a prop.
    pub fn restore_save_game(
        &mut self,
        snapshot: crate::SaveGameSnapshot,
    ) -> Result<crate::SaveGameRestoreReport, crate::SaveGameError> {
        snapshot.validate()?;
        let crate::SaveGameSnapshot {
            scene,
            world_origin,
            world_origin_shift_count,
            game_state,
            physics_bodies,
            custom_state,
            ..
        } = snapshot;
        self.load_scene(scene).map_err(|diagnostics| {
            crate::SaveGameError::SceneRestore(
                diagnostics
                    .iter()
                    .map(|diagnostic| format!("{}: {}", diagnostic.code, diagnostic.message))
                    .collect::<Vec<_>>()
                    .join("; "),
            )
        })?;
        self.runtime
            .with_world_mut(|world| world.restore_world_origin(world_origin))
            .expect("load_scene installed a world")
            .map_err(|error| crate::SaveGameError::InvalidSnapshot(error.to_string()))?;
        self.world_origin_shift_count = world_origin_shift_count;
        self.last_world_origin_shift = None;

        #[cfg(feature = "subsystem-gameplay")]
        if let Some(state) = game_state.and_then(engine_gameplay::GameState::from_u32) {
            self.state_manager.force_transition(state);
        }
        #[cfg(not(feature = "subsystem-gameplay"))]
        let _ = game_state;

        #[cfg(feature = "subsystem-physics")]
        let (restored_physics_bodies, skipped_physics_bodies) = {
            let mut restored = 0;
            let mut skipped = Vec::new();
            let runtime = &self.runtime;
            let resolved = runtime
                .with_world(|world| {
                    physics_bodies
                        .iter()
                        .map(|body| (body, world.entity_by_persistent_id(&body.entity_id)))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if let Some(physics) = &mut self.physics {
                for (body, entity) in resolved {
                    let Some(entity) = entity else {
                        skipped.push(body.entity_id.clone());
                        continue;
                    };
                    let state = engine_physics::RigidBodyRuntimeState {
                        position: body.position,
                        rotation: body.rotation,
                        linear_velocity: body.linear_velocity,
                        angular_velocity: body.angular_velocity,
                        sleeping: body.sleeping,
                    };
                    if physics.restore_runtime_body_state(entity, &state) {
                        restored += 1;
                    } else {
                        skipped.push(body.entity_id.clone());
                    }
                }
                runtime.with_world_mut(|world| physics.sync_to_ecs(world));
            } else {
                skipped.extend(physics_bodies.iter().map(|body| body.entity_id.clone()));
            }
            (restored, skipped)
        };
        #[cfg(not(feature = "subsystem-physics"))]
        let (restored_physics_bodies, skipped_physics_bodies) = {
            let skipped = physics_bodies
                .into_iter()
                .map(|body| body.entity_id)
                .collect();
            (0, skipped)
        };

        Ok(crate::SaveGameRestoreReport {
            restored_physics_bodies,
            skipped_physics_bodies,
            custom_state,
        })
    }

    #[cfg(feature = "subsystem-scripting-csharp")]
    pub(super) fn process_script_save_requests(&mut self) {
        const SCRIPT_STATE_KEY: &str = "script_state_json";
        let requests = self.runtime.take_pending_save_requests();
        for (index, request) in requests.into_iter().enumerate() {
            let engine_script::OwnedGameplaySaveRequest {
                owner_entity_id,
                slot,
                operation,
            } = request;
            let outcome = if index > 0 {
                Err("only one save or load operation may execute per frame".to_string())
            } else if let Some(directory) = self.script_save_directory.clone() {
                let path = directory.join(format!("{slot}.save"));
                match operation {
                    engine_script::GameplaySaveOperation::Save { state_json } => self
                        .capture_save_game(std::collections::BTreeMap::from([(
                            SCRIPT_STATE_KEY.to_string(),
                            engine_serialize::Value::Str(state_json),
                        )]))
                        .and_then(|snapshot| crate::write_save_game(path, &snapshot))
                        .map(|_| (engine_script::GameplaySaveEventKind::Saved, None))
                        .map_err(|error| error.to_string()),
                    engine_script::GameplaySaveOperation::Load => crate::read_save_game(path)
                        .and_then(|snapshot| self.restore_save_game(snapshot))
                        .and_then(|report| {
                            let state_json = report
                                .custom_state
                                .get(SCRIPT_STATE_KEY)
                                .and_then(|value| match value {
                                    engine_serialize::Value::Str(value) => Some(value.clone()),
                                    _ => None,
                                })
                                .ok_or_else(|| {
                                    crate::SaveGameError::InvalidSnapshot(
                                        "checkpoint does not contain script state JSON".into(),
                                    )
                                })?;
                            Ok((
                                engine_script::GameplaySaveEventKind::Loaded,
                                Some(state_json),
                            ))
                        })
                        .map_err(|error| error.to_string()),
                }
            } else {
                Err("the runtime host did not configure a script save directory".to_string())
            };
            let event = match outcome {
                Ok((kind, state_json)) => engine_script::GameplaySaveEvent {
                    slot,
                    kind,
                    state_json,
                    error: None,
                },
                Err(error) => engine_script::GameplaySaveEvent {
                    slot,
                    kind: engine_script::GameplaySaveEventKind::Failed,
                    state_json: None,
                    error: Some(error),
                },
            };
            self.runtime.push_script_save_event(owner_entity_id, event);
        }
    }
}
