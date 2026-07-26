use std::collections::{HashMap, HashSet};

use crossbeam_channel::Sender;
use rapier3d::na;
use rapier3d::prelude::*;

use crate::components::{BodyType, Collider, ColliderShape, RigidBody};
use crate::convert::{from_rapier_isometry, from_rapier_vec, to_rapier_isometry, to_rapier_vec};
use crate::events::{
    CollisionEvent, CollisionEventKind, JointBreakEvent, PhysicsEvents, TriggerEvent,
    TriggerEventKind,
};
use crate::joints::{JointDescriptor, JointHandle, JointType};
use crate::queries::{
    OverlapHitResult, PhysicsQueryFilter, QueryBatcher, QueryResults, RaycastHitResult,
    SweepHitResult,
};
use crate::Entity;
use crate::RigidBodyRuntimeState;
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

pub(crate) fn to_rapier_shared_shape(shape: &ColliderShape) -> Option<SharedShape> {
    match shape {
        ColliderShape::Cuboid { hx, hy, hz }
            if [hx, hy, hz]
                .into_iter()
                .all(|value| value.is_finite() && *value > 0.0) =>
        {
            Some(SharedShape::cuboid(*hx, *hy, *hz))
        }
        ColliderShape::Ball { radius } if radius.is_finite() && *radius > 0.0 => {
            Some(SharedShape::ball(*radius))
        }
        ColliderShape::Capsule {
            half_height,
            radius,
        } if half_height.is_finite()
            && *half_height >= 0.0
            && radius.is_finite()
            && *radius > 0.0 =>
        {
            let a = na::Point3::new(0.0, -*half_height, 0.0);
            let b = na::Point3::new(0.0, *half_height, 0.0);
            Some(SharedShape::capsule(a, b, *radius))
        }
        ColliderShape::HeightField {
            rows,
            columns,
            heights,
            scale,
        } => {
            let valid = *rows >= 2
                && *columns >= 2
                && heights.len() == *rows as usize * *columns as usize
                && heights.iter().all(|height| height.is_finite())
                && scale.iter().all(|value| value.is_finite() && *value > 0.0);
            if !valid {
                // Collider data can arrive through generic script writes. A
                // malformed payload must not reach Rapier's assert-heavy
                // constructor or turn into a phantom fallback collider.
                return None;
            }
            let matrix = na::DMatrix::from_row_slice(*rows as usize, *columns as usize, heights);
            Some(SharedShape::heightfield(
                matrix,
                na::Vector3::new(scale[0], scale[1], scale[2]),
            ))
        }
        _ => None,
    }
}

type EntityPair = (Entity, Entity);

fn canonical_entity_pair(a: Entity, b: Entity) -> EntityPair {
    let a_key = (a.index(), a.generation());
    let b_key = (b.index(), b.generation());
    if a_key <= b_key {
        (a, b)
    } else {
        (b, a)
    }
}

// ── Internal event handler ──────────────────────────────────────────────────

/// Internal data for a collision (non-trigger) event.
#[derive(Debug, Clone)]
struct RawContactEvent {
    collider1: ColliderHandle,
    collider2: ColliderHandle,
    started: bool,
}

/// Internal data for a trigger / sensor intersection event.
#[derive(Debug, Clone)]
struct RawIntersectionEvent {
    collider1: ColliderHandle,
    collider2: ColliderHandle,
    intersecting: bool,
}

struct BackendEventHandler {
    tx_col: Sender<RawContactEvent>,
    tx_int: Sender<RawIntersectionEvent>,
}

