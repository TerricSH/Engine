use std::sync::{Arc, Mutex};

use tracing::debug;

use crate::backend::{RapierBackend, RaycastHit};
use crate::components::{ColliderShape, RigidBody};
use crate::debug::{ColliderDebugInfo, PhysicsDebugDraw};
use crate::events::{CollisionEvent, PhysicsEvents};
use crate::joints::{JointDescriptor, JointHandle};
use crate::queries::{
    OverlapQuery, QueryBatcher, QueryResults, RaycastQuery, SweepQuery,
};
use crate::{Collider, Entity, PhysicsMaterial, Transform};

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
    debug_colliders: Arc<Mutex<Vec<ColliderDebugInfo>>>,
    /// Accumulated queries waiting for batched execution.
    query_batcher: QueryBatcher,
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
            debug_colliders: Arc::new(Mutex::new(Vec::new())),
            query_batcher: QueryBatcher::new(),
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
    fn execute_pending_commands(&mut self) {
        let commands = std::mem::take(&mut self.pending_commands);
        for cmd in commands {
            match cmd {
                PhysicsCommand::ApplyForce { entity, force } => {
                    self.backend.apply_force(entity.index(), force);
                }
                PhysicsCommand::ApplyImpulse { entity, impulse } => {
                    self.backend.apply_impulse(entity.index(), impulse);
                }
                PhysicsCommand::SetBodyPosition { entity, position } => {
                    if let Some((_, rot)) = self.backend.sync_body_transform(entity.index()) {
                        self.backend
                            .set_body_transform(entity.index(), position, rot);
                    }
                }
                PhysicsCommand::SetBodyRotation { entity, rotation } => {
                    if let Some((pos, _)) = self.backend.sync_body_transform(entity.index()) {
                        self.backend
                            .set_body_transform(entity.index(), pos, rotation);
                    }
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
        // 1. Execute pending commands.
        self.execute_pending_commands();

        // 2. Synchronise ECS → physics (creates/updates bodies and colliders).
        self.sync_from_ecs_internal(world);

        // 3. Fixed timestep accumulator.
        self.accumulator += dt;
        let max_steps = 8; // safety limit to prevent spiral of death
        let mut steps_taken = 0;

        while self.accumulator >= self.fixed_timestep && steps_taken < max_steps {
            self.backend.integration.dt = self.fixed_timestep;

            // Run one physics step and capture events.
            let events = self.backend.step();
            self.pending_events.extend(events);

            self.accumulator -= self.fixed_timestep;
            steps_taken += 1;
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
        // Collect all entity indices that have RigidBody components.
        let mut seen_bodies: Vec<u32> = Vec::new();

        for (entity, rigid_body) in world.query::<RigidBody>() {
            let idx = entity.index();
            seen_bodies.push(idx);

            if !self.backend.has_body(idx) {
                // Get the Transform for positioning.
                let transform = world.get::<Transform>(entity).cloned().unwrap_or_default();
                self.backend.create_body(idx, rigid_body, &transform);
            }
        }

        // Remove bodies for entities that no longer have RigidBody.
        let to_remove_bodies: Vec<u32> = self
            .backend
            .body_map
            .keys()
            .copied()
            .filter(|idx| !seen_bodies.contains(idx))
            .collect();
        for idx in to_remove_bodies {
            self.backend.remove_body(idx);
        }

        // Collect all entity indices that have Collider components.
        let mut seen_colliders: Vec<u32> = Vec::new();

        for (entity, collider) in world.query::<Collider>() {
            let idx = entity.index();
            seen_colliders.push(idx);

            if !self.backend.has_collider(idx) {
                // Find the parent body entity. A collider should be attached
                // to the same entity's rigid body, or we search for the first
                // ancestor with a RigidBody.
                let body_entity = if self.backend.has_body(idx) {
                    idx
                } else {
                    // Search for a parent entity with a RigidBody
                    // (simple: same entity or parent chain — we use same entity for now)
                    idx
                };

                let material = world.get::<PhysicsMaterial>(entity);
                self.backend
                    .create_collider(idx, collider, body_entity, material);
            }
        }

        // Remove colliders for entities that no longer have Collider.
        let to_remove_colliders: Vec<u32> = self
            .backend
            .collider_map
            .keys()
            .copied()
            .filter(|idx| !seen_colliders.contains(idx))
            .collect();
        for idx in to_remove_colliders {
            self.backend.remove_collider(idx);
        }
    }

    /// Synchronise Rapier backend → ECS components.
    ///
    /// Writes the world-space position of each physics body back into the
    /// entity's `Transform` component.
    pub fn sync_to_ecs(&mut self, world: &mut crate::World) {
        self.sync_to_ecs_internal(world);
    }

    fn sync_to_ecs_internal(&mut self, world: &mut crate::World) {
        let body_indices: Vec<u32> = self.backend.body_map.keys().copied().collect();

        for idx in body_indices {
            let entity = Entity::new(idx, 0);

            // Only sync if the entity is still alive and has a Transform.
            if !world.is_alive(entity) {
                continue;
            }

            if let Some((pos, rot)) = self.backend.sync_body_transform(idx) {
                if let Some(transform) = world.get_mut::<Transform>(entity) {
                    transform.translation = pos;
                    transform.rotation = rot;
                }
            }
        }
    }

    // ── Collision events ────────────────────────────────────────────────

    /// Drain all collision events that were collected during the last step.
    pub fn drain_events(&mut self) -> PhysicsEvents {
        PhysicsEvents {
            events: std::mem::take(&mut self.pending_events),
        }
    }

    /// Read (without draining) the pending collision events.
    pub fn pending_events(&self) -> &[CollisionEvent] {
        &self.pending_events
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

    /// Alias for `raycast`, used by engine-character.
    pub fn cast_ray(
        &self,
        origin: glam::Vec3,
        direction: glam::Vec3,
        max_distance: f32,
    ) -> Option<RaycastHit> {
        self.backend.raycast(origin, direction, max_distance)
    }

    /// Find all entities whose colliders overlap with the given shape.
    pub fn query_proximity(&self, shape: &ColliderShape, position: glam::Vec3) -> Vec<Entity> {
        self.backend.query_proximity(shape, position)
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
        self.backend.execute_batched_queries(&batcher)
    }

    // ── Joint API ───────────────────────────────────────────────────────

    /// Create a joint between two entities.
    ///
    /// Returns `None` if either entity does not have a registered rigid body.
    pub fn create_joint(&mut self, desc: JointDescriptor) -> Option<JointHandle> {
        let body_a_handle = *self.backend.body_map.get(&desc.entity_a)?;
        let body_b_handle = *self.backend.body_map.get(&desc.entity_b)?;
        self.backend.create_joint(&desc, body_a_handle, body_b_handle)
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
