use super::*;

impl GameLoop {
    /// Initialise the physics world using gravity from the scene settings
    /// (or a default of (0, -9.81, 0)) and sync any RigidBody/Collider
    /// components already in the ECS world.
    ///
    /// No-op when the `subsystem-physics` feature is not enabled.
    pub fn init_physics(&mut self) {
        #[cfg(feature = "subsystem-physics")]
        {
            let gravity = self
                .runtime
                .with_world(|world| world.scene_settings().gravity)
                .flatten()
                .map(|g| glam::Vec3::new(g[0], g[1], g[2]))
                .unwrap_or(glam::Vec3::new(0.0, -9.81, 0.0));
            let mut pw = PhysicsWorld::new(gravity);
            self.runtime.with_world(|world| pw.sync_from_ecs(world));
            self.physics = Some(pw);
            self.physics_events.clear();
        }
    }

    /// Events produced by the most recent physics update.
    ///
    /// This snapshot is replaced on the next call to [`Self::update`]. Use
    /// [`Self::take_physics_events`] when the caller wants to take ownership.
    #[cfg(feature = "subsystem-physics")]
    pub fn physics_events(&self) -> &PhysicsEvents {
        &self.physics_events
    }

    /// Re-synchronise the physics world after direct ECS world mutations
    /// that bypass [`load_scene`](Self::load_scene) — world-partition cell
    /// streaming merges/unloads commit at the frame boundary and call this.
    ///
    /// With the `subsystem-physics` feature this runs the incremental
    /// `PhysicsWorld::sync_from_ecs`: bodies and colliders are created for
    /// newly merged entities and removed for unloaded ones, while every
    /// untouched entity keeps its exact simulation state. Scene-level
    /// physics settings (gravity) cannot change through cell merges because
    /// merges preserve world scene metadata, so no full rebuild is needed.
    /// Without the `subsystem-physics` feature this is a no-op.
    pub fn resync_physics_from_world(&mut self) {
        #[cfg(feature = "subsystem-physics")]
        if let Some(ref mut physics) = self.physics {
            self.runtime
                .with_world(|world| physics.sync_from_ecs(world));
        }
    }

    // ── World origin shifting (ENG-01 Phase 2) ──────────────────────────

    /// Take the most recent physics event snapshot, leaving it empty.
    #[cfg(feature = "subsystem-physics")]
    pub fn take_physics_events(&mut self) -> PhysicsEvents {
        std::mem::take(&mut self.physics_events)
    }

    /// Switch a scene-authored ragdoll between animation and physics
    /// ownership. Activation impulse is distributed across generated bodies;
    /// deactivation blends back over `recovery_duration` seconds.
    #[cfg(all(feature = "subsystem-animation", feature = "subsystem-physics"))]
    pub fn set_ragdoll_active(
        &mut self,
        entity_id: &str,
        active: bool,
        recovery_duration: f32,
        impulse: Vec3,
    ) -> Result<Vec<String>, String> {
        let previous = self
            .runtime
            .with_world(|world| {
                world
                    .entity_by_persistent_id(entity_id)
                    .and_then(|entity| world.get::<engine_animation::RagdollComponent>(entity))
                    .cloned()
            })
            .flatten()
            .ok_or_else(|| format!("entity '{entity_id}' has no Ragdoll component"))?;
        crate::ragdoll_runtime::set_active(self, entity_id, active, recovery_duration, impulse)?;
        crate::ragdoll_runtime::reconcile_before_physics(self);
        let generated = self
            .runtime
            .with_world(|world| {
                let entity = world
                    .entity_by_persistent_id(entity_id)
                    .ok_or_else(|| format!("ragdoll target '{entity_id}' disappeared"))?;
                let ragdoll = world
                    .get::<engine_animation::RagdollComponent>(entity)
                    .ok_or_else(|| format!("entity '{entity_id}' lost its Ragdoll component"))?;
                if ragdoll.generated_body_ids.len() != ragdoll.bodies.len()
                    || ragdoll.generated_joint_ids.len() != ragdoll.constraints.len()
                {
                    return Err(format!(
                        "ragdoll graph for '{entity_id}' could not be generated"
                    ));
                }
                let mut body_ids = Vec::with_capacity(ragdoll.generated_body_ids.len());
                for body_id in ragdoll.generated_body_ids.values() {
                    let body = world.entity_by_persistent_id(body_id).ok_or_else(|| {
                        format!("ragdoll body '{body_id}' for '{entity_id}' is missing")
                    })?;
                    if world.get::<engine_physics::RigidBody>(body).is_none()
                        || world.get::<engine_physics::Collider>(body).is_none()
                    {
                        return Err(format!(
                            "ragdoll body '{body_id}' for '{entity_id}' is incomplete"
                        ));
                    }
                    body_ids.push(body_id.clone());
                }
                for joint_id in &ragdoll.generated_joint_ids {
                    let joint = world.entity_by_persistent_id(joint_id).ok_or_else(|| {
                        format!("ragdoll joint '{joint_id}' for '{entity_id}' is missing")
                    })?;
                    if world.get::<engine_physics::PhysicsJoint>(joint).is_none() {
                        return Err(format!(
                            "ragdoll joint '{joint_id}' for '{entity_id}' is incomplete"
                        ));
                    }
                }
                Ok(body_ids)
            })
            .ok_or_else(|| "no active world".to_string())?;
        match generated {
            Ok(body_ids) => Ok(body_ids),
            Err(error) => {
                self.runtime.with_world_mut(|world| {
                    if let Some(entity) = world.entity_by_persistent_id(entity_id) {
                        world.add_component(entity, previous);
                    }
                });
                crate::ragdoll_runtime::reconcile_before_physics(self);
                Err(error)
            }
        }
    }