impl EventHandler for BackendEventHandler {
    fn handle_collision_event(
        &self,
        _bodies: &RigidBodySet,
        colliders: &ColliderSet,
        event: rapier3d::geometry::CollisionEvent,
        _contact_pair: Option<&ContactPair>,
    ) {
        let c1 = event.collider1();
        let c2 = event.collider2();

        // Route sensor (trigger) events through the intersection channel;
        // regular collisions go through the collision channel.
        let is_sensor = colliders.get(c1).map(|c| c.is_sensor()).unwrap_or(false)
            || colliders.get(c2).map(|c| c.is_sensor()).unwrap_or(false);

        if is_sensor {
            let _ = self.tx_int.send(RawIntersectionEvent {
                collider1: c1,
                collider2: c2,
                intersecting: event.started(),
            });
        } else {
            let _ = self.tx_col.send(RawContactEvent {
                collider1: c1,
                collider2: c2,
                started: event.started(),
            });
        }
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
/// Owns all Rapier simulation state and maintains maps from complete entity
/// handles (index plus generation) to Rapier handles. Backend handles are
/// NEVER serialised or exposed in
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
    query_pipeline_dirty: bool,

    /// Maps complete entity handles to Rapier rigid body handles.
    pub(crate) body_map: HashMap<Entity, RigidBodyHandle>,
    /// Maps complete entity handles to Rapier collider handles and shapes.
    pub(crate) collider_map: HashMap<Entity, (ColliderHandle, ColliderShape)>,

    /// Maps complete entity handles to all attached joint handles.
    pub(crate) joint_entity_map: HashMap<Entity, HashSet<u32>>,
    /// Maps our JointHandle.0 → full Rapier ImpulseJointHandle (with generation).
    pub(crate) joint_handle_lookup: HashMap<u32, ImpulseJointHandle>,
    /// Authored break thresholds keyed by engine joint handle.
    joint_break_limits: HashMap<u32, (f32, f32)>,
    /// Complete body entities for break-event reporting.
    joint_bodies: HashMap<u32, (Entity, Entity)>,
    /// Auto-incrementing counter for JointHandle IDs.
    next_joint_id: u32,

    /// Active sensor (trigger) overlaps from the previous frame.
    /// Used to derive Entered vs Stay [`TriggerEvent`]s.
    /// Keys are canonical pairs of complete entity handles.
    active_sensor_overlaps: HashSet<EntityPair>,

    /// Active non‑sensor collision pairs from the previous frame.
    /// Used to derive [`CollisionEventKind::ContactStaying`].
    /// Keys are canonical pairs of complete entity handles.
    active_collision_overlaps: HashSet<EntityPair>,
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

    // ── Body management ─────────────────────────────────────────────────

    /// Create a rigid body for the given entity, returning the body count.
    pub fn create_body(
        &mut self,
        entity: Entity,
        body: &RigidBody,
        transform: &Transform,
    ) -> usize {
        if self.body_map.contains_key(&entity) {
            return self.bodies.len();
        }

        // This low-level public API has no World from which it can prove which
        // generation is current. Reject a conflicting slot instead of letting
        // a delayed stale call replace live state (including across wrapping).
        if self
            .body_map
            .keys()
            .any(|stored| stored.index() == entity.index())
        {
            return self.bodies.len();
        }

        self.insert_body(entity, body, transform)
    }

    /// Replace any previous generation in this slot with a World-validated
    /// current entity.
    pub(crate) fn replace_body_for_current_entity(
        &mut self,
        entity: Entity,
        body: &RigidBody,
        transform: &Transform,
    ) -> usize {
        if self.body_map.contains_key(&entity) {
            return self.bodies.len();
        }

        let stale_entities: Vec<Entity> = self
            .body_map
            .keys()
            .copied()
            .filter(|stored| stored.index() == entity.index())
            .collect();
        for stale in stale_entities {
            self.remove_body(stale);
        }

        self.insert_body(entity, body, transform)
    }

    fn insert_body(&mut self, entity: Entity, body: &RigidBody, transform: &Transform) -> usize {
        let iso = to_rapier_isometry(transform.translation, transform.rotation);

        let builder = match body.body_type {
            BodyType::Static => RigidBodyBuilder::fixed(),
            BodyType::Dynamic => RigidBodyBuilder::dynamic(),
            BodyType::Kinematic => RigidBodyBuilder::kinematic_position_based(),
        };

        let rapier_body = builder
            .position(iso)
            .additional_mass(body.mass)
            .linear_damping(body.linear_damping)
            .angular_damping(body.angular_damping)
            .gravity_scale(body.gravity_scale)
            .can_sleep(body.can_sleep)
            .enabled(body.enabled)
            .ccd_enabled(body.ccd_enabled)
            .build();

        let handle = self.bodies.insert(rapier_body);
        self.body_map.insert(entity, handle);
        self.bodies.len()
    }

    /// Create a collider and attach it to the body of the given entity.
    pub fn create_collider(
        &mut self,
        entity: Entity,
        collider: &Collider,
        body_entity: Entity,
        material: Option<&crate::PhysicsMaterial>,
    ) {
        if self.collider_map.contains_key(&entity) {
            return;
        }

        // As with create_body, only an exact free slot may be populated through
        // the public API. Replacement requires World validation.
        if self
            .collider_map
            .keys()
            .any(|stored| stored.index() == entity.index())
        {
            return;
        }

        self.insert_collider(entity, collider, body_entity, material);
    }

    /// Replace any previous generation in this slot with a World-validated
    /// current entity collider.
    pub(crate) fn replace_collider_for_current_entity(
        &mut self,
        entity: Entity,
        collider: &Collider,
        body_entity: Entity,
        material: Option<&crate::PhysicsMaterial>,
    ) {
        if self.collider_map.contains_key(&entity) {
            return;
        }

        let stale_entities: Vec<Entity> = self
            .collider_map
            .keys()
            .copied()
            .filter(|stored| stored.index() == entity.index())
            .collect();
        for stale in stale_entities {
            self.remove_collider(stale);
        }

        self.insert_collider(entity, collider, body_entity, material);
    }

    fn insert_collider(
        &mut self,
        entity: Entity,
        collider: &Collider,
        body_entity: Entity,
        material: Option<&crate::PhysicsMaterial>,
    ) {
        let body_handle = match self.body_map.get(&body_entity) {
            Some(&h) => h,
            None => return,
        };

        let Some(shape) = to_rapier_shared_shape(&collider.shape) else {
            tracing::warn!(entity = ?entity, "ignored invalid collider shape");
            return;
        };

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
            .active_events(ActiveEvents::COLLISION_EVENTS)
            .build();

        let collider_handle =
            self.colliders
                .insert_with_parent(rapier_collider, body_handle, &mut self.bodies);

        self.collider_map
            .insert(entity, (collider_handle, collider.shape.clone()));
        self.query_pipeline_dirty = true;
    }

    /// Remove a rigid body and all its colliders.
    pub fn remove_body(&mut self, entity: Entity) {
        if let Some(handle) = self.body_map.remove(&entity) {
            let attached_colliders: Vec<Entity> = self
                .collider_map
                .iter()
                .filter_map(|(&collider_entity, &(collider_handle, _))| {
                    self.colliders.get(collider_handle).and_then(|collider| {
                        (collider.parent() == Some(handle)).then_some(collider_entity)
                    })
                })
                .collect();
            let removed_colliders = !attached_colliders.is_empty();
            self.bodies.remove(
                handle,
                &mut self.islands,
                &mut self.colliders,
                &mut self.impulse_joints,
                &mut self.multibody_joints,
                true,
            );
            for collider_entity in attached_colliders {
                self.collider_map.remove(&collider_entity);
                self.clear_entity_overlaps(collider_entity);
            }
            if removed_colliders {
                self.query_pipeline_dirty = true;
            }
        }

        self.clear_entity_overlaps(entity);

        // Clean up joint tracking: Rapier's bodies.remove() already removed
        // any joints attached to this body from impulse_joints, so we clean
        // up our own tracking maps to prevent stale handle entries.
        self.joint_entity_map.remove(&entity);
        self.joint_handle_lookup
            .retain(|_, rapier_handle| self.impulse_joints.get(*rapier_handle).is_some());
        let valid_joint_ids: HashSet<u32> = self.joint_handle_lookup.keys().copied().collect();
        self.joint_break_limits
            .retain(|joint_id, _| valid_joint_ids.contains(joint_id));
        self.joint_bodies
            .retain(|joint_id, _| valid_joint_ids.contains(joint_id));
        self.joint_entity_map.retain(|_, joint_ids| {
            joint_ids.retain(|joint_id| valid_joint_ids.contains(joint_id));
            !joint_ids.is_empty()
        });
    }

    fn clear_entity_overlaps(&mut self, entity: Entity) {
        self.active_sensor_overlaps
            .retain(|pair| pair.0 != entity && pair.1 != entity);
        self.active_collision_overlaps
            .retain(|pair| pair.0 != entity && pair.1 != entity);
    }

    // ── Joint management ────────────────────────────────────────────────

    /// Create a joint between two rigid bodies.
    pub fn create_joint(
        &mut self,
        desc: &JointDescriptor,
        body_a_handle: RigidBodyHandle,
        body_b_handle: RigidBodyHandle,
    ) -> Option<JointHandle> {
        desc.validate().ok()?;
        let frame_a = na::Isometry3::from_parts(
            na::Translation3::new(desc.anchor_a[0], desc.anchor_a[1], desc.anchor_a[2]),
            na::UnitQuaternion::identity(),
        );
        let frame_b = na::Isometry3::from_parts(
            na::Translation3::new(desc.anchor_b[0], desc.anchor_b[1], desc.anchor_b[2]),
            na::UnitQuaternion::identity(),
        );
        let anchor_a = na::Point3::new(desc.anchor_a[0], desc.anchor_a[1], desc.anchor_a[2]);
        let anchor_b = na::Point3::new(desc.anchor_b[0], desc.anchor_b[1], desc.anchor_b[2]);

        // Build the appropriate Rapier joint type and insert directly.
        // NOTE: Rapier 0.22 builders implement Into<GenericJoint> so we pass
        // them straight to impulse_joints.insert().
        let rapier_handle = match desc.joint_type {
            JointType::Fixed => {
                let b = FixedJointBuilder::new()
                    .local_frame1(frame_a)
                    .local_frame2(frame_b);
                self.impulse_joints
                    .insert(body_a_handle, body_b_handle, b, true)
            }
            JointType::Revolute => {
                let axis = na::Unit::new_normalize(na::Vector3::new(
                    desc.axis[0],
                    desc.axis[1],
                    desc.axis[2],
                ));
                let mut b = RevoluteJointBuilder::new(axis)
                    .local_anchor1(anchor_a)
                    .local_anchor2(anchor_b);
                if let Some(l) = &desc.limits {
                    b = b.limits([l.min, l.max]);
                }
                if let Some(m) = &desc.motor {
                    b = b.motor(m.target_pos, m.target_vel, m.stiffness, m.damping);
                }
                self.impulse_joints
                    .insert(body_a_handle, body_b_handle, b, true)
            }
            JointType::Prismatic => {
                let axis = na::Unit::new_normalize(na::Vector3::new(
                    desc.axis[0],
                    desc.axis[1],
                    desc.axis[2],
                ));
                let mut b = PrismaticJointBuilder::new(axis)
                    .local_anchor1(anchor_a)
                    .local_anchor2(anchor_b);
                if let Some(l) = &desc.limits {
                    b = b.limits([l.min, l.max]);
                }
                if let Some(m) = &desc.motor {
                    b = b.set_motor(m.target_pos, m.target_vel, m.stiffness, m.damping);
                }
                self.impulse_joints
                    .insert(body_a_handle, body_b_handle, b, true)
            }
            JointType::Spherical => {
                let mut b = SphericalJointBuilder::new()
                    .local_frame1(frame_a)
                    .local_frame2(frame_b);
                if let Some(l) = &desc.limits {
                    use rapier3d::dynamics::JointAxis;
                    b = b
                        .limits(JointAxis::AngX, [l.min, l.max])
                        .limits(JointAxis::AngY, [l.min, l.max])
                        .limits(JointAxis::AngZ, [l.min, l.max]);
                }
                if let Some(m) = &desc.motor {
                    use rapier3d::dynamics::JointAxis;
                    b = b
                        .motor(
                            JointAxis::AngX,
                            m.target_pos,
                            m.target_vel,
                            m.stiffness,
                            m.damping,
                        )
                        .motor(
                            JointAxis::AngY,
                            m.target_pos,
                            m.target_vel,
                            m.stiffness,
                            m.damping,
                        )
                        .motor(
                            JointAxis::AngZ,
                            m.target_pos,
                            m.target_vel,
                            m.stiffness,
                            m.damping,
                        );
                }
                self.impulse_joints
                    .insert(body_a_handle, body_b_handle, b, true)
            }
        };

        // Generate a unique handle ID and store the full Rapier handle.
        let our_id = self.next_joint_id;
        self.next_joint_id += 1;
        self.joint_handle_lookup.insert(our_id, rapier_handle);
        self.joint_break_limits
            .insert(our_id, (desc.break_force, desc.break_torque));
        self.joint_bodies
            .insert(our_id, (desc.entity_a, desc.entity_b));

        // Track entity → joint mapping.
        self.joint_entity_map
            .entry(desc.entity_a)
            .or_default()
            .insert(our_id);
        self.joint_entity_map
            .entry(desc.entity_b)
            .or_default()
            .insert(our_id);

        Some(JointHandle(our_id))
    }

    /// Remove a joint by handle.
    pub fn remove_joint(&mut self, handle: JointHandle) {
        if let Some(rapier_handle) = self.joint_handle_lookup.remove(&handle.0) {
            self.impulse_joints.remove(rapier_handle, true);
        }
        self.joint_break_limits.remove(&handle.0);
        self.joint_bodies.remove(&handle.0);
        // Clean up entity tracking.
        self.joint_entity_map.retain(|_, handles| {
            handles.remove(&handle.0);
            !handles.is_empty()
        });
    }

    /// Number of active impulse joints.
    pub fn joint_count(&self) -> usize {
        self.impulse_joints.len()
    }

    pub fn has_joint(&self, handle: JointHandle) -> bool {
        self.joint_handle_lookup
            .get(&handle.0)
            .is_some_and(|rapier_handle| self.impulse_joints.get(*rapier_handle).is_some())
    }

    /// Remove joints whose solver impulse exceeded an authored force/torque
    /// threshold during the completed fixed step.
    ///
    /// Rapier exposes constraint impulses (N·s and N·m·s). Dividing their
    /// linear/angular norms by Rapier's final TGS substep duration yields the
    /// reaction force/torque estimate used for deterministic break decisions.
    fn break_overloaded_joints(&mut self) -> Vec<JointBreakEvent> {
        let substep_dt = self.integration.dt / self.integration.num_solver_iterations.get() as f32;
        if !substep_dt.is_finite() || substep_dt <= 0.0 {
            return Vec::new();
        }
        let mut broken = Vec::new();
        for (&joint_id, &rapier_handle) in &self.joint_handle_lookup {
            let Some(&(break_force, break_torque)) = self.joint_break_limits.get(&joint_id) else {
                continue;
            };
            if break_force <= 0.0 && break_torque <= 0.0 {
                continue;
            }
            let Some(joint) = self.impulse_joints.get(rapier_handle) else {
                continue;
            };
            let linear_impulse =
                na::Vector3::new(joint.impulses[0], joint.impulses[1], joint.impulses[2]);
            let angular_impulse =
                na::Vector3::new(joint.impulses[3], joint.impulses[4], joint.impulses[5]);
            let force = linear_impulse.norm() / substep_dt;
            let torque = angular_impulse.norm() / substep_dt;
            let force_broke = break_force > 0.0 && (!force.is_finite() || force > break_force);
            let torque_broke = break_torque > 0.0 && (!torque.is_finite() || torque > break_torque);
            if force_broke || torque_broke {
                let Some(&(entity_a, entity_b)) = self.joint_bodies.get(&joint_id) else {
                    continue;
                };
                broken.push(JointBreakEvent {
                    handle: JointHandle(joint_id),
                    joint_entity: None,
                    entity_a,
                    entity_b,
                    force,
                    torque,
                });
            }
        }
        for event in &broken {
            self.remove_joint(event.handle);
        }
        broken
    }

    /// Remove a collider but keep the body.
    pub fn remove_collider(&mut self, entity: Entity) {
        if let Some((handle, _)) = self.collider_map.remove(&entity) {
            self.colliders
                .remove(handle, &mut self.islands, &mut self.bodies, true);
            self.query_pipeline_dirty = true;
        }
        self.clear_entity_overlaps(entity);
    }

    // ── Transform synchronisation ───────────────────────────────────────

    /// Read back the world-space transform of a body.
    pub fn sync_body_transform(&self, entity: Entity) -> Option<(glam::Vec3, glam::Quat)> {
        let handle = self.body_map.get(&entity)?;
        let body = self.bodies.get(*handle)?;
        Some(from_rapier_isometry(body.position()))
    }

    pub(crate) fn runtime_body_states(&self) -> Vec<(Entity, RigidBodyRuntimeState)> {
        self.body_map
            .iter()
            .filter_map(|(&entity, &handle)| {
                let body = self.bodies.get(handle)?;
                let (position, rotation) = from_rapier_isometry(body.position());
                Some((
                    entity,
                    RigidBodyRuntimeState {
                        position: position.to_array(),
                        rotation: rotation.to_array(),
                        linear_velocity: [body.linvel().x, body.linvel().y, body.linvel().z],
                        angular_velocity: [body.angvel().x, body.angvel().y, body.angvel().z],
                        sleeping: body.is_sleeping(),
                    },
                ))
            })
            .collect()
    }

    pub(crate) fn restore_runtime_body_state(
        &mut self,
        entity: Entity,
        state: &RigidBodyRuntimeState,
    ) -> bool {
        let Some(&handle) = self.body_map.get(&entity) else {
            return false;
        };
        let Some(body) = self.bodies.get_mut(handle) else {
            return false;
        };
        let rotation = glam::Quat::from_array(state.rotation).normalize();
        body.set_position(
            to_rapier_isometry(glam::Vec3::from_array(state.position), rotation),
            false,
        );
        body.set_linvel(
            to_rapier_vec(glam::Vec3::from_array(state.linear_velocity)),
            false,
        );
        body.set_angvel(
            to_rapier_vec(glam::Vec3::from_array(state.angular_velocity)),
            false,
        );
        if state.sleeping {
            body.sleep();
        } else {
            body.wake_up(true);
        }
        self.query_pipeline_dirty = true;
        true
    }

    pub(crate) fn set_linear_velocity(&mut self, entity: Entity, velocity: glam::Vec3) {
        let Some(&handle) = self.body_map.get(&entity) else {
            return;
        };
        if let Some(body) = self.bodies.get_mut(handle) {
            body.set_linvel(to_rapier_vec(velocity), true);
        }
    }

    pub(crate) fn set_angular_velocity(&mut self, entity: Entity, velocity: glam::Vec3) {
        let Some(&handle) = self.body_map.get(&entity) else {
            return;
        };
        if let Some(body) = self.bodies.get_mut(handle) {
            body.set_angvel(to_rapier_vec(velocity), true);
        }
    }

    pub(crate) fn set_body_type(&mut self, entity: Entity, body_type: BodyType) {
        let Some(&handle) = self.body_map.get(&entity) else {
            return;
        };
        let Some(body) = self.bodies.get_mut(handle) else {
            return;
        };
        let body_type = match body_type {
            BodyType::Static => RigidBodyType::Fixed,
            BodyType::Dynamic => RigidBodyType::Dynamic,
            BodyType::Kinematic => RigidBodyType::KinematicPositionBased,
        };
        body.set_body_type(body_type, true);
        self.query_pipeline_dirty = true;
    }

    /// Set the world-space transform of a body.
    pub fn set_body_transform(&mut self, entity: Entity, pos: glam::Vec3, rot: glam::Quat) {
        if let Some(&handle) = self.body_map.get(&entity) {
            if let Some(body) = self.bodies.get_mut(handle) {
                body.set_position(to_rapier_isometry(pos, rot), true);
            }
        }
    }

    /// Translate a body by `offset` without disturbing its simulation state.
    ///
    /// Used by world-origin shifts: the body's rotation, linear/angular
    /// velocities, forces, and sleep state are all preserved — only its
    /// position moves. `wake_up = false` keeps sleeping bodies asleep and
    /// leaves awake bodies awake. Returns `false` when no body is registered
    /// for `entity`.
    pub fn translate_body(&mut self, entity: Entity, offset: glam::Vec3) -> bool {
        let Some(&handle) = self.body_map.get(&entity) else {
            return false;
        };
        let Some(body) = self.bodies.get_mut(handle) else {
            return false;
        };
        let mut position = *body.position();
        position.translation.vector += to_rapier_vec(offset);
        body.set_position(position, false);
        self.query_pipeline_dirty = true;
        true
    }

    /// Push modified body positions into their attached colliders.
    ///
    /// `RigidBody::set_position` only updates the body; collider poses (and
    /// therefore the query pipeline) observe the move after this
    /// propagation, which the physics step normally performs.
    pub(crate) fn propagate_body_positions_to_colliders(&mut self) {
        self.bodies
            .propagate_modified_body_positions_to_colliders(&mut self.colliders);
    }

    // ── Force / impulse ─────────────────────────────────────────────────

    /// Set the gravity scale of a registered body without waking it.
    ///
    /// Used by the gravity-source system to zero the global-gravity
    /// multiplier on source-driven bodies and to restore the ECS-authored
    /// scale when a body leaves every source's range.
    pub(crate) fn set_body_gravity_scale(&mut self, entity: Entity, scale: f32) {
        if let Some(&handle) = self.body_map.get(&entity) {
            if let Some(body) = self.bodies.get_mut(handle) {
                body.set_gravity_scale(scale, false);
            }
        }
    }

    /// Total mass of a registered body (additional mass plus collider mass).
    pub(crate) fn body_mass(&self, entity: Entity) -> Option<f32> {
        let handle = self.body_map.get(&entity)?;
        Some(self.bodies.get(*handle)?.mass())
    }

    /// Apply a force at the centre of mass.
    pub fn apply_force(&mut self, entity: Entity, force: glam::Vec3) {
        if let Some(&handle) = self.body_map.get(&entity) {
            if let Some(body) = self.bodies.get_mut(handle) {
                body.add_force(to_rapier_vec(force), true);
            }
        }
    }

    /// Apply an impulse at the centre of mass.
    pub fn apply_impulse(&mut self, entity: Entity, impulse: glam::Vec3) {
        if let Some(&handle) = self.body_map.get(&entity) {
            if let Some(body) = self.bodies.get_mut(handle) {
                body.apply_impulse(to_rapier_vec(impulse), true);
            }
        }
    }

    /// Apply a torque to a rigid body.
    pub fn apply_torque(&mut self, entity: Entity, torque: glam::Vec3) {
        if let Some(&handle) = self.body_map.get(&entity) {
            if let Some(body) = self.bodies.get_mut(handle) {
                body.add_torque(to_rapier_vec(torque), true);
            }
        }
    }

    /// Apply an instantaneous angular impulse to a rigid body.
    pub fn apply_torque_impulse(&mut self, entity: Entity, torque_impulse: glam::Vec3) {
        if let Some(&handle) = self.body_map.get(&entity) {
            if let Some(body) = self.bodies.get_mut(handle) {
                body.apply_torque_impulse(to_rapier_vec(torque_impulse), true);
            }
        }
    }

    // ── Queries ─────────────────────────────────────────────────────────

    /// Build the rapier-side [`QueryFilter`] for a [`PhysicsQueryFilter`].
    ///
    /// Layer filtering maps the script-visible mask onto the collider's
    /// `collision_group` membership bits: a collider is a candidate iff
    /// `(collision_group & layer_mask) != 0`. Colliders whose own
    /// `collision_mask` is zero never collide with anything — including
    /// queries — so they are never candidates either.
    fn rapier_query_filter(&self, filter: &PhysicsQueryFilter) -> QueryFilter<'static> {
        let mut rapier_filter = if filter.include_sensors {
            QueryFilter::default()
        } else {
            QueryFilter::default().exclude_sensors()
        };
        if let Some(layer_mask) = filter.layer_mask {
            rapier_filter.groups = Some(InteractionGroups::new(
                Group::ALL,
                Group::from_bits_truncate(layer_mask),
            ));
        }
        if let Some(excluded) = filter.exclude_entity {
            if let Some(&(handle, _)) = self.collider_map.get(&excluded) {
                rapier_filter.exclude_collider = Some(handle);
            }
        }
        rapier_filter
    }

