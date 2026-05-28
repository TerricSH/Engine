use crate::{
    from_rapier_vec, to_rapier_vec, ColliderHandle, ColliderShape, PhysicsError, RayHit,
    RigidBodyHandle,
};

fn make_rigid_body_handle(raw: u32) -> RigidBodyHandle {
    RigidBodyHandle(rapier3d::dynamics::RigidBodyHandle::from_raw_parts(raw, 0))
}

fn make_collider_handle(raw: u32) -> ColliderHandle {
    ColliderHandle(rapier3d::geometry::ColliderHandle::from_raw_parts(raw, 0))
}

// ── RigidBodyHandle tests ────────────────────────────────────────────────

#[test]
fn rigid_body_handle_debug() {
    let handle = make_rigid_body_handle(42);
    let debug = format!("{:?}", handle);
    assert!(debug.contains("RigidBodyHandle"));
}

#[test]
fn rigid_body_handle_clone_copy() {
    let a = make_rigid_body_handle(1);
    let b = a;
    let c = a;
    assert_eq!(a, b);
    assert_eq!(a, c);
}

#[test]
fn rigid_body_handle_equality() {
    let a = make_rigid_body_handle(5);
    let b = make_rigid_body_handle(5);
    let c = make_rigid_body_handle(10);
    assert_eq!(a, b);
    assert_ne!(a, c);
}

// ── ColliderHandle tests ─────────────────────────────────────────────────

#[test]
fn collider_handle_debug() {
    let handle = make_collider_handle(7);
    let debug = format!("{:?}", handle);
    assert!(debug.contains("ColliderHandle"));
}

#[test]
fn collider_handle_equality() {
    let a = make_collider_handle(3);
    let b = make_collider_handle(3);
    let c = make_collider_handle(5);
    assert_eq!(a, b);
    assert_ne!(a, c);
}

// ── ColliderShape tests ──────────────────────────────────────────────────

#[test]
fn collider_shape_cuboid_debug() {
    let shape = ColliderShape::Cuboid {
        hx: 0.5,
        hy: 1.0,
        hz: 0.5,
    };
    let debug = format!("{:?}", shape);
    assert!(debug.contains("Cuboid"));
    assert!(debug.contains("hx: 0.5"));
}

#[test]
fn collider_shape_sphere_debug() {
    let shape = ColliderShape::Sphere { radius: 1.0 };
    let debug = format!("{:?}", shape);
    assert!(debug.contains("Sphere"));
    assert!(debug.contains("radius: 1.0"));
}

#[test]
fn collider_shape_capsule_debug() {
    let shape = ColliderShape::Capsule {
        half_height: 0.5,
        radius: 0.3,
    };
    let debug = format!("{:?}", shape);
    assert!(debug.contains("Capsule"));
    assert!(debug.contains("half_height: 0.5"));
}

#[test]
fn collider_shape_partial_eq() {
    let a = ColliderShape::Cuboid {
        hx: 1.0,
        hy: 2.0,
        hz: 3.0,
    };
    let b = ColliderShape::Cuboid {
        hx: 1.0,
        hy: 2.0,
        hz: 3.0,
    };
    let c = ColliderShape::Sphere { radius: 1.0 };
    assert_eq!(a, b);
    assert_ne!(a, c);
}

// ── Conversion helper tests ──────────────────────────────────────────────

#[test]
fn to_rapier_vec_converts_glam_to_nalgebra() {
    let glam_v = glam::Vec3::new(1.0, 2.0, 3.0);
    let rapier_v = to_rapier_vec(glam_v);
    assert_eq!(rapier_v.x, 1.0);
    assert_eq!(rapier_v.y, 2.0);
    assert_eq!(rapier_v.z, 3.0);
}

#[test]
fn from_rapier_vec_converts_nalgebra_to_glam() {
    let rapier_v = rapier3d::na::Vector3::new(4.0, 5.0, 6.0);
    let glam_v = from_rapier_vec(rapier_v);
    assert_eq!(glam_v.x, 4.0);
    assert_eq!(glam_v.y, 5.0);
    assert_eq!(glam_v.z, 6.0);
}

#[test]
fn vec_conversion_roundtrip() {
    let original = glam::Vec3::new(-1.5, 2.7, std::f32::consts::PI);
    let rapier_v = to_rapier_vec(original);
    let back = from_rapier_vec(rapier_v);
    assert!((original - back).length() < 1e-6);
}

// ── PhysicsError display tests ───────────────────────────────────────────

#[test]
fn physics_error_invalid_handle_display() {
    let err = PhysicsError::InvalidHandle;
    assert_eq!(
        err.to_string(),
        "rigid body handle is invalid or has been removed"
    );
}

#[test]
fn physics_error_invalid_collider_handle_display() {
    let err = PhysicsError::InvalidColliderHandle;
    assert_eq!(
        err.to_string(),
        "collider handle is invalid or has been removed"
    );
}

#[test]
fn physics_error_world_stopped_display() {
    let err = PhysicsError::WorldStopped;
    assert_eq!(
        err.to_string(),
        "physics world is stopped or not initialized"
    );
}

#[test]
fn physics_error_debug() {
    let err = PhysicsError::WorldStopped;
    let debug = format!("{:?}", err);
    assert!(debug.contains("WorldStopped"));
}

// ── RayHit tests ─────────────────────────────────────────────────────────

#[test]
fn ray_hit_construction() {
    let body_handle = make_rigid_body_handle(1);
    let hit = RayHit {
        point: glam::Vec3::new(0.0, 5.0, 0.0),
        normal: glam::Vec3::Y,
        distance: 5.0,
        body_handle,
    };
    assert_eq!(hit.point, glam::Vec3::new(0.0, 5.0, 0.0));
    assert_eq!(hit.normal, glam::Vec3::Y);
    assert_eq!(hit.distance, 5.0);
    assert_eq!(hit.body_handle, body_handle);
}

#[test]
fn ray_hit_debug_format() {
    let hit = RayHit {
        point: glam::Vec3::ZERO,
        normal: glam::Vec3::Y,
        distance: 1.0,
        body_handle: make_rigid_body_handle(0),
    };
    let debug = format!("{:?}", hit);
    assert!(debug.contains("RayHit"));
    assert!(debug.contains("point"));
    assert!(debug.contains("normal"));
    assert!(debug.contains("distance"));
}

#[test]
fn ray_hit_partial_eq() {
    let bh = make_rigid_body_handle(2);
    let a = RayHit {
        point: glam::Vec3::new(1.0, 0.0, 0.0),
        normal: glam::Vec3::X,
        distance: 1.0,
        body_handle: bh,
    };
    let b = RayHit {
        point: glam::Vec3::new(1.0, 0.0, 0.0),
        normal: glam::Vec3::X,
        distance: 1.0,
        body_handle: make_rigid_body_handle(2),
    };
    let c = RayHit {
        point: glam::Vec3::new(2.0, 0.0, 0.0),
        normal: glam::Vec3::X,
        distance: 2.0,
        body_handle: make_rigid_body_handle(2),
    };
    assert_eq!(a, b);
    assert_ne!(a, c);
}