    #[cfg(all(feature = "subsystem-scripting-csharp", feature = "subsystem-physics"))]
    pub(super) fn resolved_script_physics_events(
        &self,
    ) -> std::collections::BTreeMap<String, Vec<engine_script::GameplayPhysicsEvent>> {
        use engine_physics::{CollisionEventKind, TriggerEventKind};
        use engine_script::{GameplayPhysicsEvent, GameplayPhysicsEventKind};

        self.runtime
            .with_world(|world| {
                let mut by_entity =
                    std::collections::BTreeMap::<String, Vec<GameplayPhysicsEvent>>::new();
                let mut record_pair =
                    |entity_a,
                     entity_b,
                     kind: GameplayPhysicsEventKind,
                     joint_id: Option<String>,
                     force: Option<f32>,
                     torque: Option<f32>| {
                        let Some(entity_a) = world.persistent_id(entity_a) else {
                            return;
                        };
                        let Some(entity_b) = world.persistent_id(entity_b) else {
                            return;
                        };
                        by_entity.entry(entity_a.to_owned()).or_default().push(
                            GameplayPhysicsEvent {
                                kind,
                                other_entity_id: entity_b.to_owned(),
                                joint_id: joint_id.clone(),
                                force,
                                torque,
                            },
                        );
                        by_entity.entry(entity_b.to_owned()).or_default().push(
                            GameplayPhysicsEvent {
                                kind,
                                other_entity_id: entity_a.to_owned(),
                                joint_id,
                                force,
                                torque,
                            },
                        );
                    };

                for event in &self.physics_events.collisions {
                    let kind = match event.kind {
                        CollisionEventKind::ContactStarted => {
                            GameplayPhysicsEventKind::CollisionEntered
                        }
                        CollisionEventKind::ContactStaying => {
                            GameplayPhysicsEventKind::CollisionStayed
                        }
                        CollisionEventKind::ContactStopped => {
                            GameplayPhysicsEventKind::CollisionExited
                        }
                    };
                    record_pair(event.entity_a, event.entity_b, kind, None, None, None);
                }
                for event in &self.physics_events.triggers {
                    let kind = match event.kind {
                        TriggerEventKind::Entered => GameplayPhysicsEventKind::TriggerEntered,
                        TriggerEventKind::Stay => GameplayPhysicsEventKind::TriggerStayed,
                        TriggerEventKind::Exited => GameplayPhysicsEventKind::TriggerExited,
                    };
                    record_pair(event.entity_a, event.entity_b, kind, None, None, None);
                }
                for event in &self.physics_events.joint_breaks {
                    let joint_id = event
                        .joint_entity
                        .and_then(|entity| world.persistent_id(entity))
                        .map(str::to_owned);
                    record_pair(
                        event.entity_a,
                        event.entity_b,
                        GameplayPhysicsEventKind::JointBroken,
                        joint_id,
                        Some(event.force),
                        Some(event.torque),
                    );
                }
                by_entity
            })
            .unwrap_or_default()
    }