    /// Cast a ray against all colliders and return the closest hit.
    pub fn raycast(
        &self,
        origin: glam::Vec3,
        dir: glam::Vec3,
        max_dist: f32,
    ) -> Option<RaycastHit> {
        self.raycast_filtered(origin, dir, max_dist, &PhysicsQueryFilter::default())
    }

    /// Cast a ray with a candidate filter and return the closest hit.
    pub fn raycast_filtered(
        &self,
        origin: glam::Vec3,
        dir: glam::Vec3,
        max_dist: f32,
        filter: &PhysicsQueryFilter,
    ) -> Option<RaycastHit> {
        let ray = Ray::new(
            na::Point3::new(origin.x, origin.y, origin.z),
            na::Vector3::new(dir.x, dir.y, dir.z),
        );
        let filter = self.rapier_query_filter(filter);

        let (collider_handle, intersection) = self.query_pipeline.cast_ray_and_get_normal(
            &self.bodies,
            &self.colliders,
            &ray,
            max_dist,
            true,
            filter,
        )?;

        let entity = self
            .collider_map
            .iter()
            .find(|(_, &(handle, _))| handle == collider_handle)
            .map(|(&entity, _)| entity)?;

        Some(RaycastHit {
            entity,
            point: origin + dir * intersection.time_of_impact,
            normal: from_rapier_vec(intersection.normal),
            distance: intersection.time_of_impact,
        })
    }

