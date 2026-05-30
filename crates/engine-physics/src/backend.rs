use std::collections::HashMap;

use crossbeam_channel::Sender;
use rapier3d::na;
use rapier3d::prelude::*;

use crate::components::{BodyType, Collider, ColliderShape, RigidBody};
use crate::convert::{from_rapier_isometry, from_rapier_vec, to_rapier_isometry, to_rapier_vec};
use crate::events::{CollisionEvent, CollisionEventKind};
use crate::Entity;
use crate::Transform;

// Note: all rapier types are imported via rapier3d::prelude::* above.

// ── Raycast result ──────────────────────────────────────────────────────────

/// Result of a raycast query.
#[derive(Debug, Clone)]
pub struct RaycastHit {
    /// The entity that was hit.
    pub entity: Entity,
    /// World-space intersection point.
    pub point: glam::Vec3,
    /// Surface normal at the intersection point.
    pub normal: glam::Vec3,
    /// Distance from the ray origin to the intersection.
    pub distance: f32,
}

// ── Helper: convert ColliderShape to Rapier SharedShape ─────────────────────

pub(crate) fn to_rapier_shared_shape(shape: &ColliderShape) -> SharedShape {
    match *shape {
        ColliderShape::Cuboid { hx, hy, hz } => SharedShape::cuboid(hx, hy, hz),
        ColliderShape::Ball { radius } => SharedShape::ball(radius),
        ColliderShape::Capsule {
            half_height,
            radius,
        } => {
            let a = na::Point3::new(0.0, -half_height, 0.0);
            let b = na::Point3::new(0.0, half_height, 0.0);
            SharedShape::capsule(a, b, radius)
        }
    }
}

// ── Internal event handler ──────────────────────────────────────────────────

/// Internal event data sent over the channel during physics step.
#[derive(Debug, Clone)]
struct RawContactEvent {
    collider1: ColliderHandle,
    collider2: ColliderHandle,
    started: bool,
}

struct BackendEventHandler {
    tx: Sender<RawContactEvent>,
}

impl EventHandler for BackendEventHandler {
    fn handle_collision_event(
        &self,
        _bodies: &RigidBodySet,
        _colliders: &ColliderSet,
        event: rapier3d::geometry::CollisionEvent,
        _contact_pair: Option<&ContactPair>,
    ) {
        let _ = self.tx.send(RawContactEvent {
            collider1: event.collider1(),
            collider2: event.collider2(),
            started: event.started(),
        });
    }

    fn handle_contact_force_event(
        &self,
        _dt: f32,
        _bodies: &RigidBodySet,
        _colliders: &ColliderSet,
        _contact_pair: &ContactPair,
        _total_force_magnitude: f32,
    ) {
        // Not forwarded in this implementation.
    }
}

// ── RapierBackend ───────────────────────────────────────────────────────────

/// Rapier 3D physics backend adapter.
///
/// Owns all Rapier simulation state and maintains maps from entity indices
/// to Rapier handles.  Backend handles are NEVER serialised or exposed in
/// ECS components — they are purely internal to this adapter.
pub struct RapierBackend {
    pub(crate) pipeline: PhysicsPipeline,
    pub(crate) gravity: na::Vector3<f32>,
    pub(crate) integration: IntegrationParameters,
    pub(crate) islands: IslandManager,
    pub(crate) broad_phase: BroadPhaseMultiSap,
    pub(crate) narrow_phase: NarrowPhase,
    pub(crate) bodies: RigidBodySet,
    pub(crate) colliders: ColliderSet,
    pub(crate) impulse_joints: ImpulseJointSet,
    pub(crate) multibody_joints: MultibodyJointSet,
    pub(crate) ccd_solver: CCDSolver,
    pub(crate) query_pipeline: QueryPipeline,

