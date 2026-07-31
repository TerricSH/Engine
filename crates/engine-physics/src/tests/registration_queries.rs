// Component Registration Tests
// ══════════════════════════════════════════════════════════════════════════════

#[test]
fn physics_component_type_ids_are_unique() {
    assert_ne!(RigidBody::TYPE_ID, Collider::TYPE_ID);
    assert_ne!(RigidBody::TYPE_ID, PhysicsMaterial::TYPE_ID);
    assert_ne!(Collider::TYPE_ID, PhysicsMaterial::TYPE_ID);
    assert_ne!(PhysicsJoint::TYPE_ID, RigidBody::TYPE_ID);
    assert_ne!(PhysicsJoint::TYPE_ID, Collider::TYPE_ID);
    assert_ne!(Destructible::TYPE_ID, PhysicsJoint::TYPE_ID);
}

#[test]
fn register_physics_extensions_adds_all_components() {
    let mut component_registry = engine_scene::registry::ComponentRegistry::new();

    crate::register_physics_extensions(
        &mut component_registry,
        None, // debug_draw_registry
    );

    assert!(component_registry.is_registered(RigidBody::TYPE_ID));
    assert!(component_registry.is_registered(Collider::TYPE_ID));
    assert!(component_registry.is_registered(PhysicsMaterial::TYPE_ID));
    assert!(component_registry.is_registered(PhysicsJoint::TYPE_ID));
    assert!(component_registry.is_registered(Destructible::TYPE_ID));

    // Verify that storages can be created.
    let storages = component_registry.create_storages();
    assert!(storages.contains_key(RigidBody::TYPE_ID));
    assert!(storages.contains_key(Collider::TYPE_ID));
    assert!(storages.contains_key(PhysicsMaterial::TYPE_ID));
    assert!(storages.contains_key(PhysicsJoint::TYPE_ID));
    assert!(storages.contains_key(Destructible::TYPE_ID));
}

#[test]
fn register_physics_extensions_with_debug_draw() {
    let mut component_registry = engine_scene::registry::ComponentRegistry::new();
    let mut debug_registry = engine_renderer::debug_draw::DebugDrawRegistry::new();

    crate::register_physics_extensions(&mut component_registry, Some(&mut debug_registry));

    assert_eq!(debug_registry.provider_count(), 1);
}

#[test]
fn register_physics_extensions_is_idempotent() {
    let mut component_registry = engine_scene::registry::ComponentRegistry::new();

    crate::register_physics_extensions(&mut component_registry, None);
    crate::register_physics_extensions(&mut component_registry, None);

    // Should not panic or duplicate.
    assert!(component_registry.is_registered(RigidBody::TYPE_ID));
}

// ══════════════════════════════════════════════════════════════════════════════
// Query Types Tests
// ══════════════════════════════════════════════════════════════════════════════

#[test]
fn raycast_query_struct() {
    let q = crate::RaycastQuery {
        origin: glam::Vec3::ZERO,
        direction: glam::Vec3::Y,
        max_distance: 100.0,
    };
    assert_eq!(q.max_distance, 100.0);
}

#[test]
fn overlap_query_struct() {
    let q = crate::OverlapQuery {
        shape: ColliderShape::Ball { radius: 1.0 },
        position: glam::Vec3::ZERO,
    };
    assert_eq!(q.shape, ColliderShape::Ball { radius: 1.0 });
}

#[test]
fn sweep_query_struct() {
    let q = crate::SweepQuery {
        shape: ColliderShape::Cuboid {
            hx: 0.5,
            hy: 0.5,
            hz: 0.5,
        },
        from: glam::Vec3::ZERO,
        to: glam::Vec3::new(10.0, 0.0, 0.0),
    };
    assert_eq!(q.from, glam::Vec3::ZERO);
    assert_eq!(q.to.x, 10.0);
}

#[test]
fn query_results_default_empty() {
    let r = crate::QueryResults::new();
    assert!(r.is_empty());
    assert_eq!(r.len(), 0);
}

// ══════════════════════════════════════════════════════════════════════════════
// Conversion Tests
// ══════════════════════════════════════════════════════════════════════════════

#[test]
fn vec_conversion_roundtrip() {
    let original = glam::Vec3::new(-1.5, 2.7, std::f32::consts::PI);
    let rapier_v = crate::to_rapier_vec(original);
    let back = crate::from_rapier_vec(rapier_v);
    assert!((original - back).length() < 1e-6);
}

#[test]
fn to_rapier_vec_converts_glam_to_nalgebra() {
    let glam_v = glam::Vec3::new(1.0, 2.0, 3.0);
    let rapier_v = crate::to_rapier_vec(glam_v);
    assert_eq!(rapier_v.x, 1.0);
    assert_eq!(rapier_v.y, 2.0);
    assert_eq!(rapier_v.z, 3.0);
}

#[test]
fn from_rapier_vec_converts_nalgebra_to_glam() {
    let rapier_v = rapier3d::na::Vector3::new(4.0, 5.0, 6.0);
    let glam_v = crate::from_rapier_vec(rapier_v);
    assert_eq!(glam_v.x, 4.0);
    assert_eq!(glam_v.y, 5.0);
    assert_eq!(glam_v.z, 6.0);
}

// ══════════════════════════════════════════════════════════════════════════════