    /// Sweep a shape along a direction and return the closest hit.
    ///
    /// The returned [`RaycastHit`] reuses the raycast payload: `point` is the
    /// world-space contact point on the hit collider, `normal` its outward
    /// surface normal, and `distance` the travel distance of the sweep.
    /// When the shape starts in penetration the hit reports a zero distance
    /// and the contact geometry is only a conservative approximation.
    pub fn cast_shape_filtered(
        &self,
        shape: &ColliderShape,
        origin: glam::Vec3,
        dir: glam::Vec3,
        max_dist: f32,
        filter: &PhysicsQueryFilter,
    ) -> Option<RaycastHit> {
        use rapier3d::parry::query::ShapeCastOptions;

        let rapier_shape = to_rapier_shared_shape(shape)?;
        let iso = Isometry::from_parts(
            na::Translation3::new(origin.x, origin.y, origin.z),
            na::UnitQuaternion::identity(),
        );
        let rapier_dir = na::Vector3::new(dir.x, dir.y, dir.z);
        let options = ShapeCastOptions {
            max_time_of_impact: max_dist,
            target_distance: 0.0,
            stop_at_penetration: true,
            compute_impact_geometry_on_penetration: false,
        };
        let filter = self.rapier_query_filter(filter);

        let (collider_handle, hit) = self.query_pipeline.cast_shape(
            &self.bodies,
            &self.colliders,
            &iso,
            &rapier_dir,
            &*rapier_shape,
            options,
            filter,
        )?;

        let entity = self
            .collider_map
            .iter()
            .find(|(_, &(handle, _))| handle == collider_handle)
            .map(|(&entity, _)| entity)?;

        // `normal1`/`witness1` refer to the hit world collider in world
        // space (rapier swaps the composite-shape side for shape casts).
        Some(RaycastHit {
            entity,
            point: glam::Vec3::new(hit.witness1.x, hit.witness1.y, hit.witness1.z),
            normal: glam::Vec3::new(hit.normal1.x, hit.normal1.y, hit.normal1.z),
            distance: hit.time_of_impact,
        })
    }