    /// Maps entity index → Rapier rigid body handle.
    pub(crate) body_map: HashMap<u32, RigidBodyHandle>,
    /// Maps entity index → (Rapier collider handle, original shape).
    pub(crate) collider_map: HashMap<u32, (ColliderHandle, ColliderShape)>,
}

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
            body_map: HashMap::new(),
            collider_map: HashMap::new(),
        }
    }

    /// Advance the simulation by one fixed timestep and return collision events.
    pub fn step(&mut self) -> Vec<CollisionEvent> {
        // Drain stale events.
        // (No stale events in our design since we create a new channel each step.)

        let (tx, rx) = crossbeam_channel::unbounded();
        let handler = BackendEventHandler { tx };

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

        // Build reverse collider map for event resolution.
        let mut collider_to_entity = HashMap::new();
        for (&entity_idx, &(ch, _)) in &self.collider_map {
            collider_to_entity.insert(ch, entity_idx);
        }

        let mut events = Vec::new();
        while let Ok(raw) = rx.try_recv() {
            let kind = if raw.started {
                CollisionEventKind::ContactStarted
            } else {
                CollisionEventKind::ContactStopped
            };

            let entity_a = collider_to_entity
                .get(&raw.collider1)
                .copied()
                .map(|i| Entity::new(i, 0));
            let entity_b = collider_to_entity
                .get(&raw.collider2)
                .copied()
                .map(|i| Entity::new(i, 0));

            if let (Some(a), Some(b)) = (entity_a, entity_b) {
                events.push(CollisionEvent {
                    kind,
                    entity_a: a,
                    entity_b: b,
                });
            }
        }

        events
    }

    // ── Body management ─────────────────────────────────────────────────

    /// Create a rigid body for the given entity, returning the body count.
    pub fn create_body(
        &mut self,
        entity_index: u32,
        body: &RigidBody,
        transform: &Transform,
    ) -> usize {
        if self.body_map.contains_key(&entity_index) {
            return self.bodies.len();
        }

        let iso = to_rapier_isometry(transform.translation, transform.rotation);

        let builder = match body.body_type {
            BodyType::Static => RigidBodyBuilder::fixed(),
            BodyType::Dynamic => RigidBodyBuilder::dynamic(),
            BodyType::Kinematic => RigidBodyBuilder::kinematic_position_based(),
        };

        let rapier_body = builder
            .position(iso)
            .linear_damping(body.linear_damping)
            .angular_damping(body.angular_damping)
            .gravity_scale(body.gravity_scale)
            .can_sleep(body.can_sleep)
            .enabled(body.enabled)
            .build();

        let handle = self.bodies.insert(rapier_body);
        self.body_map.insert(entity_index, handle);
        self.bodies.len()
    }

    /// Create a collider and attach it to the body of the given entity.
    pub fn create_collider(
        &mut self,
        entity_index: u32,
        collider: &Collider,
        body_entity: u32,
        material: Option<&crate::PhysicsMaterial>,
    ) {
        if self.collider_map.contains_key(&entity_index) {
            return;
        }

        let body_handle = match self.body_map.get(&body_entity) {
            Some(&h) => h,
            None => return,
        };

        let shape = to_rapier_shared_shape(&collider.shape);

        let density = material.map_or(collider.density, |m| m.density);
        let friction = material.map_or(collider.friction, |m| m.friction);
        let restitution = material.map_or(collider.restitution, |m| m.restitution);

        let groups = InteractionGroups::new(
            Group::from_bits_truncate(collider.collision_group),
            Group::from_bits_truncate(collider.collision_mask),
        );

        let rapier_collider = ColliderBuilder::new(shape)
            .density(density)
            .friction(friction)
            .restitution(restitution)
            .sensor(collider.is_trigger)
            .collision_groups(groups)
            .build();

        let collider_handle =
            self.colliders
                .insert_with_parent(rapier_collider, body_handle, &mut self.bodies);

        self.collider_map
            .insert(entity_index, (collider_handle, collider.shape.clone()));
    }

    /// Remove a rigid body and all its colliders.
    pub fn remove_body(&mut self, entity_index: u32) {
        if let Some(handle) = self.body_map.remove(&entity_index) {
            self.bodies.remove(
                handle,
                &mut self.islands,
                &mut self.colliders,
                &mut self.impulse_joints,
                &mut self.multibody_joints,
                true,
            );
        }
        self.collider_map.remove(&entity_index);
    }

    /// Remove a collider but keep the body.
    pub fn remove_collider(&mut self, entity_index: u32) {
        if let Some((handle, _)) = self.collider_map.remove(&entity_index) {
            self.colliders
                .remove(handle, &mut self.islands, &mut self.bodies, true);
        }
    }

    // ── Transform synchronisation ───────────────────────────────────────

    /// Read back the world-space transform of a body.
    pub fn sync_body_transform(&self, entity_index: u32) -> Option<(glam::Vec3, glam::Quat)> {
        let handle = self.body_map.get(&entity_index)?;
        let body = self.bodies.get(*handle)?;
        Some(from_rapier_isometry(body.position()))
    }

    /// Set the world-space transform of a body.
    pub fn set_body_transform(&mut self, entity_index: u32, pos: glam::Vec3, rot: glam::Quat) {
        if let Some(&handle) = self.body_map.get(&entity_index) {
            if let Some(body) = self.bodies.get_mut(handle) {
                body.set_position(to_rapier_isometry(pos, rot), true);
            }
        }
    }

    // ── Force / impulse ─────────────────────────────────────────────────

    /// Apply a force at the centre of mass.
    pub fn apply_force(&mut self, entity_index: u32, force: glam::Vec3) {
        if let Some(&handle) = self.body_map.get(&entity_index) {
            if let Some(body) = self.bodies.get_mut(handle) {
                body.add_force(to_rapier_vec(force), true);
            }
        }
    }

    /// Apply an impulse at the centre of mass.
    pub fn apply_impulse(&mut self, entity_index: u32, impulse: glam::Vec3) {
        if let Some(&handle) = self.body_map.get(&entity_index) {
            if let Some(body) = self.bodies.get_mut(handle) {
                body.apply_impulse(to_rapier_vec(impulse), true);
            }
        }
    }

    // ── Queries ─────────────────────────────────────────────────────────

    /// Cast a ray against all colliders and return the closest hit.
    pub fn raycast(
        &self,
        origin: glam::Vec3,
        dir: glam::Vec3,
        max_dist: f32,
    ) -> Option<RaycastHit> {
        let ray = Ray::new(
            na::Point3::new(origin.x, origin.y, origin.z),
            na::Vector3::new(dir.x, dir.y, dir.z),
        );
        let filter = QueryFilter::default().exclude_sensors();

        let (collider_handle, intersection) = self.query_pipeline.cast_ray_and_get_normal(
            &self.bodies,
            &self.colliders,
            &ray,
            max_dist,
            true,
            filter,
        )?;

        let collider = self.colliders.get(collider_handle)?;
        let body_handle = collider.parent()?;

        let entity_index = self
            .body_map
            .iter()
            .find(|(_, &h)| h == body_handle)
            .map(|(&idx, _)| idx)?;

        Some(RaycastHit {
            entity: Entity::new(entity_index, 0),
            point: origin + dir * intersection.time_of_impact,
            normal: from_rapier_vec(intersection.normal),
            distance: intersection.time_of_impact,
        })
    }

    /// Find all entities whose colliders overlap with the given shape.
    pub fn query_proximity(&self, shape: &ColliderShape, pos: glam::Vec3) -> Vec<Entity> {
        let rapier_shape = to_rapier_shared_shape(shape);
        let iso = Isometry::from_parts(
            na::Translation3::new(pos.x, pos.y, pos.z),
            na::UnitQuaternion::identity(),
        );
        let filter = QueryFilter::default().exclude_sensors();
        let mut entities = Vec::new();

        self.query_pipeline.intersections_with_shape(
            &self.bodies,
            &self.colliders,
            &iso,
            &*rapier_shape,
            filter,
            |collider_handle| {
                if let Some(collider) = self.colliders.get(collider_handle) {
                    if let Some(parent) = collider.parent() {
                        if let Some(entity_index) = self
                            .body_map
                            .iter()
                            .find(|(_, &h)| h == parent)
                            .map(|(&i, _)| i)
                        {
                            entities.push(Entity::new(entity_index, 0));
                        }
                    }
                }
                true
            },
        );

        entities
    }

    /// Update the query pipeline after structural changes.
    pub fn sync_query_pipeline(&mut self) {
        self.query_pipeline.update(&self.colliders);
    }

    /// Check whether an entity has a body registered.
    pub fn has_body(&self, entity_index: u32) -> bool {
        self.body_map.contains_key(&entity_index)
    }

    /// Check whether an entity has a collider registered.
    pub fn has_collider(&self, entity_index: u32) -> bool {
        self.collider_map.contains_key(&entity_index)
    }

    /// Return a reference to the bodies set.
    pub fn bodies(&self) -> &RigidBodySet {
        &self.bodies
    }

    /// Return a reference to the colliders set.
    pub fn colliders(&self) -> &ColliderSet {
        &self.colliders
    }

    /// Return an iterator over all (entity_index, body_handle) pairs.
    pub fn body_entries(&self) -> impl Iterator<Item = (u32, RigidBodyHandle)> + '_ {
        self.body_map.iter().map(|(&k, &v)| (k, v))
    }

    /// Collect debug info for all colliders.
    pub fn collider_debug_info(&self) -> Vec<(ColliderShape, glam::Vec3, glam::Quat)> {
        let mut info = Vec::new();
        for (&_entity_idx, &(collider_handle, ref shape)) in &self.collider_map {
            let collider = match self.colliders.get(collider_handle) {
                Some(c) => c,
                None => continue,
            };
            let body_handle = match collider.parent() {
                Some(h) => h,
                None => continue,
            };
            let body = match self.bodies.get(body_handle) {
                Some(b) => b,
                None => continue,
            };
            let pos = body.position();
            let translation = glam::Vec3::new(
                pos.translation.vector.x,
                pos.translation.vector.y,
                pos.translation.vector.z,
            );
            let quat = pos.rotation.quaternion();
            let rotation = glam::Quat::from_xyzw(quat.i, quat.j, quat.k, quat.w);
            info.push((shape.clone(), translation, rotation));
        }
        info
    }
}