    /// Execute the physics queries drained from the latest script update and
    /// stage the results for the next frame snapshot.
    ///
    /// Queries run against the physics world after this frame's step and ECS
    /// sync, so answers are consistent with the physics events delivered in
    /// the same update. Results are frame-local: they replace the previous
    /// staging map and are consumed by the next script tick.
    #[cfg(all(feature = "subsystem-scripting-csharp", feature = "subsystem-physics"))]
    pub(super) fn execute_script_physics_queries(&mut self) {
        let pending = self.runtime.take_pending_physics_queries();
        if pending.is_empty() {
            return;
        }
        let mut results = std::collections::BTreeMap::<
            String,
            Vec<engine_script::GameplayPhysicsQueryResult>,
        >::new();
        for engine_script::OwnedGameplayPhysicsQuery { entity_id, query } in pending {
            let result = self.execute_script_physics_query(&query);
            results.entry(entity_id).or_default().push(result);
        }
        self.script_physics_query_results = results;
    }

    /// Resolve validated script forces/impulses by persistent id and queue
    /// them for the next safe physics step.
    #[cfg(all(feature = "subsystem-scripting-csharp", feature = "subsystem-physics"))]
    pub(super) fn queue_script_physics_mutations(&mut self) {
        let pending = self.runtime.take_pending_physics_mutations();
        if self.physics.is_none() {
            return;
        }
        for engine_script::OwnedGameplayPhysicsMutation {
            owner_entity_id: _,
            mutation,
        } in pending
        {
            use engine_script::{GameplayJointType, GameplayPhysicsMutation};
            match mutation {
                GameplayPhysicsMutation::ApplyForce { entity_id, force } => {
                    let entity = self
                        .runtime
                        .with_world(|world| world.entity_by_persistent_id(&entity_id))
                        .flatten();
                    if let (Some(entity), Some(physics)) = (entity, self.physics.as_mut()) {
                        physics.queue_command(engine_physics::PhysicsCommand::ApplyForce {
                            entity,
                            force: Vec3::from(force),
                        });
                    }
                }
                GameplayPhysicsMutation::ApplyImpulse { entity_id, impulse } => {
                    let entity = self
                        .runtime
                        .with_world(|world| world.entity_by_persistent_id(&entity_id))
                        .flatten();
                    if let (Some(entity), Some(physics)) = (entity, self.physics.as_mut()) {
                        physics.queue_command(engine_physics::PhysicsCommand::ApplyImpulse {
                            entity,
                            impulse: Vec3::from(impulse),
                        });
                    }
                }
                GameplayPhysicsMutation::ApplyTorque { entity_id, torque } => {
                    let entity = self
                        .runtime
                        .with_world(|world| world.entity_by_persistent_id(&entity_id))
                        .flatten();
                    if let (Some(entity), Some(physics)) = (entity, self.physics.as_mut()) {
                        physics.queue_command(engine_physics::PhysicsCommand::ApplyTorque {
                            entity,
                            torque: Vec3::from(torque),
                        });
                    }
                }
                GameplayPhysicsMutation::ApplyTorqueImpulse {
                    entity_id,
                    torque_impulse,
                } => {
                    let entity = self
                        .runtime
                        .with_world(|world| world.entity_by_persistent_id(&entity_id))
                        .flatten();
                    if let (Some(entity), Some(physics)) = (entity, self.physics.as_mut()) {
                        physics.queue_command(engine_physics::PhysicsCommand::ApplyTorqueImpulse {
                            entity,
                            torque_impulse: Vec3::from(torque_impulse),
                        });
                    }
                }
                GameplayPhysicsMutation::CreateJoint {
                    joint_id,
                    body_a,
                    body_b,
                    joint_type,
                    anchor_a,
                    anchor_b,
                    axis,
                    limits,
                    motor,
                    break_force,
                    break_torque,
                } => {
                    self.runtime.with_world_mut(|world| {
                        if world.entity_by_persistent_id(&body_a).is_none()
                            || world.entity_by_persistent_id(&body_b).is_none()
                        {
                            return;
                        }
                        let constraint = match world.entity_by_persistent_id(&joint_id) {
                            Some(entity) => entity,
                            None => {
                                let Ok(entity) = world.create_persistent_entity(joint_id.clone())
                                else {
                                    return;
                                };
                                entity
                            }
                        };
                        world.add_component(
                            constraint,
                            engine_physics::PhysicsJoint {
                                enabled: true,
                                body_a,
                                body_b,
                                joint_type: match joint_type {
                                    GameplayJointType::Fixed => engine_physics::JointType::Fixed,
                                    GameplayJointType::Revolute => {
                                        engine_physics::JointType::Revolute
                                    }
                                    GameplayJointType::Prismatic => {
                                        engine_physics::JointType::Prismatic
                                    }
                                    GameplayJointType::Spherical => {
                                        engine_physics::JointType::Spherical
                                    }
                                },
                                anchor_a,
                                anchor_b,
                                axis,
                                limits: limits.map(|limits| engine_physics::JointLimits {
                                    min: limits.min,
                                    max: limits.max,
                                    stiffness: limits.stiffness,
                                    damping: limits.damping,
                                }),
                                motor: motor.map(|motor| engine_physics::JointMotor {
                                    target_vel: motor.target_vel,
                                    target_pos: motor.target_pos,
                                    stiffness: motor.stiffness,
                                    damping: motor.damping,
                                }),
                                break_force,
                                break_torque,
                            },
                        );
                    });
                }
                GameplayPhysicsMutation::RemoveJoint { joint_id } => {
                    self.runtime.with_world_mut(|world| {
                        if let Some(entity) = world.entity_by_persistent_id(&joint_id) {
                            world.remove_component::<engine_physics::PhysicsJoint>(entity);
                        }
                    });
                }
            }
        }
    }

