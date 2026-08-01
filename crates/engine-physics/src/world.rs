use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use tracing::debug;

use crate::backend::{RapierBackend, RaycastHit};
use crate::components::{ColliderShape, RigidBody};
use crate::debug::{ColliderDebugInfo, PhysicsDebugDraw};
use crate::events::{CollisionEvent, JointBreakEvent, PhysicsEvents, TriggerEvent};
use crate::gravity::{sum_source_gravity, GravitySource};
use crate::joints::{JointDescriptor, JointHandle, PhysicsJoint};
use crate::queries::{
    OverlapQuery, PhysicsQueryFilter, QueryBatcher, QueryResults, RaycastQuery, SweepQuery,
};
use crate::{BodyType, Collider, Entity, PhysicsMaterial, Transform};

// ── PhysicsCommand ──────────────────────────────────────────────────────────

/// Commands that can be queued for safe execution during the next physics step.
///
/// These are accumulated during a frame and executed at the start of the
/// next `PhysicsWorld::step()` call, avoiding mid-frame mutation of backend
/// state.
#[derive(Clone, Debug)]
pub enum PhysicsCommand {
    /// Apply a continuous force at the centre of mass.
    ApplyForce { entity: Entity, force: glam::Vec3 },
    /// Apply an instantaneous impulse at the centre of mass.
    ApplyImpulse { entity: Entity, impulse: glam::Vec3 },
    /// Apply a continuous torque.
    ApplyTorque { entity: Entity, torque: glam::Vec3 },
    /// Apply an instantaneous angular impulse.
    ApplyTorqueImpulse {
        entity: Entity,
        torque_impulse: glam::Vec3,
    },
    /// Teleport the body to a new position.
    SetBodyPosition {
        entity: Entity,
        position: glam::Vec3,
    },
    /// Teleport the body to a new rotation.
    SetBodyRotation {
        entity: Entity,
        rotation: glam::Quat,
    },
    SetLinearVelocity {
        entity: Entity,
        velocity: glam::Vec3,
    },
    SetAngularVelocity {
        entity: Entity,
        velocity: glam::Vec3,
    },
    /// Switch physics ownership without rebuilding the body or its joints.
    SetBodyType { entity: Entity, body_type: BodyType },
}

/// Serializable simulation state that is not part of the authored
/// [`RigidBody`] component.
///
/// Save games use this to preserve moving props across a checkpoint. Backend
/// handles and solver caches remain private and are rebuilt on restore.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RigidBodyRuntimeState {
    pub position: [f32; 3],
    pub rotation: [f32; 4],
    pub linear_velocity: [f32; 3],
    pub angular_velocity: [f32; 3],
    pub sleeping: bool,
}

