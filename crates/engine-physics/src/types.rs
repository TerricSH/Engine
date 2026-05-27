use rapier3d::dynamics;
use rapier3d::geometry;
use thiserror::Error;

// ── Error type ──────────────────────────────────────────────────────────────

/// Typed errors returned by [`PhysicsWorld`] methods.
#[derive(Error, Debug)]
pub enum PhysicsError {
    #[error("rigid body handle is invalid or has been removed")]
    InvalidHandle,
    #[error("collider handle is invalid or has been removed")]
    InvalidColliderHandle,
    #[error("physics world is stopped or not initialized")]
    WorldStopped,
}

// ── Opaque handles ──────────────────────────────────────────────────────────

/// Opaque handle to a rigid body managed by [`PhysicsWorld`].
///
/// Obtained from [`PhysicsWorld::add_dynamic_body`] or
/// [`PhysicsWorld::add_static_body`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RigidBodyHandle(pub(crate) dynamics::RigidBodyHandle);

/// Opaque handle to a collider attached to a rigid body.
///
/// Obtained from [`PhysicsWorld::add_collider`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ColliderHandle(pub(crate) geometry::ColliderHandle);

// ── Collider shape ──────────────────────────────────────────────────────────

/// Describes the shape of a collider before it is attached to a rigid body.
#[derive(Debug, Clone, PartialEq)]
pub enum ColliderShape {
    /// Axis-aligned box defined by its half-extents along each local axis.
    Cuboid {
        hx: f32,
        hy: f32,
        hz: f32,
    },
    /// Sphere with the given radius.
    Sphere {
        radius: f32,
    },
    /// Capsule (cylinder capped with hemispheres) aligned to the local +Y axis.
    Capsule {
        half_height: f32,
        radius: f32,
    },
}

// ── Ray-cast result ─────────────────────────────────────────────────────────

/// Result of a ray-cast query performed by [`PhysicsWorld::cast_ray`].
#[derive(Debug, Clone, PartialEq)]
pub struct RayHit {
    /// World-space intersection point.
    pub point: glam::Vec3,
    /// Surface normal at the intersection point.
    pub normal: glam::Vec3,
    /// Distance from the ray origin to the intersection.
    pub distance: f32,
    /// Handle of the rigid body that was hit.
    pub body_handle: RigidBodyHandle,
}

// ── Contact event data ──────────────────────────────────────────────────────

/// A contact event (started or stopped) produced during
/// [`PhysicsWorld::step`].
///
/// Read these events by draining the channel returned by
/// [`PhysicsWorld::contact_receiver`] between steps.
#[derive(Debug, Clone, PartialEq)]
pub struct ContactEventData {
    pub collider1: ColliderHandle,
    pub collider2: ColliderHandle,
    pub started: bool,
}