    #[cfg(all(feature = "subsystem-scripting-csharp", feature = "subsystem-physics"))]
    pub(super) fn process_script_damage_requests(&mut self) {
        let pending = self.runtime.take_pending_damage_requests();
        for request in pending {
            let target = self
                .runtime
                .with_world(|world| world.entity_by_persistent_id(&request.target_entity_id))
                .flatten();
            let source = self
                .runtime
                .with_world(|world| world.entity_by_persistent_id(&request.owner_entity_id))
                .flatten();
            let Some(target) = target else {
                continue;
            };
            let damage_request = engine_physics::DamageRequest {
                source,
                amount: request.amount,
                kind: match request.damage_kind {
                    engine_script::GameplayDamageKind::Generic => {
                        engine_physics::DamageKind::Generic
                    }
                    engine_script::GameplayDamageKind::Impact => engine_physics::DamageKind::Impact,
                    engine_script::GameplayDamageKind::Bullet => engine_physics::DamageKind::Bullet,
                    engine_script::GameplayDamageKind::Blast => engine_physics::DamageKind::Blast,
                    engine_script::GameplayDamageKind::Fire => engine_physics::DamageKind::Fire,
                },
                hit_position: request.hit_position,
                impulse: request.impulse,
            };
            let result = self.runtime.with_world_mut(|world| {
                engine_physics::apply_damage(world, target, &damage_request)
            });
            let event = match result {
                Some(Ok(Some(event))) => event,
                Some(Ok(None)) => continue,
                Some(Err(error)) => {
                    let mut diagnostic = engine_serialize::Diagnostic::new(
                        "SCRIPT_DAMAGE_REJECTED",
                        engine_serialize::DiagnosticSeverity::Error,
                        "script",
                        format!(
                            "script entity '{}' could not damage '{}': {error}",
                            request.owner_entity_id, request.target_entity_id
                        ),
                    );
                    diagnostic.entity = Some(request.owner_entity_id);
                    self.runtime
                        .diagnostics_collector_mut()
                        .push_script_diags(vec![diagnostic]);
                    continue;
                }
                None => continue,
            };

            let mut spawned_entity_ids = Vec::new();
            if event.broke {
                let source_state = self.physics.as_ref().and_then(|physics| {
                    physics
                        .runtime_body_states()
                        .into_iter()
                        .find_map(|(entity, state)| (entity == target).then_some(state))
                });
                let target_translation = self
                    .runtime
                    .with_world(|world| {
                        world
                            .get::<engine_scene::components::Transform>(target)
                            .map(|transform| transform.translation.to_array())
                    })
                    .flatten()
                    .or(event.hit_position);
                let before_ids = self
                    .runtime
                    .with_world(|world| {
                        world
                            .persistent_entities()
                            .map(|(id, _)| id.to_owned())
                            .collect::<std::collections::BTreeSet<_>>()
                    })
                    .unwrap_or_default();

                let mut fracture_diagnostics = Vec::new();
                let replacement_succeeded = if let Some(prefab) = event.replacement_prefab.as_ref()
                {
                    self.runtime.spawn_script_prefab(
                        &request.owner_entity_id,
                        &prefab.id,
                        target_translation,
                        &mut fracture_diagnostics,
                        0,
                    );
                    let after_ids = self
                        .runtime
                        .with_world(|world| {
                            world
                                .persistent_entities()
                                .map(|(id, _)| id.to_owned())
                                .collect::<std::collections::BTreeSet<_>>()
                        })
                        .unwrap_or_default();
                    spawned_entity_ids = after_ids
                        .difference(&before_ids)
                        .cloned()
                        .collect::<Vec<_>>();
                    !spawned_entity_ids.is_empty()
                } else {
                    true
                };

                if replacement_succeeded {
                    if let Some(physics) = self.physics.as_mut() {
                        let rigid_pieces = self
                            .runtime
                            .with_world(|world| {
                                spawned_entity_ids
                                    .iter()
                                    .filter_map(|id| {
                                        let entity = world.entity_by_persistent_id(id)?;
                                        world
                                            .get::<engine_physics::RigidBody>(entity)
                                            .map(|_| entity)
                                    })
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_default();
                        let piece_count = rigid_pieces.len().max(1) as f32;
                        for piece in rigid_pieces {
                            if event.inherit_velocity {
                                if let Some(state) = source_state.as_ref() {
                                    physics.queue_command(
                                        engine_physics::PhysicsCommand::SetLinearVelocity {
                                            entity: piece,
                                            velocity: Vec3::from(state.linear_velocity),
                                        },
                                    );
                                    physics.queue_command(
                                        engine_physics::PhysicsCommand::SetAngularVelocity {
                                            entity: piece,
                                            velocity: Vec3::from(state.angular_velocity),
                                        },
                                    );
                                }
                            }
                            let impulse = Vec3::from(event.impulse)
                                * (event.fracture_impulse_scale / piece_count);
                            if impulse != Vec3::ZERO {
                                physics.queue_command(
                                    engine_physics::PhysicsCommand::ApplyImpulse {
                                        entity: piece,
                                        impulse,
                                    },
                                );
                            }
                        }
                    }

                    if event.destroy_on_break {
                        crate::runtime::destroy_script_entity(
                            &self.runtime.world_slot,
                            &mut self.runtime.scripting.engine,
                            &request.owner_entity_id,
                            &request.target_entity_id,
                            &mut fracture_diagnostics,
                        );
                    }
                }
                if !fracture_diagnostics.is_empty() {
                    self.runtime
                        .diagnostics_collector_mut()
                        .push_script_diags(fracture_diagnostics);
                }
            }

            let gameplay_event = engine_script::GameplayDamageEvent {
                target_entity_id: request.target_entity_id.clone(),
                source_entity_id: Some(request.owner_entity_id.clone()),
                damage_kind: request.damage_kind,
                raw_damage: event.raw_damage,
                applied_damage: event.applied_damage,
                remaining_health: event.remaining_health,
                hit_position: event.hit_position,
                impulse: event.impulse,
                broke: event.broke,
                spawned_entity_ids,
            };
            self.runtime
                .push_script_damage_event(request.target_entity_id.clone(), gameplay_event.clone());
            if request.owner_entity_id != request.target_entity_id {
                self.runtime
                    .push_script_damage_event(request.owner_entity_id, gameplay_event);
            }
        }
    }

    #[cfg(all(
        feature = "subsystem-scripting-csharp",
        feature = "subsystem-physics",
        feature = "subsystem-animation"
    ))]
    pub(super) fn process_script_ragdoll_requests(&mut self) {
        let pending = self.runtime.take_pending_ragdoll_requests();
        for request in pending {
            match self.set_ragdoll_active(
                &request.target_entity_id,
                request.active,
                request.recovery_duration,
                Vec3::from(request.impulse),
            ) {
                Ok(body_entity_ids) => {
                    let event = engine_script::GameplayRagdollEvent {
                        entity_id: request.target_entity_id.clone(),
                        active: request.active,
                        recovering: !request.active && request.recovery_duration > 0.0,
                        body_entity_ids,
                    };
                    self.runtime
                        .push_script_ragdoll_event(request.target_entity_id.clone(), event.clone());
                    if request.owner_entity_id != request.target_entity_id {
                        self.runtime
                            .push_script_ragdoll_event(request.owner_entity_id, event);
                    }
                }
                Err(error) => {
                    let mut diagnostic = engine_serialize::Diagnostic::new(
                        "SCRIPT_RAGDOLL_REJECTED",
                        engine_serialize::DiagnosticSeverity::Error,
                        "script",
                        format!(
                            "script entity '{}' could not change ragdoll ownership for '{}': {error}",
                            request.owner_entity_id, request.target_entity_id
                        ),
                    );
                    diagnostic.entity = Some(request.owner_entity_id);
                    self.runtime
                        .diagnostics_collector_mut()
                        .push_script_diags(vec![diagnostic]);
                }
            }
        }
    }

    /// Run one validated script physics query against the physics world,
    /// translating backend hits into persistent entity ids so scripts never
    /// observe raw ECS handles.
    #[cfg(all(feature = "subsystem-scripting-csharp", feature = "subsystem-physics"))]
    pub(super) fn execute_script_physics_query(
        &self,
        query: &engine_script::GameplayPhysicsQuery,
    ) -> engine_script::GameplayPhysicsQueryResult {
        use engine_script::{GameplayPhysicsQuery, GameplayPhysicsQueryResult};

        // Translate the script-side filter into backend terms: the
        // persistent exclude id becomes the ECS entity it names (already
        // validated to exist when the command was applied).
        let query_filter = query
            .filter()
            .map(|filter| engine_physics::PhysicsQueryFilter {
                layer_mask: filter.layer_mask,
                include_sensors: filter.include_sensors,
                exclude_entity: filter.exclude_entity.as_deref().and_then(|persistent_id| {
                    self.runtime
                        .with_world(|world| world.entity_by_persistent_id(persistent_id))
                        .flatten()
                }),
            });
        let query_filter = query_filter.unwrap_or_default();

        /// Translate a backend hit into a script result, reporting a miss
        /// when the hit collider has no persistent id to name.
        fn hit_result(
            hit: Option<engine_physics::RaycastHit>,
            metadata: impl Fn(
                engine_physics::Entity,
                f32,
            )
                -> Option<(String, Option<engine_script::GameplayInteractionSnapshot>)>,
            hit_kind: impl Fn(
                String,
                [f32; 3],
                [f32; 3],
                f32,
                Option<engine_script::GameplayInteractionSnapshot>,
            ) -> GameplayPhysicsQueryResult,
            miss: impl Fn() -> GameplayPhysicsQueryResult,
        ) -> GameplayPhysicsQueryResult {
            let Some(hit) = hit else {
                return miss();
            };
            match metadata(hit.entity, hit.distance) {
                Some((entity_id, interaction)) => hit_kind(
                    entity_id,
                    hit.point.to_array(),
                    hit.normal.to_array(),
                    hit.distance,
                    interaction,
                ),
                // A collider without a persistent id cannot be named to
                // scripts, so the query reports no usable hit.
                None => miss(),
            }
        }

        let hit_metadata = |entity: engine_physics::Entity, distance: f32| {
            self.runtime
                .with_world(|world| {
                    let entity_id = world.persistent_id(entity)?.to_owned();
                    let interaction = world
                        .get::<engine_scene::components::Interactable>(entity)
                        .filter(|interactable| {
                            interactable.enabled && distance <= interactable.max_distance
                        })
                        .map(|interactable| engine_script::GameplayInteractionSnapshot {
                            prompt: interactable.prompt.clone(),
                            action: interactable.action.clone(),
                            max_distance: interactable.max_distance,
                            grabbable: interactable.grabbable,
                        });
                    Some((entity_id, interaction))
                })
                .flatten()
        };

        match *query {
            GameplayPhysicsQuery::Raycast {
                query_id,
                origin,
                direction,
                max_distance,
                ..
            } => {
                let miss = || GameplayPhysicsQueryResult::RaycastMiss { query_id };
                let Some(physics) = self.physics.as_ref() else {
                    return miss();
                };
                let direction = Vec3::from(direction).normalize_or_zero();
                if direction == Vec3::ZERO {
                    return miss();
                }
                let max_distance = max_distance.min(engine_script::MAX_PHYSICS_QUERY_DISTANCE);
                let hit = physics.raycast_filtered(
                    Vec3::from(origin),
                    direction,
                    max_distance,
                    &query_filter,
                );
                hit_result(
                    hit,
                    hit_metadata,
                    |entity_id, point, normal, distance, interaction| {
                        GameplayPhysicsQueryResult::RaycastHit {
                            query_id,
                            entity_id,
                            point,
                            normal,
                            distance,
                            interaction,
                        }
                    },
                    miss,
                )
            }
            GameplayPhysicsQuery::SphereCast {
                query_id,
                origin,
                radius,
                direction,
                max_distance,
                ..
            } => {
                let miss = || GameplayPhysicsQueryResult::SphereCastMiss { query_id };
                let Some(physics) = self.physics.as_ref() else {
                    return miss();
                };
                let direction = Vec3::from(direction).normalize_or_zero();
                if direction == Vec3::ZERO {
                    return miss();
                }
                let radius = radius.min(engine_script::MAX_PHYSICS_QUERY_DISTANCE);
                let max_distance = max_distance.min(engine_script::MAX_PHYSICS_QUERY_DISTANCE);
                let hit = physics.cast_shape(
                    &engine_physics::ColliderShape::Ball { radius },
                    Vec3::from(origin),
                    direction,
                    max_distance,
                    &query_filter,
                );
                hit_result(
                    hit,
                    hit_metadata,
                    |entity_id, point, normal, distance, interaction| {
                        GameplayPhysicsQueryResult::SphereCastHit {
                            query_id,
                            entity_id,
                            point,
                            normal,
                            distance,
                            interaction,
                        }
                    },
                    miss,
                )
            }
            GameplayPhysicsQuery::OverlapSphere {
                query_id,
                center,
                radius,
                ..
            } => {
                let mut entity_ids = Vec::new();
                if let Some(physics) = self.physics.as_ref() {
                    let radius = radius.min(engine_script::MAX_PHYSICS_QUERY_DISTANCE);
                    let hits = physics.query_proximity_filtered(
                        &engine_physics::ColliderShape::Ball { radius },
                        Vec3::from(center),
                        &query_filter,
                    );
                    let persistent_ids = self
                        .runtime
                        .with_world(|world| {
                            hits.iter()
                                .filter_map(|entity| {
                                    world.persistent_id(*entity).map(str::to_owned)
                                })
                                .collect::<std::collections::BTreeSet<_>>()
                        })
                        .unwrap_or_default();
                    entity_ids.extend(
                        persistent_ids
                            .into_iter()
                            .take(engine_script::MAX_PHYSICS_OVERLAP_RESULTS),
                    );
                }
                GameplayPhysicsQueryResult::OverlapSphere {
                    query_id,
                    entity_ids,
                }
            }
        }
    }
}
