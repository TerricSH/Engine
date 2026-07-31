use super::*;

impl RapierBackend {
    /// Create a new Rapier backend with the given gravity.
    pub fn new(gravity: glam::Vec3) -> Self {
        Self {
            pipeline: PhysicsPipeline::new(),
            gravity: na::Vector3::new(gravity.x, gravity.y, gravity.z),
            integration: IntegrationParameters::default(),
            islands: IslandManager::new(),
            broad_phase: BroadPhaseMultiSap::new(),
            narrow_phase: NarrowPhase::new(),
            bodies: RigidBodySet::new(),
            colliders: ColliderSet::new(),
            impulse_joints: ImpulseJointSet::new(),
            multibody_joints: MultibodyJointSet::new(),
            ccd_solver: CCDSolver::new(),
            query_pipeline: QueryPipeline::new(),
            query_pipeline_dirty: false,
            body_map: HashMap::new(),
            collider_map: HashMap::new(),
            joint_entity_map: HashMap::new(),
            joint_handle_lookup: HashMap::new(),
            joint_break_limits: HashMap::new(),
            joint_bodies: HashMap::new(),
            next_joint_id: 0,
            active_sensor_overlaps: HashSet::new(),
            active_collision_overlaps: HashSet::new(),
        }
    }

    /// Advance the simulation by one fixed timestep.
    ///
    /// Returns both collision (contact) events and trigger (sensor) events.
    pub fn step(&mut self) -> PhysicsEvents {
        let (tx_col, rx_col) = crossbeam_channel::unbounded();
        let (tx_int, rx_int) = crossbeam_channel::unbounded();
        let handler = BackendEventHandler { tx_col, tx_int };

        self.pipeline.step(
            &self.gravity,
            &self.integration,
            &mut self.islands,
            &mut self.broad_phase,
            &mut self.narrow_phase,
            &mut self.bodies,
            &mut self.colliders,
            &mut self.impulse_joints,
            &mut self.multibody_joints,
            &mut self.ccd_solver,
            Some(&mut self.query_pipeline),
            &(),
            &handler,
        );
        let joint_breaks = self.break_overloaded_joints();

        // ── Build reverse collider → entity map ──────────────────────────
        let mut collider_to_entity = HashMap::new();
        for (&entity, &(ch, _)) in &self.collider_map {
            collider_to_entity.insert(ch, entity);
        }

        // ── Resolve collider handle → entity helper ──────────────────────
        let resolve =
            |handle: ColliderHandle| -> Option<Entity> { collider_to_entity.get(&handle).copied() };

        // ── 1. Collect collision (non-trigger) events ─────────────────────
        let mut collisions = Vec::new();
        while let Ok(raw) = rx_col.try_recv() {
            let kind = if raw.started {
                CollisionEventKind::ContactStarted
            } else {
                CollisionEventKind::ContactStopped
            };
            if let (Some(a), Some(b)) = (resolve(raw.collider1), resolve(raw.collider2)) {
                collisions.push(CollisionEvent {
                    kind,
                    entity_a: a,
                    entity_b: b,
                });
            }
        }

        // ── 2. Collect intersection (sensor/trigger) events ───────────────
        //
        // Rapier fires IntersectionEvent only on state changes (start/stop
        // intersecting).  Persistent overlaps between frames do NOT produce
        // events.  We therefore use a two-pass approach:
        //
        //   a) Read the event channel → Entered / Exited.
        //   b) Post-step query-pipeline scan → derive Stay for overlapping
        //      pairs that were already active last frame.

        let mut triggers = Vec::new();
        let mut event_overlaps: HashSet<EntityPair> = HashSet::new();

        // Helper: build a canonical complete-entity key.
        let entity_key = |c1: ColliderHandle, c2: ColliderHandle| -> Option<EntityPair> {
            Some(canonical_entity_pair(resolve(c1)?, resolve(c2)?))
        };

        // ── 2a. Process intersection events from the channel ────────────
        while let Ok(raw) = rx_int.try_recv() {
            if raw.intersecting {
                let key = match entity_key(raw.collider1, raw.collider2) {
                    Some(k) => k,
                    None => continue,
                };
                let kind = if self.active_sensor_overlaps.contains(&key) {
                    TriggerEventKind::Stay
                } else {
                    TriggerEventKind::Entered
                };
                if let (Some(a), Some(b)) = (resolve(raw.collider1), resolve(raw.collider2)) {
                    triggers.push(TriggerEvent {
                        kind,
                        entity_a: a,
                        entity_b: b,
                    });
                }
                event_overlaps.insert(key);
            } else {
                // Exited — Rapier reports when two colliders stop intersecting.
                let key = match entity_key(raw.collider1, raw.collider2) {
                    Some(k) => k,
                    None => continue,
                };
                if self.active_sensor_overlaps.contains(&key) {
                    if let (Some(a), Some(b)) = (resolve(raw.collider1), resolve(raw.collider2)) {
                        triggers.push(TriggerEvent {
                            kind: TriggerEventKind::Exited,
                            entity_a: a,
                            entity_b: b,
                        });
                    }
                }
            }
        }

        // ── 2b. Narrow-phase scan for Stay events ─────────────────────
        //
        // Rapier's NarrowPhase tracks ALL active contact and intersection
        // pairs after a step — read them directly instead of doing O(n)
        // per-collider query pipeline scans.
        let mut full_overlaps: HashSet<EntityPair> = HashSet::new();
        let mut collision_overlaps: HashSet<EntityPair> = HashSet::new();

        // Read all active intersection (sensor) pairs.
        // Rapier returns (ColliderHandle, ColliderHandle, intersecting: bool).
        for (c1, c2, _intersecting) in self.narrow_phase.intersection_pairs() {
            let Some(&e1) = collider_to_entity.get(&c1) else {
                continue;
            };
            let Some(&e2) = collider_to_entity.get(&c2) else {
                continue;
            };
            full_overlaps.insert(canonical_entity_pair(e1, e2));
        }

        // Read all active contact (non-sensor) pairs.
        for pair in self.narrow_phase.contact_pairs() {
            let Some(&e1) = collider_to_entity.get(&pair.collider1) else {
                continue;
            };
            let Some(&e2) = collider_to_entity.get(&pair.collider2) else {
                continue;
            };
            collision_overlaps.insert(canonical_entity_pair(e1, e2));
        }

        // Generate Stay for sensor overlaps that persisted from last frame
        // but were NOT reported as new by Rapier's event stream.
        for &key in &full_overlaps {
            if self.active_sensor_overlaps.contains(&key) && !event_overlaps.contains(&key) {
                triggers.push(TriggerEvent {
                    kind: TriggerEventKind::Stay,
                    entity_a: key.0,
                    entity_b: key.1,
                });
            }
        }

        self.active_sensor_overlaps = full_overlaps;

        // Generate ContactStaying for collision pairs that persisted.
        for &key in &collision_overlaps {
            if self.active_collision_overlaps.contains(&key) {
                collisions.push(CollisionEvent {
                    kind: CollisionEventKind::ContactStaying,
                    entity_a: key.0,
                    entity_b: key.1,
                });
            }
        }

        self.active_collision_overlaps = collision_overlaps;
        self.query_pipeline_dirty = false;

        PhysicsEvents {
            collisions,
            triggers,
            joint_breaks,
        }
    }
}