    /// Find all entities whose colliders overlap with the given shape.
    pub fn query_proximity(&self, shape: &ColliderShape, pos: glam::Vec3) -> Vec<Entity> {
        self.query_proximity_filtered(shape, pos, &PhysicsQueryFilter::default())
    }

    /// Find all entities whose colliders overlap with the given shape,
    /// honouring a candidate filter.
    pub fn query_proximity_filtered(
        &self,
        shape: &ColliderShape,
        pos: glam::Vec3,
        filter: &PhysicsQueryFilter,
    ) -> Vec<Entity> {
        let Some(rapier_shape) = to_rapier_shared_shape(shape) else {
            return Vec::new();
        };
        let iso = Isometry::from_parts(
            na::Translation3::new(pos.x, pos.y, pos.z),
            na::UnitQuaternion::identity(),
        );
        let filter = self.rapier_query_filter(filter);
        let mut entities = Vec::new();

        self.query_pipeline.intersections_with_shape(
            &self.bodies,
            &self.colliders,
            &iso,
            &*rapier_shape,
            filter,
            |collider_handle| {
                if let Some(entity) = self
                    .collider_map
                    .iter()
                    .find(|(_, &(handle, _))| handle == collider_handle)
                    .map(|(&entity, _)| entity)
                {
                    entities.push(entity);
                }
                true
            },
        );

        entities
    }