impl RigidBodyRuntimeState {
    pub fn validate(&self) -> Result<(), String> {
        if !self
            .position
            .iter()
            .chain(self.rotation.iter())
            .chain(self.linear_velocity.iter())
            .chain(self.angular_velocity.iter())
            .all(|value| value.is_finite())
        {
            return Err("rigid-body runtime state contains a non-finite value".into());
        }
        let rotation_length_squared = self.rotation.iter().map(|value| value * value).sum::<f32>();
        if rotation_length_squared <= f32::EPSILON {
            return Err("rigid-body runtime state has a zero-length rotation".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
struct SyncedSceneJoint {
    component: PhysicsJoint,
    entity_a: Entity,
    entity_b: Entity,
    handle: JointHandle,
}

#[derive(Clone, Debug, PartialEq)]
struct SyncedCollider {
    component: Collider,
    material: Option<PhysicsMaterial>,
}

// ── PhysicsWorld ────────────────────────────────────────────────────────────

/// The main physics simulation world.
///
/// Owns a [`RapierBackend`] and coordinates:
/// - Fixed-timestep simulation with accumulated delta time
/// - Bidirectional ECS synchronisation (`sync_from_ecs` / `sync_to_ecs`)
/// - A command queue for safe mid-frame mutation
/// - Collision event collection
/// - Debug draw data propagation
pub struct PhysicsWorld {
    pub(crate) backend: RapierBackend,
    gravity: glam::Vec3,
    fixed_timestep: f32,
    accumulator: f32,
    pending_commands: Vec<PhysicsCommand>,
    pending_events: Vec<CollisionEvent>,
    pending_triggers: Vec<TriggerEvent>,
    pending_joint_breaks: Vec<JointBreakEvent>,
    scene_joints: HashMap<Entity, SyncedSceneJoint>,
    synced_colliders: HashMap<Entity, SyncedCollider>,
    debug_colliders: Arc<Mutex<Vec<ColliderDebugInfo>>>,
    /// Accumulated queries waiting for batched execution.
    query_batcher: QueryBatcher,
    /// Dynamic bodies whose Rapier gravity scale is currently zeroed because
    /// at least one [`GravitySource`] shapes their effective gravity. Used to
    /// restore the ECS-authored `gravity_scale` when a body falls back to the
    /// global gravity.
    gravity_overridden: HashSet<Entity>,
}

impl PhysicsWorld {
    /// Create a new physics world with default gravity (0, -9.81, 0).
    pub fn new(gravity: glam::Vec3) -> Self {
        Self {
            backend: RapierBackend::new(gravity),
            gravity,
            fixed_timestep: 1.0 / 60.0,
            accumulator: 0.0,
            pending_commands: Vec::new(),
            pending_events: Vec::new(),
            pending_triggers: Vec::new(),
            pending_joint_breaks: Vec::new(),
            scene_joints: HashMap::new(),
            synced_colliders: HashMap::new(),
            debug_colliders: Arc::new(Mutex::new(Vec::new())),
            query_batcher: QueryBatcher::new(),
            gravity_overridden: HashSet::new(),
        }
    }

    /// Create a `PhysicsWorld` that shares its debug collider data with the
    /// given `PhysicsDebugDraw` provider.
    pub fn with_debug_draw(gravity: glam::Vec3, debug: &PhysicsDebugDraw) -> Self {
        Self {
            debug_colliders: debug.shared_data(),
            ..Self::new(gravity)
        }
    }

    // ── Configuration ───────────────────────────────────────────────────

    /// Set the gravity vector.
    pub fn set_gravity(&mut self, gravity: glam::Vec3) {
        self.gravity = gravity;
        self.backend.gravity = glam_to_rapier_vec(gravity);
    }

    /// Current gravity vector.
    pub fn gravity(&self) -> glam::Vec3 {
        self.gravity
    }

    /// Set the fixed timestep in seconds (default: 1/60).
    pub fn set_fixed_timestep(&mut self, dt: f32) {
        self.fixed_timestep = dt;
    }

    /// Get the current fixed timestep.
    pub fn fixed_timestep(&self) -> f32 {
        self.fixed_timestep
    }

    // ── Command queue ───────────────────────────────────────────────────

    /// Queue a command for execution during the next physics step.
    ///
    /// This is the safe way to apply forces, impulses, or teleport bodies
    /// from game code running outside the physics step.
    pub fn queue_command(&mut self, cmd: PhysicsCommand) {
        self.pending_commands.push(cmd);
    }

    /// Drain and execute all queued commands.
    fn execute_pending_commands(&mut self, world: &mut crate::World) {
        let commands = std::mem::take(&mut self.pending_commands);
        for cmd in commands {
            let entity = match &cmd {
                PhysicsCommand::ApplyForce { entity, .. }
                | PhysicsCommand::ApplyImpulse { entity, .. }
                | PhysicsCommand::ApplyTorque { entity, .. }
                | PhysicsCommand::ApplyTorqueImpulse { entity, .. }
                | PhysicsCommand::SetBodyPosition { entity, .. }
                | PhysicsCommand::SetBodyRotation { entity, .. }
                | PhysicsCommand::SetLinearVelocity { entity, .. }
                | PhysicsCommand::SetAngularVelocity { entity, .. }
                | PhysicsCommand::SetBodyType { entity, .. } => *entity,
            };
            if !world.is_alive(entity) {
                continue;
            }

            match cmd {
                PhysicsCommand::ApplyForce { entity, force } => {
                    self.backend.apply_force(entity, force);
                }
                PhysicsCommand::ApplyImpulse { entity, impulse } => {
                    self.backend.apply_impulse(entity, impulse);
                }
                PhysicsCommand::ApplyTorque { entity, torque } => {
                    self.backend.apply_torque(entity, torque);
                }
                PhysicsCommand::ApplyTorqueImpulse {
                    entity,
                    torque_impulse,
                } => {
                    self.backend.apply_torque_impulse(entity, torque_impulse);
                }
                PhysicsCommand::SetBodyPosition { entity, position } => {
                    if let Some((_, rot)) = self.backend.sync_body_transform(entity) {
                        self.backend.set_body_transform(entity, position, rot);
                    }
                }
                PhysicsCommand::SetBodyRotation { entity, rotation } => {
                    if let Some((pos, _)) = self.backend.sync_body_transform(entity) {
                        self.backend.set_body_transform(entity, pos, rotation);
                    }
                }
                PhysicsCommand::SetLinearVelocity { entity, velocity } => {
                    self.backend.set_linear_velocity(entity, velocity);
                }
                PhysicsCommand::SetAngularVelocity { entity, velocity } => {
                    self.backend.set_angular_velocity(entity, velocity);
                }
                PhysicsCommand::SetBodyType { entity, body_type } => {
                    if let Some(component) = world.get_mut::<RigidBody>(entity) {
                        component.body_type = body_type;
                    }
                    self.backend.set_body_type(entity, body_type);
                }
            }
        }
    }

    // ── Simulation ──────────────────────────────────────────────────────

    /// Advance the simulation by `dt` seconds using a fixed timestep
    /// accumulator.
    ///
    /// Processes queued commands, runs ECS → physics sync, steps the
    /// simulation the required number of times, and collects collision
    /// events.
    pub fn step(&mut self, dt: f32, world: &mut crate::World) {
        // 1. Synchronise ECS → physics so stale generations are removed and
        // the current live generation is installed before commands run.
        self.sync_from_ecs_internal(world);

        // 2. Execute commands only against the current live generation.
        self.execute_pending_commands(world);

        // 3. Fixed timestep accumulator.
        self.accumulator += dt;
        let max_steps = 8; // safety limit to prevent spiral of death
        let mut steps_taken = 0;

        while self.accumulator >= self.fixed_timestep && steps_taken < max_steps {
            self.backend.integration.dt = self.fixed_timestep;

            // Resolve per-body gravity from active gravity sources for this
            // fixed step (no-op fast path when no sources exist).
            self.apply_gravity_sources(world);

            // Run one physics step and capture both collision and trigger events.
            let events = self.backend.step();
            self.pending_events.extend(events.collisions);
            self.pending_triggers.extend(events.triggers);
            for mut event in events.joint_breaks {
                let joint_entity = self.scene_joints.iter().find_map(|(entity, synced)| {
                    (synced.handle == event.handle).then_some(*entity)
                });
                if let Some(joint_entity) = joint_entity {
                    event.joint_entity = Some(joint_entity);
                    self.scene_joints.remove(&joint_entity);
                    // A broken persistent joint must not be recreated by the
                    // next ECS sync or by a checkpoint captured afterwards.
                    world.remove_component::<PhysicsJoint>(joint_entity);
                }
                self.pending_joint_breaks.push(event);
            }

            self.accumulator -= self.fixed_timestep;
            steps_taken += 1;
        }

        // A frame may not advance the fixed-step simulation. Structural ECS
        // changes must still be visible to every query API immediately.
        if self.backend.query_pipeline_is_dirty() {
            self.backend.sync_query_pipeline();
        }

        // Clamp accumulator to prevent large catch-up after a pause.
        if self.accumulator > self.fixed_timestep * 4.0 {
            self.accumulator = 0.0;
        }

        // 4. Sync physics → ECS (write back transforms).
        self.sync_to_ecs_internal(world);

        // 5. Update debug collider data.
        if let Ok(mut debug) = self.debug_colliders.lock() {
            *debug = self
                .backend
                .collider_debug_info()
                .into_iter()
                .map(|(shape, pos, rot)| ColliderDebugInfo {
                    shape,
                    position: pos,
                    rotation: rot,
                })
                .collect();
        }

        debug!(
            dt = self.fixed_timestep,
            steps = steps_taken,
            bodies = self.backend.bodies.len(),
            colliders = self.backend.colliders.len(),
            events = self.pending_events.len(),
            "Physics step complete"
        );
    }

    // ── ECS synchronisation ─────────────────────────────────────────────

    /// Whether the backend currently has a body registered for `entity`.
    pub fn has_body(&self, entity: Entity) -> bool {
        self.backend.has_body(entity)
    }

    /// Number of bodies currently registered in the backend.
    pub fn body_count(&self) -> usize {
        self.backend.body_map.len()
    }

    /// Snapshot every registered body's transient simulation state.
    pub fn runtime_body_states(&self) -> Vec<(Entity, RigidBodyRuntimeState)> {
        self.backend.runtime_body_states()
    }

    /// Restore transient state after the authored ECS body has been rebuilt.
    ///
    /// Returns `false` if the state is invalid or the entity has no body.
    pub fn restore_runtime_body_state(
        &mut self,
        entity: Entity,
        state: &RigidBodyRuntimeState,
    ) -> bool {
        if state.validate().is_err() {
            return false;
        }
        self.backend.restore_runtime_body_state(entity, state)
    }

    /// Synchronise ECS components → Rapier backend.
    ///
    /// Creates bodies/colliders for entities that have the relevant
    /// components but are not yet registered in the backend. Removes
    /// bodies/colliders for entities that no longer have the components.
    pub fn sync_from_ecs(&mut self, world: &crate::World) {
        self.sync_from_ecs_internal(world);
        self.backend.sync_query_pipeline();
    }

    fn sync_from_ecs_internal(&mut self, world: &crate::World) {
        // Collect all complete entity handles that have RigidBody components.
        let mut seen_bodies: HashSet<Entity> = HashSet::new();

        for (entity, rigid_body) in world.query::<RigidBody>() {
            seen_bodies.insert(entity);

            if !self.backend.has_body(entity) {
                debug_assert!(world.is_alive(entity));
                // Get the Transform for positioning.
                let transform = world.get::<Transform>(entity).cloned().unwrap_or_default();
                self.backend
                    .replace_body_for_current_entity(entity, rigid_body, &transform);
            }
        }

        // Remove bodies for entities that no longer have RigidBody.
        let to_remove_bodies: Vec<Entity> = self
            .backend
            .body_map
            .keys()
            .copied()
            .filter(|entity| !seen_bodies.contains(entity))
            .collect();
        for entity in to_remove_bodies {
            self.backend.remove_body(entity);
        }

        // Collect all complete entity handles that have Collider components.
        let mut seen_colliders: HashSet<Entity> = HashSet::new();

        for (entity, collider) in world.query::<Collider>() {
            seen_colliders.insert(entity);
            let material = world.get::<PhysicsMaterial>(entity).cloned();
            let desired = SyncedCollider {
                component: collider.clone(),
                material: material.clone(),
            };
            let changed = self
                .synced_colliders
                .get(&entity)
                .is_none_or(|synced| synced != &desired);
            if changed && self.backend.has_collider(entity) {
                self.backend.remove_collider(entity);
            }
            if !self.backend.has_collider(entity) {
                debug_assert!(world.is_alive(entity));
                // Find the parent body entity. A collider should be attached
                // to the same entity's rigid body, or we search for the first
                // ancestor with a RigidBody.
                let body_entity = if self.backend.has_body(entity) {
                    entity
                } else {
                    // Search for a parent entity with a RigidBody
                    // (simple: same entity or parent chain — we use same entity for now)
                    entity
                };

                self.backend.replace_collider_for_current_entity(
                    entity,
                    collider,
                    body_entity,
                    material.as_ref(),
                );
            }
            if self.backend.has_collider(entity) {
                self.synced_colliders.insert(entity, desired);
            } else {
                self.synced_colliders.remove(&entity);
            }
        }

        // Remove colliders for entities that no longer have Collider.
        let to_remove_colliders: Vec<Entity> = self
            .backend
            .collider_map
            .keys()
            .copied()
            .filter(|entity| !seen_colliders.contains(entity))
            .collect();
        for entity in to_remove_colliders {
            self.backend.remove_collider(entity);
            self.synced_colliders.remove(&entity);
        }
        self.synced_colliders
            .retain(|entity, _| seen_colliders.contains(entity));

        self.sync_joints_from_ecs(world);
    }

    fn sync_joints_from_ecs(&mut self, world: &crate::World) {
        let desired = world
            .query::<PhysicsJoint>()
            .filter(|(_, joint)| joint.enabled && joint.validate().is_ok())
            .filter_map(|(joint_entity, joint)| {
                let entity_a = world.entity_by_persistent_id(&joint.body_a)?;
                let entity_b = world.entity_by_persistent_id(&joint.body_b)?;
                (self.backend.has_body(entity_a) && self.backend.has_body(entity_b))
                    .then_some((joint_entity, (joint.clone(), entity_a, entity_b)))
            })
            .collect::<HashMap<_, _>>();

        let stale = self
            .scene_joints
            .iter()
            .filter_map(|(joint_entity, synced)| {
                let keep =
                    desired
                        .get(joint_entity)
                        .is_some_and(|(component, entity_a, entity_b)| {
                            component == &synced.component
                                && entity_a == &synced.entity_a
                                && entity_b == &synced.entity_b
                                && self.backend.has_joint(synced.handle)
                        });
                (!keep).then_some((*joint_entity, synced.handle))
            })
            .collect::<Vec<_>>();
        for (joint_entity, handle) in stale {
            self.backend.remove_joint(handle);
            self.scene_joints.remove(&joint_entity);
        }

        for (joint_entity, (component, entity_a, entity_b)) in desired {
            if self.scene_joints.contains_key(&joint_entity) {
                continue;
            }
            let descriptor = component.descriptor(entity_a, entity_b);
            if let Some(handle) = self.create_joint(descriptor) {
                self.scene_joints.insert(
                    joint_entity,
                    SyncedSceneJoint {
                        component,
                        entity_a,
                        entity_b,
                        handle,
                    },
                );
            }
        }
    }

    /// Synchronise Rapier backend → ECS components.
    ///
    /// Writes the world-space position of each physics body back into the
    /// entity's `Transform` component.
    pub fn sync_to_ecs(&mut self, world: &mut crate::World) {
        self.sync_to_ecs_internal(world);
    }

    /// Teleport every registered body by `offset`, preserving simulation
    /// state.
    ///
    /// World-origin shifts use this instead of rebuilding the physics world:
    /// each body's rotation, velocities, accumulated forces, joints, and
    /// sleep state survive the teleport exactly, so a moving body continues
    /// seamlessly and a sleeping body stays asleep. The query pipeline is
    /// refreshed immediately so raycasts and overlap queries issued before
    /// the next [`step`](Self::step) observe the shifted positions.
    ///
    /// Returns the number of bodies teleported.
    pub fn translate_bodies(&mut self, offset: glam::Vec3) -> usize {
        let entities: Vec<Entity> = self.backend.body_map.keys().copied().collect();
        let mut teleported = 0;
        for entity in entities {
            if self.backend.translate_body(entity, offset) {
                teleported += 1;
            }
        }
        if teleported > 0 {
            // Bodies were teleported outside a step: propagate their poses to
            // the attached colliders, then refresh the query pipeline so
            // raycasts/overlaps observe the shift before the next step.
            self.backend.propagate_body_positions_to_colliders();
            self.backend.sync_query_pipeline();
        }
        teleported
    }

    fn sync_to_ecs_internal(&mut self, world: &mut crate::World) {
        let body_entities: Vec<Entity> = self.backend.body_map.keys().copied().collect();

        for entity in body_entities {
            // Only sync if the entity is still alive and has a Transform.
            if !world.is_alive(entity) {
                continue;
            }

            if let Some((pos, rot)) = self.backend.sync_body_transform(entity) {
                if let Some(transform) = world.get_mut::<Transform>(entity) {
                    transform.translation = pos;
                    transform.rotation = rot;
                }
            }
        }
    }

    // ── Gravity sources ───────────────────────────────────────────────

    /// Apply effective per-body gravity from active [`GravitySource`]
    /// components for the next fixed step.
    ///
    /// Semantics (see the `gravity` module docs): contributions from all
    /// in-range sources are summed per dynamic body; bodies no source reaches
    /// keep the configured global gravity. Rapier only supports a single
    /// global gravity vector scaled per body, so source-driven bodies get
    /// their Rapier gravity scale zeroed and receive the effective gravity
    /// (times their ECS `gravity_scale`) as a mass-normalised impulse each
    /// fixed step, which supports arbitrary pull directions. Bodies that fall
    /// back to global gravity have their ECS-authored scale restored.
    ///
    /// Because sources are re-read from the ECS world every fixed step,
    /// runtime edits to `GravitySource` components (including script writes
    /// through the component bridge) take effect on the next physics step.
    fn apply_gravity_sources(&mut self, world: &crate::World) {
        let sources: Vec<&GravitySource> = world
            .query::<GravitySource>()
            .map(|(_, source)| source)
            .collect();

        let mut still_overridden: HashSet<Entity> = HashSet::new();

        if !sources.is_empty() {
            let dt = self.fixed_timestep;
            let body_entities: Vec<Entity> = self.backend.body_map.keys().copied().collect();
            for entity in body_entities {
                // Gravity only acts on enabled dynamic bodies.
                let Some(rigid_body) = world.get::<RigidBody>(entity) else {
                    continue;
                };
                if rigid_body.body_type != BodyType::Dynamic || !rigid_body.enabled {
                    continue;
                }
                let Some((position, _)) = self.backend.sync_body_transform(entity) else {
                    continue;
                };

                let Some(acceleration) = sum_source_gravity(sources.iter().copied(), position)
                else {
                    continue;
                };

                // Zero the global-gravity multiplier once; the per-step
                // impulse below fully drives this body's gravity.
                if !self.gravity_overridden.contains(&entity) {
                    self.backend.set_body_gravity_scale(entity, 0.0);
                }
                still_overridden.insert(entity);

                let scaled = acceleration * rigid_body.gravity_scale;
                if scaled.length_squared() <= f32::EPSILON {
                    // Fields cancel out (or the body sits at a source
                    // centre): no impulse, so the body may still sleep.
                    continue;
                }
                let mass = self.backend.body_mass(entity).unwrap_or(0.0);
                if mass <= 0.0 {
                    continue;
                }
                // dv = a·dt is mass-independent, so impulse = m·a·dt.
                self.backend.apply_impulse(entity, scaled * (mass * dt));
            }
        }

        // Restore the ECS-authored gravity scale on bodies that fell back to
        // the global gravity (out of range, sources removed/disabled, or no
        // sources left in the world at all).
        let to_restore: Vec<Entity> = self
            .gravity_overridden
            .difference(&still_overridden)
            .copied()
            .collect();
        for entity in to_restore {
            if let Some(rigid_body) = world.get::<RigidBody>(entity) {
                self.backend
                    .set_body_gravity_scale(entity, rigid_body.gravity_scale);
            }
        }
        self.gravity_overridden = still_overridden;
    }

    // ── Collision events ────────────────────────────────────────────────

    /// Drain all events (collisions + triggers) collected during the last step.
    pub fn drain_events(&mut self) -> PhysicsEvents {
        PhysicsEvents {
            collisions: std::mem::take(&mut self.pending_events),
            triggers: std::mem::take(&mut self.pending_triggers),
            joint_breaks: std::mem::take(&mut self.pending_joint_breaks),
        }
    }

    /// Read (without draining) the pending collision events.
    pub fn pending_events(&self) -> &[CollisionEvent] {
        &self.pending_events
    }

    /// Read (without draining) the pending trigger events.
    pub fn pending_triggers(&self) -> &[TriggerEvent] {
        &self.pending_triggers
    }

    pub fn pending_joint_breaks(&self) -> &[JointBreakEvent] {
        &self.pending_joint_breaks
    }

    // ── Queries ─────────────────────────────────────────────────────────

    /// Cast a ray and return the closest hit.
    pub fn raycast(
        &self,
        origin: glam::Vec3,
        direction: glam::Vec3,
        max_distance: f32,
    ) -> Option<RaycastHit> {
        self.backend.raycast(origin, direction, max_distance)
    }

    /// Cast a ray with a candidate filter and return the closest hit.
    pub fn raycast_filtered(
        &self,
        origin: glam::Vec3,
        direction: glam::Vec3,
        max_distance: f32,
        filter: &PhysicsQueryFilter,
    ) -> Option<RaycastHit> {
        self.backend
            .raycast_filtered(origin, direction, max_distance, filter)
    }

    /// Alias for `raycast`, used by engine-character.
    pub fn cast_ray(
        &self,
        origin: glam::Vec3,
        direction: glam::Vec3,
        max_distance: f32,
    ) -> Option<RaycastHit> {
        self.backend.raycast(origin, direction, max_distance)
    }

    /// Sweep a shape along a direction and return the closest hit.
    ///
    /// Reuses the [`RaycastHit`] payload: contact point and outward normal
    /// on the hit collider, plus the sweep travel distance.
    pub fn cast_shape(
        &self,
        shape: &ColliderShape,
        origin: glam::Vec3,
        direction: glam::Vec3,
        max_distance: f32,
        filter: &PhysicsQueryFilter,
    ) -> Option<RaycastHit> {
        self.backend
            .cast_shape_filtered(shape, origin, direction, max_distance, filter)
    }

    /// Find all entities whose colliders overlap with the given shape.
    pub fn query_proximity(&self, shape: &ColliderShape, position: glam::Vec3) -> Vec<Entity> {
        self.backend.query_proximity(shape, position)
    }

    /// Find all entities whose colliders overlap with the given shape,
    /// honouring a candidate filter.
    pub fn query_proximity_filtered(
        &self,
        shape: &ColliderShape,
        position: glam::Vec3,
        filter: &PhysicsQueryFilter,
    ) -> Vec<Entity> {
        self.backend
            .query_proximity_filtered(shape, position, filter)
    }

    // ── Batched queries ──────────────────────────────────────────────

    /// Queue a raycast query for batched execution.
    ///
    /// The query will be executed when [`execute_queries`] is called.
    pub fn queue_raycast(&mut self, query: RaycastQuery) {
        self.query_batcher.push_raycast(query);
    }

    /// Queue an overlap (proximity) query for batched execution.
    ///
    /// The query will be executed when [`execute_queries`] is called.
    pub fn queue_overlap(&mut self, query: OverlapQuery) {
        self.query_batcher.push_overlap(query);
    }

    /// Queue a sweep (shape cast) query for batched execution.
    ///
    /// The query will be executed when [`execute_queries`] is called.
    pub fn queue_sweep(&mut self, query: SweepQuery) {
        self.query_batcher.push_sweep(query);
    }

    /// Execute all queued batched queries and return the results.
    ///
    /// After calling this method the internal batcher is cleared so that
    /// new queries can be queued for the next frame.
    pub fn execute_queries(&mut self) -> QueryResults {
        let batcher = std::mem::take(&mut self.query_batcher);
        if batcher.is_empty() {
            return QueryResults::new();
        }
        if self.backend.query_pipeline_is_dirty() {
            self.backend.sync_query_pipeline();
        }
        self.backend.execute_batched_queries(&batcher)
    }

    // ── Joint API ───────────────────────────────────────────────────────

    /// Create a joint between two entities.
    ///
    /// Returns `None` if either entity does not have a registered rigid body.
    pub fn create_joint(&mut self, desc: JointDescriptor) -> Option<JointHandle> {
        let body_a_handle = *self.backend.body_map.get(&desc.entity_a)?;
        let body_b_handle = *self.backend.body_map.get(&desc.entity_b)?;
        self.backend
            .create_joint(&desc, body_a_handle, body_b_handle)
    }

    /// Remove a joint by its handle.
    pub fn remove_joint(&mut self, handle: JointHandle) {
        self.backend.remove_joint(handle);
    }

    /// Number of active joints in the simulation.
    pub fn joint_count(&self) -> usize {
        self.backend.joint_count()
    }

    // ── Debug draw ──────────────────────────────────────────────────────

    /// Return a reference to the shared debug collider data.
    pub fn debug_colliders(&self) -> Arc<Mutex<Vec<ColliderDebugInfo>>> {
        self.debug_colliders.clone()
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn glam_to_rapier_vec(v: glam::Vec3) -> rapier3d::na::Vector3<f32> {
    rapier3d::na::Vector3::new(v.x, v.y, v.z)
}
