use crossbeam_channel::{Receiver, Sender};
use rapier3d::na;
use rapier3d::prelude::*;
use tracing::{debug, info};

use crate::convert::{from_rapier_isometry, to_rapier_isometry};
use crate::types::{ColliderHandle, ColliderShape, ContactEventData, PhysicsError, RayHit, RigidBodyHandle};

// ── Event handler ───────────────────────────────────────────────────────────

struct PhysicsEventHandler {
    contact_tx: Sender<ContactEventData>,
}

impl EventHandler for PhysicsEventHandler {
    fn handle_collision_event(
        &self,
        _bodies: &RigidBodySet,
        _colliders: &ColliderSet,
        event: CollisionEvent,
        _contact_pair: Option<&ContactPair>,
    ) {
        let _ = self.contact_tx.send(ContactEventData {
            collider1: ColliderHandle(event.collider1()),
            collider2: ColliderHandle(event.collider2()),
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
        // Intentionally not forwarded in this version.
    }
}

// ── PhysicsWorld ────────────────────────────────────────────────────────────

/// A Rapier 3D physics simulation world.
///
/// Owns all simulation state (rigid bodies, colliders, joints, broad-phase,
/// narrow-phase) and exposes an engine-idiomatic API using `glam` types on
/// the public boundary. No `rapier3d` or `nalgebra` types leak into the
/// public API (per FD-030 / FD-031).
///
/// Gravity defaults to `(0.0, -9.81, 0.0)` per FD-031 in a right-handed +Y up
/// coordinate system with metres as the unit of distance.
pub struct PhysicsWorld {
    gravity: glam::Vec3,
    integration_parameters: IntegrationParameters,
    pipeline: PhysicsPipeline,
    island_manager: IslandManager,
    broad_phase: BroadPhaseMultiSap,
    narrow_phase: NarrowPhase,
    bodies: RigidBodySet,
    colliders: ColliderSet,
    impulse_joints: ImpulseJointSet,
    multibody_joints: MultibodyJointSet,
    ccd_solver: CCDSolver,
    query_pipeline: QueryPipeline,
    contact_tx: Sender<ContactEventData>,
    contact_rx: Receiver<ContactEventData>,
}

impl PhysicsWorld {
    /// Create a new physics world with the given gravity vector.
    ///
    /// Per FD-031 the engine convention is `(0.0, -9.81, 0.0)` in +Y up.
    pub fn new(gravity: glam::Vec3) -> Self {
        let (contact_tx, contact_rx) = crossbeam_channel::unbounded();
        info!(?gravity, "Creating PhysicsWorld");
        Self {
            gravity,
            integration_parameters: IntegrationParameters::default(),
            pipeline: PhysicsPipeline::new(),
            island_manager: IslandManager::new(),
            broad_phase: BroadPhaseMultiSap::new(),
            narrow_phase: NarrowPhase::new(),
            bodies: RigidBodySet::new(),
            colliders: ColliderSet::new(),
            impulse_joints: ImpulseJointSet::new(),
            multibody_joints: MultibodyJointSet::new(),
            ccd_solver: CCDSolver::new(),
            query_pipeline: QueryPipeline::new(),
            contact_tx,
            contact_rx,
        }
    }

    /// Advance the simulation by one fixed-timestep tick.
    ///
    /// `fixed_dt` is a delta-time in seconds (e.g. `1.0 / 60.0`).
    /// Contact events generated during this step are queued into the channel
    /// returned by [`contact_receiver`](Self::contact_receiver).
    pub fn step(&mut self, fixed_dt: f32) {
        self.integration_parameters.dt = fixed_dt;

        debug!(
            dt = fixed_dt,
            body_count = self.bodies.len(),
            collider_count = self.colliders.len(),
            "Physics step starting"
        );

        // Drain stale events from the previous step to avoid accumulation.
        while self.contact_rx.try_recv().is_ok() {}

        let event_handler = PhysicsEventHandler {
            contact_tx: self.contact_tx.clone(),
        };

        let rapier_gravity = na::Vector3::new(self.gravity.x, self.gravity.y, self.gravity.z);

        self.pipeline.step(
            &rapier_gravity,
            &self.integration_parameters,
            &mut self.island_manager,
            &mut self.broad_phase,
            &mut self.narrow_phase,
            &mut self.bodies,
            &mut self.colliders,
            &mut self.impulse_joints,
            &mut self.multibody_joints,
            &mut self.ccd_solver,
            Some(&mut self.query_pipeline),
            &(),
            &event_handler,
        );

        debug!("Physics step complete");
    }

    // ── Gravity ─────────────────────────────────────────────────────────────

    /// Set the gravity vector at runtime.
    pub fn set_gravity(&mut self, gravity: glam::Vec3) {
        debug!(?gravity, "Setting gravity");
        self.gravity = gravity;
    }

    /// Return the current gravity vector.
    pub fn gravity(&self) -> glam::Vec3 {
        self.gravity
    }

    // ── Rigid body management ───────────────────────────────────────────────

    /// Add a new dynamic (simulated) rigid body at the given position.
    pub fn add_dynamic_body(
        &mut self,
        translation: glam::Vec3,
        rotation: glam::Quat,
    ) -> RigidBodyHandle {
        let iso = to_rapier_isometry(translation, rotation);
        let body = RigidBodyBuilder::dynamic().position(iso).build();
        let handle = self.bodies.insert(body);
        debug!(?handle, "Added dynamic rigid body");
        RigidBodyHandle(handle)
    }

    /// Add a new static (immovable) rigid body at the given position.
    pub fn add_static_body(
        &mut self,
        translation: glam::Vec3,
        rotation: glam::Quat,
    ) -> RigidBodyHandle {
        let iso = to_rapier_isometry(translation, rotation);
        let body = RigidBodyBuilder::fixed().position(iso).build();
        let handle = self.bodies.insert(body);
        debug!(?handle, "Added static rigid body");
        RigidBodyHandle(handle)
    }

    /// Set the position (translation + rotation) of a rigid body.
    ///
    /// Returns [`PhysicsError::InvalidHandle`] if the body has been removed.
    pub fn set_body_position(
        &mut self,
        body: RigidBodyHandle,
        translation: glam::Vec3,
        rotation: glam::Quat,
    ) -> Result<(), PhysicsError> {
        let body_ref = self
            .bodies
            .get_mut(body.0)
            .ok_or(PhysicsError::InvalidHandle)?;
        body_ref.set_position(to_rapier_isometry(translation, rotation), true);
        Ok(())
    }

    /// Read back the world-space position of a rigid body.
    ///
    /// Returns `None` if the handle is invalid or the body has been removed.
    pub fn body_position(&self, body: RigidBodyHandle) -> Option<(glam::Vec3, glam::Quat)> {
        let body_ref = self.bodies.get(body.0)?;
        Some(from_rapier_isometry(body_ref.position()))
    }

    // ── Collider management ─────────────────────────────────────────────────

    /// Attach a collider with the given shape to a rigid body.
    ///
    /// # Panics
    ///
    /// Panics if `body` is not a valid rigid body handle in this world.
    pub fn add_collider(
        &mut self,
        body: RigidBodyHandle,
        shape: ColliderShape,
    ) -> ColliderHandle {
        let shared_shape = match shape {
            ColliderShape::Cuboid { hx, hy, hz } => SharedShape::cuboid(hx, hy, hz),
            ColliderShape::Sphere { radius } => SharedShape::ball(radius),
            ColliderShape::Capsule {
                half_height,
                radius,
            } => {
                // Capsule segment aligned with local +Y.
                let a = na::Point3::new(0.0, -half_height, 0.0);
                let b = na::Point3::new(0.0, half_height, 0.0);
                SharedShape::capsule(a, b, radius)
            }
        };

        let collider = ColliderBuilder::new(shared_shape).build();
        let handle = self
            .colliders
            .insert_with_parent(collider, body.0, &mut self.bodies);

        debug!(?handle, ?body, "Added collider to rigid body");
        ColliderHandle(handle)
    }

    // ── Body removal ────────────────────────────────────────────────────────

    /// Remove a rigid body (and all attached colliders) from the world.
    ///
    /// Returns `true` if the body existed and was removed, `false` if the
    /// handle was already invalid.
    pub fn remove_body(&mut self, body: RigidBodyHandle) -> bool {
        if !self.bodies.contains(body.0) {
            return false;
        }
        let removed = self.bodies.remove(
            body.0,
            &mut self.island_manager,
            &mut self.colliders,
            &mut self.impulse_joints,
            &mut self.multibody_joints,
            true,
        );
        let ok = removed.is_some();
        if ok {
            debug!(?body, "Removed rigid body");
        }
        ok
    }

    // ── Ray-cast queries ────────────────────────────────────────────────────

    /// Cast a ray against all colliders and return the closest hit, if any.
    ///
    /// The `direction` vector should be normalised before calling.
    /// Sensor colliders are excluded from results.
    pub fn cast_ray(
        &self,
        origin: glam::Vec3,
        direction: glam::Vec3,
        max_distance: f32,
    ) -> Option<RayHit> {
        let ray = Ray::new(
            na::Point3::new(origin.x, origin.y, origin.z),
            na::Vector3::new(direction.x, direction.y, direction.z),
        );
        let filter = QueryFilter::default().exclude_sensors();
        let (collider_handle, intersection) = self
            .query_pipeline
            .cast_ray_and_get_normal(
                &self.bodies,
                &self.colliders,
                &ray,
                max_distance,
                true,
                filter,
            )?;

        let collider = self.colliders.get(collider_handle)?;
        let body_handle = collider.parent()?;

        Some(RayHit {
            point: origin + direction * intersection.time_of_impact,
            normal: crate::convert::from_rapier_vec(intersection.normal),
            distance: intersection.time_of_impact,
            body_handle: RigidBodyHandle(body_handle),
        })
    }

    /// Cast a ray and return **all** hits along the ray, sorted by distance
    /// (closest first).
    ///
    /// Sensor colliders are excluded from results.
    pub fn cast_ray_all(
        &self,
        origin: glam::Vec3,
        direction: glam::Vec3,
        max_distance: f32,
    ) -> Vec<RayHit> {
        let ray = Ray::new(
            na::Point3::new(origin.x, origin.y, origin.z),
            na::Vector3::new(direction.x, direction.y, direction.z),
        );
        let filter = QueryFilter::default().exclude_sensors();
        let mut hits: Vec<RayHit> = Vec::new();

        self.query_pipeline.intersections_with_ray(
            &self.bodies,
            &self.colliders,
            &ray,
            max_distance,
            true,
            filter,
            |collider_handle, intersection| {
                if let Some(collider) = self.colliders.get(collider_handle) {
                    if let Some(parent) = collider.parent() {
                        hits.push(RayHit {
                            point: origin + direction * intersection.time_of_impact,
                            normal: crate::convert::from_rapier_vec(intersection.normal),
                            distance: intersection.time_of_impact,
                            body_handle: RigidBodyHandle(parent),
                        });
                    }
                }
                true
            },
        );

        // Sort by distance (closest first) – rapier does not guarantee
        // the callback invocation order for all hits.
        hits.sort_unstable_by(|a, b| {
            a.distance
                .partial_cmp(&b.distance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        hits
    }

    // ── Point query ─────────────────────────────────────────────────────────

    /// Find all colliders that contain the given world-space point.
    ///
    /// Sensor colliders are excluded from results.
    pub fn intersect_point(&self, point: glam::Vec3) -> Vec<ColliderHandle> {
        let rapier_point = na::Point3::new(point.x, point.y, point.z);
        let filter = QueryFilter::default().exclude_sensors();
        let mut handles: Vec<ColliderHandle> = Vec::new();

        self.query_pipeline.intersections_with_point(
            &self.bodies,
            &self.colliders,
            &rapier_point,
            filter,
            |collider_handle| {
                handles.push(ColliderHandle(collider_handle));
                true
            },
        );

        handles
    }

    // ── Contact event channel ───────────────────────────────────────────────

    /// Returns a reference to the contact event receiver.
    ///
    /// Contact events from [`step`](Self::step) are sent to this channel.
    /// Drain it between steps to avoid stale events building up.
    pub fn contact_receiver(&self) -> &Receiver<ContactEventData> {
        &self.contact_rx
    }
}