    /// Execute a set of batched queries in a single call.
    ///
    /// Processes all queued raycasts, overlaps, and sweeps, collecting
    /// per-query detailed results and a flat list of all entity hits.
    pub fn execute_batched_queries(&self, batcher: &QueryBatcher) -> QueryResults {
        use rapier3d::parry::query::details::ShapeCastOptions;

        // Build reverse map: collider handle → complete entity for O(1) lookup.
        let mut col_handle_to_entity: HashMap<ColliderHandle, Entity> = HashMap::new();
        for (&entity, &(ch, _)) in &self.collider_map {
            col_handle_to_entity.insert(ch, entity);
        }

        let mut all_hits: Vec<Entity> = Vec::new();
        let mut raycast_details: Vec<Vec<RaycastHitResult>> = Vec::new();
        let mut overlap_details: Vec<Vec<OverlapHitResult>> = Vec::new();
        let mut sweep_details: Vec<Vec<SweepHitResult>> = Vec::new();

        // ── 1. Raycasts ────────────────────────────────────────────────
        for q in &batcher.raycasts {
            let rapier_origin = na::Point3::new(q.origin.x, q.origin.y, q.origin.z);
            let rapier_dir = na::Vector3::new(q.direction.x, q.direction.y, q.direction.z);
            let ray = Ray::new(rapier_origin, rapier_dir);
            let filter = QueryFilter::default().exclude_sensors();

            let mut hits: Vec<RaycastHitResult> = Vec::new();
            self.query_pipeline.intersections_with_ray(
                &self.bodies,
                &self.colliders,
                &ray,
                q.max_distance,
                true, // solid
                filter,
                |handle, intersection| {
                    if let Some(&entity) = col_handle_to_entity.get(&handle) {
                        // `RayIntersection` has `time_of_impact`, `normal`, `feature`
                        let toi = intersection.time_of_impact;
                        hits.push(RaycastHitResult {
                            entity,
                            point: q.origin + q.direction * toi,
                            normal: glam::Vec3::new(
                                intersection.normal.x,
                                intersection.normal.y,
                                intersection.normal.z,
                            ),
                            distance: toi,
                        });
                        all_hits.push(entity);
                    }
                    true // continue collecting
                },
            );
            raycast_details.push(hits);
        }

        // ── 2. Overlaps ────────────────────────────────────────────────
        for q in &batcher.overlaps {
            let Some(shape) = to_rapier_shared_shape(&q.shape) else {
                overlap_details.push(Vec::new());
                continue;
            };
            let pos = Isometry::from_parts(
                na::Translation3::new(q.position.x, q.position.y, q.position.z),
                na::UnitQuaternion::identity(),
            );
            let filter = QueryFilter::default().exclude_sensors();

            let mut hits: Vec<OverlapHitResult> = Vec::new();
            self.query_pipeline.intersections_with_shape(
                &self.bodies,
                &self.colliders,
                &pos,
                &*shape,
                filter,
                |handle| {
                    if let Some(&entity) = col_handle_to_entity.get(&handle) {
                        hits.push(OverlapHitResult { entity });
                        all_hits.push(entity);
                    }
                    true
                },
            );
            overlap_details.push(hits);
        }

        // ── 3. Sweeps (shape casts) ────────────────────────────────────
        for q in &batcher.sweeps {
            let Some(shape) = to_rapier_shared_shape(&q.shape) else {
                sweep_details.push(Vec::new());
                continue;
            };
            let from_pos = Isometry::from_parts(
                na::Translation3::new(q.from.x, q.from.y, q.from.z),
                na::UnitQuaternion::identity(),
            );
            let delta = na::Vector3::new(q.to.x - q.from.x, q.to.y - q.from.y, q.to.z - q.from.z);
            let max_dist = delta.magnitude();
            let dir = if max_dist > f32::EPSILON {
                delta / max_dist
            } else {
                // Zero-length sweep (from == to) — no movement, skip
                let no_hits: Vec<SweepHitResult> = Vec::new();
                sweep_details.push(no_hits);
                continue;
            };

            let filter = QueryFilter::default().exclude_sensors();
            let options = ShapeCastOptions {
                max_time_of_impact: max_dist,
                target_distance: 0.0,
                stop_at_penetration: true,
                compute_impact_geometry_on_penetration: false,
            };

            let mut hits: Vec<SweepHitResult> = Vec::new();
            // `cast_shape` returns the closest hit; Rapier does not have
            // a multi-hit sweep API, so we accept the single closest.
            if let Some((handle, hit)) = self.query_pipeline.cast_shape(
                &self.bodies,
                &self.colliders,
                &from_pos,
                &dir,
                &*shape,
                options,
                filter,
            ) {
                if let Some(&entity) = col_handle_to_entity.get(&handle) {
                    let point = q.from + glam::Vec3::new(dir.x, dir.y, dir.z) * hit.time_of_impact;
                    hits.push(SweepHitResult {
                        entity,
                        point,
                        normal: glam::Vec3::new(hit.normal1.x, hit.normal1.y, hit.normal1.z),
                        distance: hit.time_of_impact,
                    });
                    all_hits.push(entity);
                }
            }
            sweep_details.push(hits);
        }

        QueryResults {
            hits: all_hits,
            raycast_details,
            overlap_details,
            sweep_details,
        }
    }

    /// Update the query pipeline after structural changes.
    pub fn sync_query_pipeline(&mut self) {
        if self.query_pipeline_dirty {
            self.query_pipeline.update(&self.colliders);
            self.query_pipeline_dirty = false;
        }
    }

    pub(crate) fn query_pipeline_is_dirty(&self) -> bool {
        self.query_pipeline_dirty
    }

    /// Check whether an entity has a body registered.
    pub fn has_body(&self, entity: Entity) -> bool {
        self.body_map.contains_key(&entity)
    }

    /// Check whether an entity has a collider registered.
    pub fn has_collider(&self, entity: Entity) -> bool {
        self.collider_map.contains_key(&entity)
    }

    /// Return a reference to the bodies set.
    pub fn bodies(&self) -> &RigidBodySet {
        &self.bodies
    }

    /// Return a reference to the colliders set.
    pub fn colliders(&self) -> &ColliderSet {
        &self.colliders
    }

    /// Return an iterator over all (entity, body_handle) pairs.
    pub fn body_entries(&self) -> impl Iterator<Item = (Entity, RigidBodyHandle)> + '_ {
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
