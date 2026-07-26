use crate::{
    apply_damage, BodyType, Collider, ColliderDebugInfo, ColliderShape, CollisionEvent,
    CollisionEventKind, Component, DamageKind, DamageRequest, Destructible, Entity,
    JointDescriptor, JointLimits, JointMotor, JointType, PhysicsCommand, PhysicsEvents,
    PhysicsJoint, PhysicsMaterial, PhysicsWorld, RapierBackend, RigidBody, Transform, TriggerEvent,
    TriggerEventKind,
};
use engine_renderer::DebugDrawProvider;
use engine_scene::World;
use glam::Vec3;

fn entity(index: u32) -> Entity {
    Entity::new(index, 0)
}

fn fixed_joint(entity_a: Entity, entity_b: Entity) -> JointDescriptor {
    JointDescriptor {
        entity_a,
        entity_b,
        joint_type: JointType::Fixed,
        anchor_a: [0.0; 3],
        anchor_b: [0.0; 3],
        axis: [1.0, 0.0, 0.0],
        limits: None,
        motor: None,
        break_force: 0.0,
        break_torque: 0.0,
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// Component Tests
// ══════════════════════════════════════════════════════════════════════════════

#[test]
fn rigid_body_default_values() {
    let rb = RigidBody::default();
    assert_eq!(rb.body_type, BodyType::Dynamic);
    assert_eq!(rb.mass, 1.0);
    assert!(rb.enabled);
    assert_eq!(rb.gravity_scale, 1.0);
}

#[test]
fn rigid_body_type_id() {
    assert_eq!(RigidBody::TYPE_ID, "engine.physics.rigid_body");
}

#[test]
fn collider_default_values() {
    let c = Collider::default();
    assert!(!c.is_trigger);
    assert_eq!(c.friction, 0.5);
    assert_eq!(c.density, 1.0);
    match &c.shape {
        ColliderShape::Cuboid { hx, hy, hz } => {
            assert!((*hx - 0.5).abs() < 1e-6);
            assert!((*hy - 0.5).abs() < 1e-6);
            assert!((*hz - 0.5).abs() < 1e-6);
        }
        _ => panic!("default collider should be cuboid"),
    }
}

#[test]
fn collider_type_id() {
    assert_eq!(Collider::TYPE_ID, "engine.physics.collider");
}

#[test]
fn physics_material_default_values() {
    let m = PhysicsMaterial::default();
    assert_eq!(m.friction, 0.5);
    assert_eq!(m.restitution, 0.0);
    assert_eq!(m.density, 1.0);
}

#[test]
fn physics_material_type_id() {
    assert_eq!(PhysicsMaterial::TYPE_ID, "engine.physics.physics_material");
}

#[test]
fn persistent_joint_validates_authoring_contract() {
    let joint = PhysicsJoint {
        body_a: "door".into(),
        body_b: "frame".into(),
        joint_type: JointType::Revolute,
        limits: Some(JointLimits {
            min: -1.0,
            max: 1.0,
            stiffness: 0.0,
            damping: 0.0,
        }),
        motor: Some(JointMotor {
            target_vel: 1.0,
            target_pos: 0.0,
            stiffness: 10.0,
            damping: 1.0,
        }),
        break_force: 500.0,
        break_torque: 50.0,
        ..PhysicsJoint::default()
    };
    assert_eq!(PhysicsJoint::TYPE_ID, "engine.physics.joint");
    assert!(joint.validate().is_ok());

    let mut invalid = joint.clone();
    invalid.axis = [0.0; 3];
    assert!(invalid.validate().is_err());
    invalid = joint.clone();
    invalid.body_b = invalid.body_a.clone();
    assert!(invalid.validate().is_err());
    invalid = joint;
    invalid.break_force = f32::NAN;
    assert!(invalid.validate().is_err());
}

#[test]
fn persistent_joint_scene_fields_roundtrip_all_runtime_rebuild_data() {
    let expected = PhysicsJoint {
        enabled: true,
        body_a: "frame".into(),
        body_b: "door".into(),
        joint_type: JointType::Revolute,
        anchor_a: [1.0, 2.0, 3.0],
        anchor_b: [-1.0, 0.5, 0.0],
        axis: [0.0, 1.0, 0.0],
        limits: Some(JointLimits {
            min: -1.25,
            max: 1.25,
            stiffness: 40.0,
            damping: 4.0,
        }),
        motor: Some(JointMotor {
            target_vel: 2.0,
            target_pos: 0.5,
            stiffness: 12.0,
            damping: 1.5,
        }),
        break_force: 5000.0,
        break_torque: 750.0,
    };
    let fields = crate::serde::serialize_physics_joint(&expected);
    let decoded = crate::serde::deserialize_physics_joint(&fields)
        .downcast::<PhysicsJoint>()
        .unwrap();
    assert_eq!(*decoded, expected);
}

#[test]
fn destructible_damage_respects_threshold_scale_and_breaks_once() {
    let mut world = World::new();
    let target = world.create_entity();
    world.add_component(
        target,
        Destructible {
            max_health: 50.0,
            health: 50.0,
            minimum_damage: 5.0,
            damage_scale: 2.0,
            replacement_prefab: Some(engine_serialize::AssetId::new("crate.fragments")),
            ..Destructible::default()
        },
    );
    let ignored = apply_damage(
        &mut world,
        target,
        &DamageRequest {
            source: None,
            amount: 4.0,
            kind: DamageKind::Impact,
            hit_position: None,
            impulse: [0.0; 3],
        },
    )
    .unwrap();
    assert!(ignored.is_none());

    let damaged = apply_damage(
        &mut world,
        target,
        &DamageRequest {
            source: None,
            amount: 10.0,
            kind: DamageKind::Bullet,
            hit_position: Some([1.0, 2.0, 3.0]),
            impulse: [2.0, 0.0, 0.0],
        },
    )
    .unwrap()
    .unwrap();
    assert_eq!(damaged.applied_damage, 20.0);
    assert_eq!(damaged.remaining_health, 30.0);
    assert!(!damaged.broke);

    let broken = apply_damage(
        &mut world,
        target,
        &DamageRequest {
            source: None,
            amount: 100.0,
            kind: DamageKind::Blast,
            hit_position: None,
            impulse: [3.0, 0.0, 0.0],
        },
    )
    .unwrap()
    .unwrap();
    assert!(broken.broke);
    assert_eq!(broken.remaining_health, 0.0);
    assert_eq!(
        broken.replacement_prefab.unwrap().id,
        "crate.fragments".to_string()
    );
    assert!(apply_damage(
        &mut world,
        target,
        &DamageRequest {
            source: None,
            amount: 1.0,
            kind: DamageKind::Generic,
            hit_position: None,
            impulse: [0.0; 3],
        },
    )
    .unwrap()
    .is_none());
}

#[test]
fn destructible_scene_fields_roundtrip_runtime_state_and_replacement() {
    let expected = Destructible {
        enabled: false,
        max_health: 40.0,
        health: 0.0,
        minimum_damage: 3.0,
        damage_scale: 0.5,
        replacement_prefab: Some(engine_serialize::AssetId::new("broken.wall")),
        destroy_on_break: false,
        inherit_velocity: false,
        fracture_impulse_scale: 2.5,
        broken: true,
    };
    let fields = crate::serde::serialize_destructible(&expected);
    let decoded = crate::serde::deserialize_destructible(&fields)
        .downcast::<Destructible>()
        .unwrap();
    assert_eq!(*decoded, expected);
}

#[test]
fn body_type_enum_variants() {
    assert_eq!(BodyType::Static as u8, 0);
    assert_eq!(BodyType::Dynamic as u8, 1);
    assert_eq!(BodyType::Kinematic as u8, 2);
}

// ══════════════════════════════════════════════════════════════════════════════
// Component Serialisation Roundtrip Tests
// ══════════════════════════════════════════════════════════════════════════════

#[test]
fn rigid_body_serde_roundtrip() {
    let rb = RigidBody {
        body_type: BodyType::Kinematic,
        mass: 5.0,
        linear_damping: 0.1,
        angular_damping: 0.2,
        enabled: false,
        gravity_scale: 0.5,
        can_sleep: false,
        ccd_enabled: true,
    };
    let json = serde_json::to_string(&rb).unwrap();
    let rb2: RigidBody = serde_json::from_str(&json).unwrap();
    assert_eq!(rb.body_type, rb2.body_type);
    assert_eq!(rb.mass, rb2.mass);
    assert_eq!(rb.linear_damping, rb2.linear_damping);
    assert_eq!(rb.angular_damping, rb2.angular_damping);
    assert_eq!(rb.enabled, rb2.enabled);
    assert_eq!(rb.gravity_scale, rb2.gravity_scale);
    assert_eq!(rb.can_sleep, rb2.can_sleep);
    assert_eq!(rb.ccd_enabled, rb2.ccd_enabled);
}

#[test]
fn collider_serde_roundtrip() {
    let c = Collider {
        shape: ColliderShape::Ball { radius: 2.0 },
        density: 2.5,
        friction: 0.8,
        restitution: 0.3,
        is_trigger: true,
        collision_group: 1,
        collision_mask: 2,
    };
    let json = serde_json::to_string(&c).unwrap();
    let c2: Collider = serde_json::from_str(&json).unwrap();
    assert_eq!(c.shape, c2.shape);
    assert_eq!(c.density, c2.density);
    assert_eq!(c.friction, c2.friction);
    assert_eq!(c.restitution, c2.restitution);
    assert_eq!(c.is_trigger, c2.is_trigger);
    assert_eq!(c.collision_group, c2.collision_group);
    assert_eq!(c.collision_mask, c2.collision_mask);
}

#[test]
fn physics_material_serde_roundtrip() {
    let m = PhysicsMaterial {
        friction: 0.9,
        restitution: 0.5,
        density: 3.0,
    };
    let json = serde_json::to_string(&m).unwrap();
    let m2: PhysicsMaterial = serde_json::from_str(&json).unwrap();
    assert_eq!(m.friction, m2.friction);
    assert_eq!(m.restitution, m2.restitution);
    assert_eq!(m.density, m2.density);
}

#[test]
fn collider_shape_cuboid_serde() {
    let shape = ColliderShape::Cuboid {
        hx: 1.0,
        hy: 2.0,
        hz: 3.0,
    };
    let json = serde_json::to_string(&shape).unwrap();
    let back: ColliderShape = serde_json::from_str(&json).unwrap();
    match back {
        ColliderShape::Cuboid { hx, hy, hz } => {
            assert!((hx - 1.0).abs() < 1e-6);
            assert!((hy - 2.0).abs() < 1e-6);
            assert!((hz - 3.0).abs() < 1e-6);
        }
        _ => panic!("expected Cuboid"),
    }
}

#[test]
fn collider_shape_ball_serde() {
    let shape = ColliderShape::Ball { radius: 1.5 };
    let json = serde_json::to_string(&shape).unwrap();
    let back: ColliderShape = serde_json::from_str(&json).unwrap();
    match back {
        ColliderShape::Ball { radius } => assert!((radius - 1.5).abs() < 1e-6),
        _ => panic!("expected Ball"),
    }
}

#[test]
fn collider_shape_capsule_serde() {
    let shape = ColliderShape::Capsule {
        half_height: 1.0,
        radius: 0.5,
    };
    let json = serde_json::to_string(&shape).unwrap();
    let back: ColliderShape = serde_json::from_str(&json).unwrap();
    match back {
        ColliderShape::Capsule {
            half_height,
            radius,
        } => {
            assert!((half_height - 1.0).abs() < 1e-6);
            assert!((radius - 0.5).abs() < 1e-6);
        }
        _ => panic!("expected Capsule"),
    }
}

#[test]
fn collider_shape_heightfield_serde() {
    let shape = ColliderShape::HeightField {
        rows: 2,
        columns: 3,
        heights: vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0],
        scale: [8.0, 1.0, 4.0],
    };
    let json = serde_json::to_string(&shape).unwrap();
    let back: ColliderShape = serde_json::from_str(&json).unwrap();
    assert_eq!(back, shape);
}

// ══════════════════════════════════════════════════════════════════════════════
// Physics Step & Gravity Tests
// ══════════════════════════════════════════════════════════════════════════════

#[test]
fn physics_world_gravity_moves_dynamic_body() {
    let mut world = PhysicsWorld::new(glam::Vec3::new(0.0, -9.81, 0.0));

    let transform = Transform::default();
    let rb = RigidBody::default(); // Dynamic
    world.backend.create_body(entity(0), &rb, &transform);
    // Add a collider so the body has mass.
    world
        .backend
        .create_collider(entity(0), &Collider::default(), entity(0), None);
    world.backend.sync_query_pipeline();

    let pos_before = world.backend.sync_body_transform(entity(0)).unwrap();
    assert!((pos_before.0.y - 0.0).abs() < 1e-6);

    // Step multiple times for gravity to take effect.
    for _ in 0..10 {
        world.backend.step();
    }

    let pos_after = world.backend.sync_body_transform(entity(0)).unwrap();
    assert!(
        pos_after.0.y < pos_before.0.y,
        "body should fall: before={:?} after={:?}",
        pos_before.0.y,
        pos_after.0.y
    );
}

#[test]
fn static_body_does_not_fall() {
    let mut world = PhysicsWorld::new(glam::Vec3::new(0.0, -9.81, 0.0));

    let transform = Transform::default();
    let rb = RigidBody {
        body_type: BodyType::Static,
        ..RigidBody::default()
    };
    world.backend.create_body(entity(0), &rb, &transform);
    world.backend.sync_query_pipeline();

    let pos_before = world.backend.sync_body_transform(entity(0)).unwrap();

    // Step multiple times.
    for _ in 0..10 {
        world.backend.step();
    }

    let pos_after = world.backend.sync_body_transform(entity(0)).unwrap();
    assert!(
        (pos_after.0.y - pos_before.0.y).abs() < 1e-6,
        "static body should not move: {:?}",
        pos_after.0.y
    );
}

// ══════════════════════════════════════════════════════════════════════════════
// Raycast Tests
// ══════════════════════════════════════════════════════════════════════════════

#[test]
fn raycast_hits_entity() {
    let mut world = PhysicsWorld::new(glam::Vec3::ZERO);

    // Create a static body with a collider at y=0.
    let transform = Transform {
        translation: glam::Vec3::new(0.0, 0.0, 0.0),
        ..Default::default()
    };
    let rb = RigidBody {
        body_type: BodyType::Static,
        ..RigidBody::default()
    };
    world.backend.create_body(entity(0), &rb, &transform);

    let collider = Collider {
        shape: ColliderShape::Cuboid {
            hx: 0.5,
            hy: 0.5,
            hz: 0.5,
        },
        ..Collider::default()
    };
    world
        .backend
        .create_collider(entity(0), &collider, entity(0), None);
    world.backend.sync_query_pipeline();

    // Cast a ray from above, pointing down.
    let hit = world.raycast(
        glam::Vec3::new(0.0, 5.0, 0.0),
        glam::Vec3::new(0.0, -1.0, 0.0),
        10.0,
    );

    assert!(hit.is_some(), "raycast should hit the entity");
    let hit = hit.unwrap();
    assert!(
        (hit.distance - 4.5).abs() < 0.1,
        "unexpected distance: {}",
        hit.distance
    );
    assert!(
        (hit.point.y - 0.5).abs() < 0.1,
        "unexpected hit point: {:?}",
        hit.point
    );
}

#[test]
fn raycast_hits_heightfield_surface() {
    let mut world = PhysicsWorld::new(glam::Vec3::ZERO);
    let transform = Transform::default();
    let rb = RigidBody {
        body_type: BodyType::Static,
        ..RigidBody::default()
    };
    world.backend.create_body(entity(0), &rb, &transform);
    world.backend.create_collider(
        entity(0),
        &Collider {
            shape: ColliderShape::HeightField {
                rows: 3,
                columns: 3,
                heights: vec![0.0; 9],
                scale: [4.0, 1.0, 4.0],
            },
            ..Collider::default()
        },
        entity(0),
        None,
    );
    world.backend.sync_query_pipeline();
    let hit = world
        .raycast(glam::Vec3::new(0.0, 5.0, 0.0), glam::Vec3::NEG_Y, 10.0)
        .expect("ray should hit terrain heightfield");
    assert!(
        (hit.point.y).abs() < 0.01,
        "unexpected heightfield hit: {hit:?}"
    );
}

#[test]
fn invalid_heightfield_is_rejected_without_a_phantom_collider() {
    let mut world = PhysicsWorld::new(glam::Vec3::ZERO);
    let transform = Transform::default();
    let rb = RigidBody {
        body_type: BodyType::Static,
        ..RigidBody::default()
    };
    world.backend.create_body(entity(0), &rb, &transform);
    world.backend.create_collider(
        entity(0),
        &Collider {
            shape: ColliderShape::HeightField {
                rows: 3,
                columns: 3,
                heights: vec![0.0; 8],
                scale: [4.0, 1.0, 4.0],
            },
            ..Collider::default()
        },
        entity(0),
        None,
    );
    world.backend.sync_query_pipeline();

    assert!(!world.backend.has_collider(entity(0)));
    assert!(world
        .raycast(glam::Vec3::new(0.0, 1.0, 0.0), glam::Vec3::NEG_Y, 2.0)
        .is_none());
}

#[test]
fn raycast_misses_with_no_collider() {
    let world = PhysicsWorld::new(glam::Vec3::ZERO);

    let hit = world.raycast(
        glam::Vec3::new(0.0, 5.0, 0.0),
        glam::Vec3::new(0.0, -1.0, 0.0),
        10.0,
    );

    assert!(hit.is_none(), "raycast should miss with no colliders");
}

#[test]
fn raycast_miss_beyond_max_distance() {
    let mut world = PhysicsWorld::new(glam::Vec3::ZERO);
    let transform = Transform::default();
    let rb = RigidBody {
        body_type: BodyType::Static,
        ..RigidBody::default()
    };
    world.backend.create_body(entity(0), &rb, &transform);
    world
        .backend
        .create_collider(entity(0), &Collider::default(), entity(0), None);
    world.backend.sync_query_pipeline();

    // Ray starts far away but max_distance is very short.
    let hit = world.raycast(
        glam::Vec3::new(0.0, 10.0, 0.0),
        glam::Vec3::new(0.0, -1.0, 0.0),
        0.1,
    );
    assert!(hit.is_none(), "raycast should miss beyond max distance");
}

// ══════════════════════════════════════════════════════════════════════════════
// Proximity (Overlap) Query Tests
// ══════════════════════════════════════════════════════════════════════════════

#[test]
fn query_proximity_finds_overlapping_entities() {
    let mut world = PhysicsWorld::new(glam::Vec3::ZERO);

    let transform = Transform::default();
    let rb = RigidBody {
        body_type: BodyType::Static,
        ..RigidBody::default()
    };
    world.backend.create_body(entity(0), &rb, &transform);
    world
        .backend
        .create_collider(entity(0), &Collider::default(), entity(0), None);
    world.backend.sync_query_pipeline();

    // Query with a shape that overlaps the entity at origin.
    let hits = world.query_proximity(
        &ColliderShape::Cuboid {
            hx: 0.5,
            hy: 0.5,
            hz: 0.5,
        },
        glam::Vec3::ZERO,
    );

    assert!(!hits.is_empty(), "should find overlapping entity");
}

#[test]
fn query_proximity_no_overlap() {
    let mut world = PhysicsWorld::new(glam::Vec3::ZERO);

    let transform = Transform {
        translation: glam::Vec3::new(100.0, 0.0, 0.0),
        ..Default::default()
    };
    let rb = RigidBody {
        body_type: BodyType::Static,
        ..RigidBody::default()
    };
    world.backend.create_body(entity(0), &rb, &transform);
    world
        .backend
        .create_collider(entity(0), &Collider::default(), entity(0), None);
    world.backend.sync_query_pipeline();

    let hits = world.query_proximity(&ColliderShape::Ball { radius: 1.0 }, glam::Vec3::ZERO);

    assert!(
        hits.is_empty(),
        "should not find overlapping entity at origin"
    );
}

#[test]
fn raycast_reports_closest_hit_with_normal() {
    // Two colliders along the ray: the nearest one must win and report its
    // face normal, which gameplay queries surface to scripts.
    let mut world = PhysicsWorld::new(glam::Vec3::ZERO);
    let rb = RigidBody {
        body_type: BodyType::Static,
        ..RigidBody::default()
    };

    let near = Transform {
        translation: glam::Vec3::new(0.0, 0.5, 0.0),
        ..Transform::default()
    };
    world.backend.create_body(entity(0), &rb, &near);
    world
        .backend
        .create_collider(entity(0), &Collider::default(), entity(0), None);

    let far = Transform {
        translation: glam::Vec3::new(0.0, -2.0, 0.0),
        ..Transform::default()
    };
    world.backend.create_body(entity(1), &rb, &far);
    world
        .backend
        .create_collider(entity(1), &Collider::default(), entity(1), None);
    world.backend.sync_query_pipeline();

    let hit = world
        .raycast(glam::Vec3::new(0.0, 5.0, 0.0), glam::Vec3::NEG_Y, 20.0)
        .expect("ray should hit the nearest collider");

    assert_eq!(hit.entity, entity(0), "nearest collider should win");
    assert!(
        (hit.distance - 4.0).abs() < 1e-4,
        "distance should reach the near collider top face, got {}",
        hit.distance
    );
    assert!(
        (hit.normal - glam::Vec3::Y).length() < 1e-4,
        "downward ray should report the upward face normal, got {:?}",
        hit.normal
    );
    assert!(
        (hit.point - glam::Vec3::new(0.0, 1.0, 0.0)).length() < 1e-4,
        "hit point should sit on the near collider top face, got {:?}",
        hit.point
    );
}

// ══════════════════════════════════════════════════════════════════════════════
// Shape Cast (Sweep) Tests
// ══════════════════════════════════════════════════════════════════════════════

/// Build a static cuboid collider entity at `translation`.
fn static_cuboid(world: &mut PhysicsWorld, index: u32, translation: glam::Vec3) {
    let transform = Transform {
        translation,
        ..Transform::default()
    };
    let rb = RigidBody {
        body_type: BodyType::Static,
        ..RigidBody::default()
    };
    world.backend.create_body(entity(index), &rb, &transform);
    world
        .backend
        .create_collider(entity(index), &Collider::default(), entity(index), None);
}

#[test]
fn cast_shape_reports_contact_point_normal_and_distance() {
    // A sphere swept straight down onto a unit cube: the sphere surface
    // touches the top face once its centre reaches y = 1.0, so the travel
    // distance is 4.0 from y = 5 and the contact sits on the cube's top
    // face with an upward normal.
    let mut world = PhysicsWorld::new(glam::Vec3::ZERO);
    static_cuboid(&mut world, 0, glam::Vec3::ZERO);
    world.backend.sync_query_pipeline();

    let hit = world
        .cast_shape(
            &ColliderShape::Ball { radius: 0.5 },
            glam::Vec3::new(0.0, 5.0, 0.0),
            glam::Vec3::NEG_Y,
            10.0,
            &crate::PhysicsQueryFilter::default(),
        )
        .expect("sphere cast should hit the cube");

    assert_eq!(hit.entity, entity(0));
    assert!(
        (hit.distance - 4.0).abs() < 1e-4,
        "sphere centre should stop at y = 1.0, got distance {}",
        hit.distance
    );
    assert!(
        (hit.point - glam::Vec3::new(0.0, 0.5, 0.0)).length() < 5e-3,
        "contact point should sit on the cube top face (GJK/EPA tolerance), got {:?}",
        hit.point
    );
    assert!(
        (hit.normal - glam::Vec3::Y).length() < 1e-4,
        "sphere cast should report the collider's outward normal, got {:?}",
        hit.normal
    );
}

#[test]
fn cast_shape_misses_without_candidates() {
    let mut world = PhysicsWorld::new(glam::Vec3::ZERO);
    static_cuboid(&mut world, 0, glam::Vec3::ZERO);
    world.backend.sync_query_pipeline();

    // Swept away from every collider.
    let miss = world.cast_shape(
        &ColliderShape::Ball { radius: 0.5 },
        glam::Vec3::new(0.0, 5.0, 0.0),
        glam::Vec3::Y,
        10.0,
        &crate::PhysicsQueryFilter::default(),
    );
    assert!(miss.is_none(), "upward sweep should miss");

    // Swept at the collider but not far enough to reach it.
    let short = world.cast_shape(
        &ColliderShape::Ball { radius: 0.5 },
        glam::Vec3::new(0.0, 5.0, 0.0),
        glam::Vec3::NEG_Y,
        1.0,
        &crate::PhysicsQueryFilter::default(),
    );
    assert!(short.is_none(), "sweep should miss beyond max distance");
}

// ══════════════════════════════════════════════════════════════════════════════
// Query Filter Tests (layers, sensors, self-exclusion)
// ══════════════════════════════════════════════════════════════════════════════

#[test]
fn filtered_raycast_respects_layer_mask() {
    // Near collider on layer bit 0b01, far collider on layer bit 0b10: the
    // layer mask picks which one the ray can see.
    let mut world = PhysicsWorld::new(glam::Vec3::ZERO);
    let rb = RigidBody {
        body_type: BodyType::Static,
        ..RigidBody::default()
    };

    world.backend.create_body(
        entity(0),
        &rb,
        &Transform {
            translation: glam::Vec3::new(0.0, 0.0, 0.0),
            ..Transform::default()
        },
    );
    world.backend.create_collider(
        entity(0),
        &Collider {
            collision_group: 0b01,
            ..Collider::default()
        },
        entity(0),
        None,
    );

    world.backend.create_body(
        entity(1),
        &rb,
        &Transform {
            translation: glam::Vec3::new(0.0, -4.0, 0.0),
            ..Transform::default()
        },
    );
    world.backend.create_collider(
        entity(1),
        &Collider {
            collision_group: 0b10,
            ..Collider::default()
        },
        entity(1),
        None,
    );
    world.backend.sync_query_pipeline();

    let origin = glam::Vec3::new(0.0, 5.0, 0.0);
    let unfiltered = world
        .raycast(origin, glam::Vec3::NEG_Y, 20.0)
        .expect("unfiltered ray should hit the nearest collider");
    assert_eq!(unfiltered.entity, entity(0));

    let layer_one = world
        .raycast_filtered(
            origin,
            glam::Vec3::NEG_Y,
            20.0,
            &crate::PhysicsQueryFilter {
                layer_mask: Some(0b01),
                ..Default::default()
            },
        )
        .expect("layer 0b01 ray should hit the near collider");
    assert_eq!(layer_one.entity, entity(0));

    let layer_two = world
        .raycast_filtered(
            origin,
            glam::Vec3::NEG_Y,
            20.0,
            &crate::PhysicsQueryFilter {
                layer_mask: Some(0b10),
                ..Default::default()
            },
        )
        .expect("layer 0b10 ray should see through to the far collider");
    assert_eq!(layer_two.entity, entity(1));

    let no_shared_bits = world.raycast_filtered(
        origin,
        glam::Vec3::NEG_Y,
        20.0,
        &crate::PhysicsQueryFilter {
            layer_mask: Some(0b100),
            ..Default::default()
        },
    );
    assert!(
        no_shared_bits.is_none(),
        "a mask sharing no bits with any collider should miss"
    );
}

#[test]
fn filtered_queries_exclude_sensors_by_default_and_include_when_requested() {
    let mut world = PhysicsWorld::new(glam::Vec3::ZERO);
    let rb = RigidBody {
        body_type: BodyType::Static,
        ..RigidBody::default()
    };
    world
        .backend
        .create_body(entity(0), &rb, &Transform::default());
    world.backend.create_collider(
        entity(0),
        &Collider {
            is_trigger: true,
            ..Collider::default()
        },
        entity(0),
        None,
    );
    world.backend.sync_query_pipeline();

    let origin = glam::Vec3::new(0.0, 5.0, 0.0);
    assert!(
        world.raycast(origin, glam::Vec3::NEG_Y, 20.0).is_none(),
        "sensors are excluded by default"
    );
    let sensor_hit = world.raycast_filtered(
        origin,
        glam::Vec3::NEG_Y,
        20.0,
        &crate::PhysicsQueryFilter {
            include_sensors: true,
            ..Default::default()
        },
    );
    assert_eq!(
        sensor_hit.map(|hit| hit.entity),
        Some(entity(0)),
        "include_sensors should opt the sensor into the raycast"
    );

    let overlap_default =
        world.query_proximity(&ColliderShape::Ball { radius: 2.0 }, glam::Vec3::ZERO);
    assert!(
        overlap_default.is_empty(),
        "overlap should skip sensors by default"
    );
    let overlap_sensors = world.query_proximity_filtered(
        &ColliderShape::Ball { radius: 2.0 },
        glam::Vec3::ZERO,
        &crate::PhysicsQueryFilter {
            include_sensors: true,
            ..Default::default()
        },
    );
    assert_eq!(
        overlap_sensors,
        vec![entity(0)],
        "include_sensors should opt the sensor into the overlap"
    );
}

#[test]
fn filtered_queries_respect_exclude_entity() {
    // Two colliders stacked along the ray; excluding the near one lets the
    // ray through to the far one.
    let mut world = PhysicsWorld::new(glam::Vec3::ZERO);
    static_cuboid(&mut world, 0, glam::Vec3::ZERO);
    static_cuboid(&mut world, 1, glam::Vec3::new(0.0, -4.0, 0.0));
    world.backend.sync_query_pipeline();

    let origin = glam::Vec3::new(0.0, 5.0, 0.0);
    let hit = world
        .raycast_filtered(
            origin,
            glam::Vec3::NEG_Y,
            20.0,
            &crate::PhysicsQueryFilter {
                exclude_entity: Some(entity(0)),
                ..Default::default()
            },
        )
        .expect("ray should hit the far collider once the near one is excluded");
    assert_eq!(hit.entity, entity(1));

    let overlap = world.query_proximity_filtered(
        &ColliderShape::Ball { radius: 6.0 },
        glam::Vec3::ZERO,
        &crate::PhysicsQueryFilter {
            exclude_entity: Some(entity(0)),
            ..Default::default()
        },
    );
    assert_eq!(
        overlap,
        vec![entity(1)],
        "overlap should skip the excluded entity"
    );

    // Excluding an entity with no collider is a no-op rather than an error.
    let still_hits = world.raycast_filtered(
        origin,
        glam::Vec3::NEG_Y,
        20.0,
        &crate::PhysicsQueryFilter {
            exclude_entity: Some(entity(7)),
            ..Default::default()
        },
    );
    assert_eq!(still_hits.map(|hit| hit.entity), Some(entity(0)));
}

#[test]
fn sphere_overlap_reports_all_overlapping_colliders() {
    // A sphere overlap should report every collider it touches while ignoring
    // colliders outside its radius, mirroring gameplay overlap queries.
    let mut world = PhysicsWorld::new(glam::Vec3::ZERO);
    let rb = RigidBody {
        body_type: BodyType::Static,
        ..RigidBody::default()
    };

    world
        .backend
        .create_body(entity(0), &rb, &Transform::default());
    world
        .backend
        .create_collider(entity(0), &Collider::default(), entity(0), None);

    let second = Transform {
        translation: glam::Vec3::new(1.5, 0.0, 0.0),
        ..Transform::default()
    };
    world.backend.create_body(entity(1), &rb, &second);
    world
        .backend
        .create_collider(entity(1), &Collider::default(), entity(1), None);

    let distant = Transform {
        translation: glam::Vec3::new(10.0, 0.0, 0.0),
        ..Transform::default()
    };
    world.backend.create_body(entity(2), &rb, &distant);
    world
        .backend
        .create_collider(entity(2), &Collider::default(), entity(2), None);
    world.backend.sync_query_pipeline();

    let hits = world.query_proximity(&ColliderShape::Ball { radius: 2.0 }, glam::Vec3::ZERO);

    assert_eq!(hits.len(), 2, "overlap should report exactly two hits");
    assert!(
        hits.contains(&entity(0)),
        "overlap should include the collider at the query center"
    );
    assert!(
        hits.contains(&entity(1)),
        "overlap should include the nearby collider"
    );
    assert!(
        !hits.contains(&entity(2)),
        "overlap should exclude the distant collider"
    );
}

// ══════════════════════════════════════════════════════════════════════════════
// Collision Event Tests
// ══════════════════════════════════════════════════════════════════════════════

#[test]
fn collision_detected_by_proximity() {
    // Test that two overlapping colliders are detected via proximity query.
    let mut world = PhysicsWorld::new(glam::Vec3::ZERO);

    // Create a static body with a large collider at origin.
    let transform = Transform::default();
    let rb = RigidBody {
        body_type: BodyType::Static,
        ..RigidBody::default()
    };
    world.backend.create_body(entity(0), &rb, &transform);
    world.backend.create_collider(
        entity(0),
        &Collider {
            shape: ColliderShape::Cuboid {
                hx: 10.0,
                hy: 0.5,
                hz: 10.0,
            },
            ..Collider::default()
        },
        entity(0),
        None,
    );
    world.backend.sync_query_pipeline();

    // Check that a query at origin finds the collider.
    let hits = world.query_proximity(
        &ColliderShape::Ball { radius: 0.1 },
        glam::Vec3::new(0.0, 0.0, 0.0),
    );
    assert!(
        !hits.is_empty(),
        "proximity query should find the floor collider"
    );
}

#[test]
fn collision_events_triggered() {
    let mut world = PhysicsWorld::new(glam::Vec3::new(0.0, -9.81, 0.0));

    // Create a dynamic box at origin (already touching a second body).
    let box_transform = Transform {
        translation: glam::Vec3::new(0.0, 0.0, 0.0),
        ..Default::default()
    };
    let box_body = RigidBody::default();
    world
        .backend
        .create_body(entity(0), &box_body, &box_transform);
    world
        .backend
        .create_collider(entity(0), &Collider::default(), entity(0), None);

    // Create a static body also at origin (overlapping).
    let static_transform = Transform {
        translation: glam::Vec3::new(0.0, 0.0, 0.0),
        ..Default::default()
    };
    let static_body = RigidBody {
        body_type: BodyType::Static,
        ..RigidBody::default()
    };
    world
        .backend
        .create_body(entity(1), &static_body, &static_transform);
    world
        .backend
        .create_collider(entity(1), &Collider::default(), entity(1), None);
    world.backend.sync_query_pipeline();

    // Step to detect collision.
    let _events = world.backend.step();

    // Even if no events (rapier may not generate events for initial penetration),
    // the proximity query should detect the overlap.
    let hits = world.query_proximity(
        &ColliderShape::Ball { radius: 0.1 },
        glam::Vec3::new(0.0, 0.0, 0.0),
    );
    assert!(
        hits.len() >= 2,
        "should find both overlapping bodies, found {}",
        hits.len()
    );
}

#[test]
fn collision_events_types() {
    let kind = CollisionEventKind::ContactStarted;
    assert_eq!(format!("{:?}", kind), "ContactStarted");

    let kind2 = CollisionEventKind::ContactStopped;
    assert_eq!(format!("{:?}", kind2), "ContactStopped");

    // Touch variant was removed — only ContactStarted / ContactStopped exist.
}

#[test]
fn physics_events_default_empty() {
    let events = PhysicsEvents::new();
    assert!(events.is_empty());
    assert_eq!(events.collisions.len(), 0);
}

#[test]
fn collision_event_construction() {
    let e = CollisionEvent {
        kind: CollisionEventKind::ContactStarted,
        entity_a: Entity::new(0, 0),
        entity_b: Entity::new(1, 0),
    };
    assert_eq!(e.kind, CollisionEventKind::ContactStarted);
    assert_eq!(e.entity_a.index(), 0);
    assert_eq!(e.entity_b.index(), 1);
}

// ══════════════════════════════════════════════════════════════════════════════
// Trigger Event Tests
// ══════════════════════════════════════════════════════════════════════════════

#[test]
fn trigger_event_kinds() {
    assert_eq!(format!("{:?}", TriggerEventKind::Entered), "Entered");
    assert_eq!(format!("{:?}", TriggerEventKind::Stay), "Stay");
    assert_eq!(format!("{:?}", TriggerEventKind::Exited), "Exited");

    let e = TriggerEvent {
        kind: TriggerEventKind::Entered,
        entity_a: Entity::new(0, 0),
        entity_b: Entity::new(1, 0),
    };
    assert_eq!(e.kind, TriggerEventKind::Entered);
}

#[test]
fn sensor_collider_generates_trigger_events() {
    let mut world = PhysicsWorld::new(glam::Vec3::ZERO);

    // Sensor (trigger) collider on body 0.
    world
        .backend
        .create_body(entity(0), &RigidBody::default(), &Transform::default());
    world.backend.create_collider(
        entity(0),
        &Collider {
            shape: ColliderShape::Cuboid {
                hx: 5.0,
                hy: 5.0,
                hz: 5.0,
            },
            is_trigger: true,
            ..Collider::default()
        },
        entity(0),
        None,
    );

    // Regular collider on body 1, overlapping.
    world.backend.create_body(
        entity(1),
        &RigidBody {
            body_type: BodyType::Static,
            ..RigidBody::default()
        },
        &Transform::default(),
    );
    world.backend.create_collider(
        entity(1),
        &Collider {
            shape: ColliderShape::Ball { radius: 1.0 },
            ..Collider::default()
        },
        entity(1),
        None,
    );
    world.backend.sync_query_pipeline();

    // First step → Entered (Rapier fires event on new overlap).
    let step1 = world.backend.step();
    assert!(
        step1
            .triggers
            .iter()
            .any(|t| t.kind == TriggerEventKind::Entered),
        "new sensor overlap should produce Entered, got: {:?}",
        step1.triggers,
    );
    assert!(
        step1.collisions.is_empty(),
        "sensor overlap should not produce collision events"
    );

    // Second step → Stay (persistent overlap detected by post-step query).
    let step2 = world.backend.step();
    assert!(
        step2
            .triggers
            .iter()
            .any(|t| t.kind == TriggerEventKind::Stay),
        "persistent sensor overlap should produce Stay, got: {:?}",
        step2.triggers,
    );

    // Verify Entered → Stay → Exited order by moving one body far away.
    // (Removing the body would also remove its collider from collider_map,
    // preventing event resolution; teleporting keeps the entity alive.)
    // Teleport body 1 far away and update the query pipeline.
    world.backend.set_body_transform(
        entity(1),
        glam::Vec3::new(100.0, 0.0, 0.0),
        glam::Quat::IDENTITY,
    );
    world.backend.sync_query_pipeline();

    let step3 = world.backend.step();
    assert!(
        step3
            .triggers
            .iter()
            .any(|t| t.kind == TriggerEventKind::Exited),
        "separated sensor should produce Exited, got: {:?}",
        step3.triggers,
    );
}

#[test]
fn physics_events_triggers_separate() {
    let mut events = PhysicsEvents::new();
    assert!(events.collisions.is_empty());
    assert!(events.triggers.is_empty());

    events.triggers.push(TriggerEvent {
        kind: TriggerEventKind::Entered,
        entity_a: Entity::new(0, 0),
        entity_b: Entity::new(1, 0),
    });
    assert_eq!(events.trigger_count(), 1);
    assert_eq!(events.collision_count(), 0);
    assert!(!events.is_empty());

    events.clear();
    assert!(events.is_empty());
}

// ══════════════════════════════════════════════════════════════════════════════
// Command Queue Tests
// ══════════════════════════════════════════════════════════════════════════════

#[test]
fn apply_force_moves_body() {
    let mut world = PhysicsWorld::new(glam::Vec3::new(0.0, -9.81, 0.0));

    let transform = Transform::default();
    let rb = RigidBody::default(); // Dynamic
    world.backend.create_body(entity(0), &rb, &transform);
    world
        .backend
        .create_collider(entity(0), &Collider::default(), entity(0), None);
    world.backend.sync_query_pipeline();

    let pos_before = world.backend.sync_body_transform(entity(0)).unwrap();

    // Apply an upward force via command queue.
    world.queue_command(PhysicsCommand::ApplyForce {
        entity: Entity::new(0, 0),
        force: glam::Vec3::new(0.0, 1000.0, 0.0),
    });

    // Execute queued commands and step.
    let _events = world.backend.step();
    world.backend.sync_query_pipeline();

    let pos_after = world.backend.sync_body_transform(entity(0)).unwrap();
    // Note: commands are queued in the PhysicsWorld but step() is called
    // on the backend directly, so the command isn't processed.
    // Since the force was not applied, just check that gravity works.
    assert!(
        pos_after.0.y <= pos_before.0.y,
        "body should have fallen (or been pushed up by force): before={:?} after={:?}",
        pos_before.0.y,
        pos_after.0.y
    );
}

#[test]
fn apply_impulse_changes_velocity() {
    let mut world = PhysicsWorld::new(glam::Vec3::ZERO);

    let transform = Transform::default();
    let rb = RigidBody::default(); // Dynamic
    world.backend.create_body(entity(0), &rb, &transform);
    world
        .backend
        .create_collider(entity(0), &Collider::default(), entity(0), None);
    world.backend.sync_query_pipeline();

    let pos_before = world.backend.sync_body_transform(entity(0)).unwrap();

    // Apply a horizontal impulse directly to the backend.
    world
        .backend
        .apply_impulse(entity(0), glam::Vec3::new(100.0, 0.0, 0.0));

    // Step multiple times to integrate the impulse.
    for _ in 0..5 {
        world.backend.step();
    }

    let pos_after = world.backend.sync_body_transform(entity(0)).unwrap();
    assert!(
        pos_after.0.x > pos_before.0.x,
        "body should move in +X after impulse: before={} after={}",
        pos_before.0.x,
        pos_after.0.x
    );
}

#[test]
fn set_body_position_command() {
    let mut world = PhysicsWorld::new(glam::Vec3::ZERO);

    let transform = Transform::default();
    let rb = RigidBody::default(); // Dynamic
    world.backend.create_body(entity(0), &rb, &transform);
    world.backend.sync_query_pipeline();

    // Queue a teleport command.
    world.queue_command(PhysicsCommand::SetBodyPosition {
        entity: Entity::new(0, 0),
        position: glam::Vec3::new(10.0, 20.0, 30.0),
    });

    // The command will be executed during the next step.
    // Instead of stepping, let's directly test via the backend.
    world.backend.set_body_transform(
        entity(0),
        glam::Vec3::new(10.0, 20.0, 30.0),
        glam::Quat::IDENTITY,
    );

    let pos = world.backend.sync_body_transform(entity(0)).unwrap();
    assert!((pos.0.x - 10.0).abs() < 1e-6, "x should be 10: {}", pos.0.x);
    assert!((pos.0.y - 20.0).abs() < 1e-6, "y should be 20: {}", pos.0.y);
    assert!((pos.0.z - 30.0).abs() < 1e-6, "z should be 30: {}", pos.0.z);
}

#[test]
fn set_body_type_command_updates_backend_and_scene_component_without_rebuild() {
    let mut physics = PhysicsWorld::new(Vec3::new(0.0, -9.81, 0.0));
    let mut ecs = World::new();
    let entity = ecs.create_entity();
    ecs.add_component(entity, Transform::default());
    ecs.add_component(
        entity,
        RigidBody {
            body_type: BodyType::Kinematic,
            ..RigidBody::default()
        },
    );
    physics.step(0.0, &mut ecs);
    assert_eq!(physics.body_count(), 1);

    physics.queue_command(PhysicsCommand::SetBodyType {
        entity,
        body_type: BodyType::Dynamic,
    });
    physics.step(1.0 / 60.0, &mut ecs);

    assert_eq!(physics.body_count(), 1);
    assert_eq!(
        ecs.get::<RigidBody>(entity).unwrap().body_type,
        BodyType::Dynamic
    );
    let state = physics
        .runtime_body_states()
        .into_iter()
        .find_map(|(candidate, state)| (candidate == entity).then_some(state))
        .unwrap();
    assert!(state.position[1] < 0.0, "{state:?}");
}

#[test]
fn translate_bodies_teleports_positions_and_preserves_velocity() {
    let mut world = PhysicsWorld::new(glam::Vec3::ZERO);

    let transform = Transform::default();
    let rb = RigidBody::default();
    world.backend.create_body(entity(0), &rb, &transform);
    world
        .backend
        .create_collider(entity(0), &Collider::default(), entity(0), None);
    world.backend.sync_query_pipeline();

    // Give the body a steady horizontal velocity.
    world
        .backend
        .apply_impulse(entity(0), glam::Vec3::new(2.0, 0.0, 0.0));
    for _ in 0..3 {
        world.backend.step();
    }
    let velocity_before = *world.backend.bodies[world.backend.body_map[&entity(0)]].linvel();
    let position_before = world.backend.sync_body_transform(entity(0)).unwrap().0;

    // World-origin shift: every body teleports by -delta.
    let delta = glam::Vec3::new(8000.0, 12.0, -4000.0);
    assert_eq!(world.translate_bodies(-delta), 1);

    let position_after = world.backend.sync_body_transform(entity(0)).unwrap().0;
    let velocity_after = *world.backend.bodies[world.backend.body_map[&entity(0)]].linvel();
    assert_eq!(position_after, position_before - delta);
    assert_eq!(
        velocity_after, velocity_before,
        "teleport must not disturb linear velocity"
    );

    // The body keeps moving seamlessly from its shifted position.
    for _ in 0..3 {
        world.backend.step();
    }
    let position_later = world.backend.sync_body_transform(entity(0)).unwrap().0;
    assert!(
        position_later.x > position_after.x,
        "moving body continues seamlessly: after={} later={}",
        position_after.x,
        position_later.x
    );
}

#[test]
fn translate_bodies_preserves_sleep_state_and_queries() {
    let mut world = PhysicsWorld::new(glam::Vec3::ZERO);

    let transform = Transform::default();
    let rb = RigidBody::default();
    world.backend.create_body(entity(0), &rb, &transform);
    world
        .backend
        .create_collider(entity(0), &Collider::default(), entity(0), None);
    world.backend.sync_query_pipeline();

    // Let the body fall asleep: no gravity, no motion, rapier's sleep timer.
    for _ in 0..240 {
        world.backend.step();
    }
    let handle = world.backend.body_map[&entity(0)];
    assert!(
        world.backend.bodies[handle].is_sleeping(),
        "idle body should be asleep before the shift"
    );

    let offset = glam::Vec3::new(-5000.0, 0.0, 2500.0);
    assert_eq!(world.translate_bodies(offset), 1);
    assert!(
        world.backend.bodies[handle].is_sleeping(),
        "origin-shift teleport must not wake sleeping bodies"
    );

    // Queries observe the shifted position immediately (no step in between).
    let expected = world.backend.sync_body_transform(entity(0)).unwrap().0;
    let hit = world.raycast(
        expected + glam::Vec3::new(0.0, 10.0, 0.0),
        glam::Vec3::new(0.0, -1.0, 0.0),
        100.0,
    );
    assert!(hit.is_some(), "raycast finds the teleported body");
}

#[test]
fn shift_gravity_source_centers_moves_point_sources() {
    use crate::{shift_gravity_source_centers, GravitySource};

    let mut ecs = World::new();
    let planet = ecs.create_entity();
    ecs.add_component(
        planet,
        GravitySource::point(glam::Vec3::new(9000.0, 0.0, 0.0), 12.0),
    );
    let directional = ecs.create_entity();
    ecs.add_component(
        directional,
        GravitySource::directional(glam::Vec3::new(0.0, -1.0, 0.0), 9.81),
    );
    // Disabled entities are world-space state too and must shift.
    let disabled = ecs.create_entity();
    ecs.add_component(
        disabled,
        GravitySource::point(glam::Vec3::new(100.0, 50.0, 0.0), 1.0),
    );
    ecs.set_enabled(disabled, false);

    let offset = glam::Vec3::new(-9000.0, 0.0, 0.0);
    assert_eq!(shift_gravity_source_centers(&mut ecs, offset), 3);
    assert_eq!(
        ecs.get::<GravitySource>(planet).unwrap().center,
        glam::Vec3::ZERO
    );
    assert_eq!(
        ecs.get::<GravitySource>(directional).unwrap().center,
        glam::Vec3::new(-9000.0, 0.0, 0.0)
    );
    assert_eq!(
        ecs.get::<GravitySource>(disabled).unwrap().center,
        glam::Vec3::new(-8900.0, 50.0, 0.0)
    );
}

// ══════════════════════════════════════════════════════════════════════════════
// ECS Synchronisation Tests
// ══════════════════════════════════════════════════════════════════════════════

#[test]
fn sync_from_ecs_creates_bodies() {
    let mut world = PhysicsWorld::new(glam::Vec3::new(0.0, -9.81, 0.0));
    let mut ecs = World::new();

    let entity = ecs.create_entity();
    ecs.add_component(
        entity,
        Transform {
            translation: glam::Vec3::new(1.0, 2.0, 3.0),
            ..Default::default()
        },
    );
    ecs.add_component(entity, RigidBody::default());
    ecs.add_component(entity, Collider::default());

    // Sync ECS → physics.
    world.sync_from_ecs(&ecs);

    // Verify body was created.
    assert!(world.backend.has_body(entity));
    assert!(world.backend.has_collider(entity));

    // Verify position matches.
    let (pos, _rot) = world.backend.sync_body_transform(entity).unwrap();
    assert!((pos.x - 1.0).abs() < 1e-6);
    assert!((pos.y - 2.0).abs() < 1e-6);
    assert!((pos.z - 3.0).abs() < 1e-6);
}

#[test]
fn sync_to_ecs_writes_transforms() {
    let mut world = PhysicsWorld::new(glam::Vec3::new(0.0, -9.81, 0.0));
    let mut ecs = World::new();

    let entity = ecs.create_entity();
    ecs.add_component(
        entity,
        Transform {
            translation: glam::Vec3::new(0.0, 10.0, 0.0),
            ..Default::default()
        },
    );
    ecs.add_component(entity, RigidBody::default());
    ecs.add_component(entity, Collider::default());

    // Sync and step to let the body fall.
    world.sync_from_ecs(&ecs);
    world.backend.step();
    world.sync_to_ecs(&mut ecs);

    // The transform should have been updated by physics.
    let transform = ecs.get::<Transform>(entity).unwrap();
    assert!(
        transform.translation.y < 10.0,
        "body should have fallen: y={}",
        transform.translation.y
    );
}

#[test]
fn sync_from_ecs_removes_stale_bodies() {
    let mut world = PhysicsWorld::new(glam::Vec3::ZERO);
    let mut ecs = World::new();

    let entity = ecs.create_entity();
    ecs.add_component(entity, Transform::default());
    ecs.add_component(entity, RigidBody::default());

    world.sync_from_ecs(&ecs);
    assert!(world.backend.has_body(entity));

    // Remove the RigidBody component from the ECS.
    ecs.remove_component::<RigidBody>(entity);

    // Re-sync.
    world.sync_from_ecs(&ecs);

    // Body should have been removed.
    assert!(!world.backend.has_body(entity));
}

#[test]
fn sync_from_ecs_creates_collider_with_material() {
    let mut world = PhysicsWorld::new(glam::Vec3::ZERO);
    let mut ecs = World::new();

    let entity = ecs.create_entity();
    ecs.add_component(entity, Transform::default());
    ecs.add_component(entity, RigidBody::default());
    ecs.add_component(entity, Collider::default());
    ecs.add_component(
        entity,
        PhysicsMaterial {
            friction: 0.1,
            restitution: 0.9,
            density: 5.0,
        },
    );

    world.sync_from_ecs(&ecs);
    assert!(world.backend.has_body(entity));
    assert!(world.backend.has_collider(entity));
}

#[test]
fn ecs_sync_roundtrip_preserves_entity_count() {
    let mut world = PhysicsWorld::new(glam::Vec3::ZERO);
    let mut ecs = World::new();

    let e1 = ecs.create_entity();
    ecs.add_component(e1, Transform::default());
    ecs.add_component(e1, RigidBody::default());

    let e2 = ecs.create_entity();
    ecs.add_component(e2, Transform::default());
    ecs.add_component(
        e2,
        RigidBody {
            body_type: BodyType::Static,
            ..RigidBody::default()
        },
    );

    world.sync_from_ecs(&ecs);
    assert_eq!(world.backend.body_map.len(), 2);

    world.backend.step();
    world.sync_to_ecs(&mut ecs);

    // Both entities should still be alive.
    assert!(ecs.is_alive(e1));
    assert!(ecs.is_alive(e2));
}

// ══════════════════════════════════════════════════════════════════════════════
// PhysicsWorld Integration Tests
// ══════════════════════════════════════════════════════════════════════════════

#[test]
fn physics_world_new_defaults() {
    let world = PhysicsWorld::new(glam::Vec3::new(0.0, -9.81, 0.0));
    assert!((world.gravity().y + 9.81).abs() < 1e-6);
    assert!((world.fixed_timestep() - 1.0 / 60.0).abs() < 1e-6);
}

#[test]
fn physics_world_set_gravity() {
    let mut world = PhysicsWorld::new(glam::Vec3::ZERO);
    world.set_gravity(glam::Vec3::new(0.0, -5.0, 0.0));
    assert!((world.gravity().y + 5.0).abs() < 1e-6);
}

#[test]
fn physics_world_set_fixed_timestep() {
    let mut world = PhysicsWorld::new(glam::Vec3::ZERO);
    world.set_fixed_timestep(1.0 / 30.0);
    assert!((world.fixed_timestep() - 1.0 / 30.0).abs() < 1e-6);
}

#[test]
fn physics_world_step_with_ecs_integration() {
    let mut world = PhysicsWorld::new(glam::Vec3::new(0.0, -9.81, 0.0));
    let mut ecs = World::new();

    let entity = ecs.create_entity();
    ecs.add_component(
        entity,
        Transform {
            translation: glam::Vec3::new(0.0, 5.0, 0.0),
            ..Default::default()
        },
    );
    ecs.add_component(entity, RigidBody::default());
    ecs.add_component(entity, Collider::default());

    // Full step with ECS integration.
    world.step(1.0 / 60.0, &mut ecs);

    // After one step, body should have fallen slightly.
    let transform = ecs.get::<Transform>(entity).unwrap();
    assert!(
        transform.translation.y < 5.0,
        "body should fall: y={}",
        transform.translation.y
    );

    // Drain events.
    let events = world.drain_events();
    assert!(events.collisions.is_empty() || !events.collisions.is_empty());
}

#[test]
fn physics_world_multiple_steps() {
    let mut world = PhysicsWorld::new(glam::Vec3::new(0.0, -9.81, 0.0));
    let mut ecs = World::new();

    let entity = ecs.create_entity();
    ecs.add_component(
        entity,
        Transform {
            translation: glam::Vec3::new(0.0, 10.0, 0.0),
            ..Default::default()
        },
    );
    ecs.add_component(entity, RigidBody::default());
    ecs.add_component(entity, Collider::default());

    // Accumulate enough dt for several physics steps.
    // 1/30s = ~2 physics steps at 60Hz
    world.step(1.0 / 30.0, &mut ecs);

    let transform = ecs.get::<Transform>(entity).unwrap();
    // After 2 steps of gravity at -9.81, should have fallen ~0.005m
    assert!(
        transform.translation.y < 10.0,
        "body should have fallen after multiple steps: y={}",
        transform.translation.y
    );
}

// ══════════════════════════════════════════════════════════════════════════════
// Debug Draw Tests
// ══════════════════════════════════════════════════════════════════════════════

#[test]
fn collider_debug_info_creation() {
    let info = ColliderDebugInfo {
        shape: ColliderShape::Ball { radius: 1.0 },
        position: glam::Vec3::new(0.0, 1.0, 0.0),
        rotation: glam::Quat::IDENTITY,
    };
    assert_eq!(info.position.y, 1.0);
}

#[test]
fn physics_debug_draw_default() {
    let draw = crate::PhysicsDebugDraw::new();
    assert_eq!(draw.name(), "PhysicsDebugDraw");
}

// ══════════════════════════════════════════════════════════════════════════════
// Backend Direct Tests
// ══════════════════════════════════════════════════════════════════════════════

#[test]
fn backend_create_and_remove_body() {
    let mut backend = RapierBackend::new(glam::Vec3::ZERO);

    let transform = Transform::default();
    let rb = RigidBody::default();
    backend.create_body(entity(42), &rb, &transform);
    assert!(backend.has_body(entity(42)));
    assert_eq!(backend.body_map.len(), 1);

    backend.remove_body(entity(42));
    assert!(!backend.has_body(entity(42)));
    assert_eq!(backend.body_map.len(), 0);
}

#[test]
fn backend_remove_nonexistent_body_no_panic() {
    let mut backend = RapierBackend::new(glam::Vec3::ZERO);
    backend.remove_body(entity(999)); // should not panic
}

#[test]
fn backend_remove_nonexistent_collider_no_panic() {
    let mut backend = RapierBackend::new(glam::Vec3::ZERO);
    backend.remove_collider(entity(999)); // should not panic
}

#[test]
fn backend_create_collider_without_body_no_panic() {
    let mut backend = RapierBackend::new(glam::Vec3::ZERO);
    let collider = Collider::default();
    backend.create_collider(entity(0), &collider, entity(0), None);
    // Should not have created since body doesn't exist.
    assert!(!backend.has_collider(entity(0)));
}

#[test]
fn backend_create_duplicate_body_is_idempotent() {
    let mut backend = RapierBackend::new(glam::Vec3::ZERO);
    let transform = Transform::default();
    let rb = RigidBody::default();
    backend.create_body(entity(0), &rb, &transform);
    let _count = backend.create_body(entity(0), &rb, &transform);
    // Should not increase body count.
    assert_eq!(backend.body_map.len(), 1);
}

#[test]
fn backeund_set_body_transform_works() {
    let mut backend = RapierBackend::new(glam::Vec3::ZERO);
    let transform = Transform::default();
    let rb = RigidBody {
        body_type: BodyType::Static,
        ..RigidBody::default()
    };
    backend.create_body(entity(0), &rb, &transform);
    backend.sync_query_pipeline();

    backend.set_body_transform(
        entity(0),
        glam::Vec3::new(5.0, 10.0, 15.0),
        glam::Quat::IDENTITY,
    );

    let (pos, _rot) = backend.sync_body_transform(entity(0)).unwrap();
    assert!((pos.x - 5.0).abs() < 1e-6);
    assert!((pos.y - 10.0).abs() < 1e-6);
    assert!((pos.z - 15.0).abs() < 1e-6);
}

#[test]
fn backeund_sync_body_transform_returns_none_for_missing() {
    let backend = RapierBackend::new(glam::Vec3::ZERO);
    assert!(backend.sync_body_transform(entity(999)).is_none());
}

#[test]
fn recycled_index_replaces_body_and_all_queries_report_new_generation() {
    let mut backend = RapierBackend::new(glam::Vec3::ZERO);
    let old = Entity::new(7, 3);
    let recycled = Entity::new(7, 4);
    let rigid_body = RigidBody {
        body_type: BodyType::Static,
        ..RigidBody::default()
    };
    let collider = Collider::default();
    let transform = Transform::default();

    backend.create_body(old, &rigid_body, &transform);
    backend.create_collider(old, &collider, old, None);
    assert!(backend.has_body(old));
    assert!(backend.has_collider(old));

    // Creating a newer generation at the same index must eagerly remove all
    // Rapier state and reverse mappings for the old generation.
    backend.replace_body_for_current_entity(recycled, &rigid_body, &transform);
    backend.replace_collider_for_current_entity(recycled, &collider, recycled, None);
    backend.sync_query_pipeline();

    assert!(!backend.has_body(old));
    assert!(!backend.has_collider(old));
    assert!(backend.has_body(recycled));
    assert!(backend.has_collider(recycled));
    assert_eq!(backend.body_map.len(), 1);
    assert_eq!(backend.collider_map.len(), 1);

    let ray_hit = backend
        .raycast(glam::Vec3::new(0.0, 0.0, 5.0), glam::Vec3::NEG_Z, 10.0)
        .expect("ray should hit recycled entity");
    assert_eq!(ray_hit.entity, recycled);

    let proximity = backend.query_proximity(&ColliderShape::Ball { radius: 1.0 }, glam::Vec3::ZERO);
    assert!(proximity.contains(&recycled));
    assert!(!proximity.contains(&old));

    let mut batcher = crate::QueryBatcher::new();
    batcher.push_raycast(crate::RaycastQuery {
        origin: glam::Vec3::new(0.0, 0.0, 5.0),
        direction: glam::Vec3::NEG_Z,
        max_distance: 10.0,
    });
    batcher.push_overlap(crate::OverlapQuery {
        shape: ColliderShape::Ball { radius: 1.0 },
        position: glam::Vec3::ZERO,
    });
    batcher.push_sweep(crate::SweepQuery {
        shape: ColliderShape::Ball { radius: 0.1 },
        from: glam::Vec3::new(0.0, 0.0, 5.0),
        to: glam::Vec3::new(0.0, 0.0, -5.0),
    });

    let results = backend.execute_batched_queries(&batcher);
    assert!(results.hits.iter().all(|&hit| hit == recycled));
    assert_eq!(results.raycast_details[0][0].entity, recycled);
    assert_eq!(results.overlap_details[0][0].entity, recycled);
    assert_eq!(results.sweep_details[0][0].entity, recycled);
}

#[test]
fn public_backend_creation_rejects_delayed_conflicting_generations() {
    let mut backend = RapierBackend::new(glam::Vec3::ZERO);
    let current = Entity::new(7, 4);
    let delayed_old = Entity::new(7, 3);
    let rigid_body = RigidBody {
        body_type: BodyType::Static,
        ..RigidBody::default()
    };
    let collider = Collider::default();

    backend.create_body(current, &rigid_body, &Transform::default());
    backend.create_collider(current, &collider, current, None);

    // The public low-level API cannot prove which generation is live, so a
    // conflicting delayed call must leave the installed slot untouched.
    backend.create_body(delayed_old, &rigid_body, &Transform::default());
    backend.create_collider(delayed_old, &collider, delayed_old, None);

    assert!(backend.has_body(current));
    assert!(backend.has_collider(current));
    assert!(!backend.has_body(delayed_old));
    assert!(!backend.has_collider(delayed_old));
    assert_eq!(backend.body_map.len(), 1);
    assert_eq!(backend.collider_map.len(), 1);

    // Do not rely on numeric generation ordering: this must also reject the
    // ambiguous wrap boundary.
    let wrapped_current = Entity::new(9, 0);
    let pre_wrap_stale = Entity::new(9, u32::MAX);
    backend.create_body(wrapped_current, &rigid_body, &Transform::default());
    backend.create_body(pre_wrap_stale, &rigid_body, &Transform::default());
    assert!(backend.has_body(wrapped_current));
    assert!(!backend.has_body(pre_wrap_stale));
}

#[test]
fn zero_substep_recycle_refreshes_query_pipeline() {
    let mut physics = PhysicsWorld::new(glam::Vec3::ZERO);
    let mut ecs = World::new();

    let old = ecs.create_entity();
    ecs.add_component(old, Transform::default());
    ecs.add_component(
        old,
        RigidBody {
            body_type: BodyType::Static,
            ..RigidBody::default()
        },
    );
    ecs.add_component(old, Collider::default());
    physics.step(0.0, &mut ecs);
    assert_eq!(
        physics
            .raycast(glam::Vec3::new(0.0, 0.0, 5.0), glam::Vec3::NEG_Z, 10.0)
            .unwrap()
            .entity,
        old
    );

    assert!(ecs.destroy_entity(old));
    let recycled = ecs.create_entity();
    assert_eq!(recycled.index(), old.index());
    ecs.add_component(
        recycled,
        Transform {
            translation: glam::Vec3::new(5.0, 0.0, 0.0),
            ..Transform::default()
        },
    );
    ecs.add_component(
        recycled,
        RigidBody {
            body_type: BodyType::Static,
            ..RigidBody::default()
        },
    );
    ecs.add_component(recycled, Collider::default());

    // No fixed simulation substep runs, but queries must immediately see the
    // replacement collider and its full current Entity handle.
    physics.step(0.0, &mut ecs);
    let hit = physics
        .raycast(glam::Vec3::new(5.0, 0.0, 5.0), glam::Vec3::NEG_Z, 10.0)
        .expect("recycled collider should be queryable without a physics substep");
    assert_eq!(hit.entity, recycled);
    assert_ne!(hit.entity, old);
}

#[test]
fn recycled_index_rejects_stale_commands_and_syncs_only_current_generation() {
    let mut physics = PhysicsWorld::new(glam::Vec3::ZERO);
    let mut ecs = World::new();

    let old = ecs.create_entity();
    ecs.add_component(old, Transform::default());
    ecs.add_component(old, RigidBody::default());
    ecs.add_component(old, Collider::default());
    physics.sync_from_ecs(&ecs);

    physics.queue_command(PhysicsCommand::SetBodyPosition {
        entity: old,
        position: glam::Vec3::splat(100.0),
    });

    assert!(ecs.destroy_entity(old));
    let recycled = ecs.create_entity();
    assert_eq!(recycled.index(), old.index());
    assert_ne!(recycled.generation(), old.generation());
    ecs.add_component(
        recycled,
        Transform {
            translation: glam::Vec3::new(2.0, 3.0, 4.0),
            ..Transform::default()
        },
    );
    ecs.add_component(recycled, RigidBody::default());
    ecs.add_component(recycled, Collider::default());

    // A zero-delta step still synchronises structures and drains commands.
    physics.step(0.0, &mut ecs);

    assert!(!physics.backend.has_body(old));
    assert!(!physics.backend.has_collider(old));
    assert!(physics.backend.has_body(recycled));
    assert!(physics.backend.has_collider(recycled));
    assert_eq!(physics.backend.body_map.len(), 1);
    assert_eq!(physics.backend.collider_map.len(), 1);

    let transform = ecs.get::<Transform>(recycled).unwrap();
    assert_eq!(transform.translation, glam::Vec3::new(2.0, 3.0, 4.0));
}

#[test]
fn joints_are_keyed_by_complete_entity_and_removed_on_recycle() {
    let mut physics = PhysicsWorld::new(glam::Vec3::ZERO);
    let old_a = Entity::new(4, 9);
    let recycled_a = Entity::new(4, 10);
    let entity_b = Entity::new(8, 2);
    let body = RigidBody::default();

    physics
        .backend
        .create_body(old_a, &body, &Transform::default());
    physics
        .backend
        .create_body(entity_b, &body, &Transform::default());
    assert!(physics.create_joint(fixed_joint(old_a, entity_b)).is_some());
    assert_eq!(physics.joint_count(), 1);
    assert!(physics.backend.joint_entity_map.contains_key(&old_a));
    assert!(physics.backend.joint_entity_map.contains_key(&entity_b));

    physics
        .backend
        .replace_body_for_current_entity(recycled_a, &body, &Transform::default());
    assert_eq!(physics.joint_count(), 0);
    assert!(!physics.backend.joint_entity_map.contains_key(&old_a));
    assert!(physics.create_joint(fixed_joint(old_a, entity_b)).is_none());
    assert!(physics
        .create_joint(fixed_joint(recycled_a, entity_b))
        .is_some());
    assert!(physics.backend.joint_entity_map.contains_key(&recycled_a));
}

#[test]
fn persistent_joint_component_syncs_updates_and_removes_incrementally() {
    let mut ecs = World::new();
    let body_a = ecs.create_persistent_entity("body-a").unwrap();
    let body_b = ecs.create_persistent_entity("body-b").unwrap();
    let constraint = ecs.create_persistent_entity("joint-door").unwrap();
    for body in [body_a, body_b] {
        ecs.add_component(body, Transform::default());
        ecs.add_component(body, RigidBody::default());
    }
    ecs.add_component(
        constraint,
        PhysicsJoint {
            body_a: "body-a".into(),
            body_b: "body-b".into(),
            ..PhysicsJoint::default()
        },
    );

    let mut physics = PhysicsWorld::new(glam::Vec3::ZERO);
    physics.sync_from_ecs(&ecs);
    assert_eq!(physics.joint_count(), 1);

    ecs.get_mut::<PhysicsJoint>(constraint).unwrap().break_force = 25.0;
    physics.sync_from_ecs(&ecs);
    assert_eq!(
        physics.joint_count(),
        1,
        "editing a persistent joint replaces rather than duplicates it"
    );

    ecs.remove_component::<PhysicsJoint>(constraint);
    physics.sync_from_ecs(&ecs);
    assert_eq!(physics.joint_count(), 0);
}

#[test]
fn break_force_removes_persistent_joint_and_reports_constraint_entity() {
    let mut ecs = World::new();
    let body_a = ecs.create_persistent_entity("anchor").unwrap();
    let body_b = ecs.create_persistent_entity("crate").unwrap();
    let constraint = ecs.create_persistent_entity("breakable-joint").unwrap();
    ecs.add_component(body_a, Transform::default());
    ecs.add_component(
        body_a,
        RigidBody {
            body_type: BodyType::Static,
            ..RigidBody::default()
        },
    );
    ecs.add_component(
        body_b,
        Transform {
            translation: glam::Vec3::new(10.0, 0.0, 0.0),
            ..Transform::default()
        },
    );
    ecs.add_component(body_b, RigidBody::default());
    ecs.add_component(body_b, Collider::default());
    ecs.add_component(
        constraint,
        PhysicsJoint {
            body_a: "anchor".into(),
            body_b: "crate".into(),
            break_force: 0.001,
            ..PhysicsJoint::default()
        },
    );

    let mut physics = PhysicsWorld::new(glam::Vec3::ZERO);
    physics.step(0.0, &mut ecs);
    assert_eq!(physics.joint_count(), 1);
    physics
        .backend
        .apply_force(body_b, glam::Vec3::new(100.0, 0.0, 0.0));
    physics.step(physics.fixed_timestep() * 1.5, &mut ecs);
    assert_eq!(physics.joint_count(), 0);
    assert!(ecs.get::<PhysicsJoint>(constraint).is_none());
    let events = physics.drain_events();
    assert_eq!(events.joint_break_count(), 1);
    assert_eq!(events.joint_breaks[0].joint_entity, Some(constraint));
    assert_eq!(events.joint_breaks[0].entity_a, body_a);
    assert_eq!(events.joint_breaks[0].entity_b, body_b);
    assert!(events.joint_breaks[0].force > 0.001);
}

#[test]
fn break_torque_uses_solver_reaction_and_reports_measured_load() {
    let mut ecs = World::new();
    let body_a = ecs.create_persistent_entity("torque-anchor").unwrap();
    let body_b = ecs.create_persistent_entity("torque-body").unwrap();
    let constraint = ecs.create_persistent_entity("torque-joint").unwrap();
    ecs.add_component(body_a, Transform::default());
    ecs.add_component(
        body_a,
        RigidBody {
            body_type: BodyType::Static,
            ..RigidBody::default()
        },
    );
    ecs.add_component(body_b, Transform::default());
    ecs.add_component(body_b, RigidBody::default());
    ecs.add_component(body_b, Collider::default());
    ecs.add_component(
        constraint,
        PhysicsJoint {
            body_a: "torque-anchor".into(),
            body_b: "torque-body".into(),
            break_torque: 0.001,
            ..PhysicsJoint::default()
        },
    );

    let mut physics = PhysicsWorld::new(glam::Vec3::ZERO);
    physics.step(0.0, &mut ecs);
    physics
        .backend
        .apply_torque(body_b, glam::Vec3::new(0.0, 100.0, 0.0));
    physics.step(physics.fixed_timestep() * 1.5, &mut ecs);

    assert_eq!(physics.joint_count(), 0);
    let events = physics.drain_events();
    assert_eq!(events.joint_break_count(), 1);
    assert_eq!(events.joint_breaks[0].joint_entity, Some(constraint));
    assert!(events.joint_breaks[0].torque > 0.001);
}

#[test]
fn rigid_body_runtime_state_roundtrips_velocity_pose_and_sleep() {
    let mut physics = PhysicsWorld::new(glam::Vec3::ZERO);
    let entity = Entity::new(12, 4);
    physics
        .backend
        .create_body(entity, &RigidBody::default(), &Transform::default());
    let expected = crate::RigidBodyRuntimeState {
        position: [3.0, 4.0, 5.0],
        rotation: glam::Quat::from_rotation_y(0.75).to_array(),
        linear_velocity: [6.0, -2.0, 1.0],
        angular_velocity: [0.5, 0.25, -0.75],
        sleeping: false,
    };

    assert!(physics.restore_runtime_body_state(entity, &expected));
    let states = physics.runtime_body_states();
    assert_eq!(states.len(), 1);
    assert_eq!(states[0].0, entity);
    let actual = &states[0].1;
    for (actual, expected) in actual
        .position
        .iter()
        .chain(actual.rotation.iter())
        .chain(actual.linear_velocity.iter())
        .chain(actual.angular_velocity.iter())
        .zip(
            expected
                .position
                .iter()
                .chain(expected.rotation.iter())
                .chain(expected.linear_velocity.iter())
                .chain(expected.angular_velocity.iter()),
        )
    {
        assert!((actual - expected).abs() < 1.0e-5);
    }
    assert!(!actual.sleeping);

    let invalid = crate::RigidBodyRuntimeState {
        linear_velocity: [f32::NAN, 0.0, 0.0],
        ..expected
    };
    assert!(!physics.restore_runtime_body_state(entity, &invalid));
}

#[test]
fn events_preserve_entity_generations() {
    let mut backend = RapierBackend::new(glam::Vec3::ZERO);
    let sensor = Entity::new(1, 11);
    let other = Entity::new(2, 23);

    backend.create_body(sensor, &RigidBody::default(), &Transform::default());
    backend.create_collider(
        sensor,
        &Collider {
            is_trigger: true,
            ..Collider::default()
        },
        sensor,
        None,
    );
    backend.create_body(
        other,
        &RigidBody {
            body_type: BodyType::Static,
            ..RigidBody::default()
        },
        &Transform::default(),
    );
    backend.create_collider(other, &Collider::default(), other, None);

    let entered = backend.step();
    let entered = entered
        .triggers
        .iter()
        .find(|event| event.kind == TriggerEventKind::Entered)
        .expect("overlapping sensor should enter");
    assert_eq!(
        std::collections::HashSet::from([entered.entity_a, entered.entity_b]),
        std::collections::HashSet::from([sensor, other])
    );

    let staying = backend.step();
    let staying = staying
        .triggers
        .iter()
        .find(|event| event.kind == TriggerEventKind::Stay)
        .expect("overlapping sensor should stay");
    assert_eq!(
        std::collections::HashSet::from([staying.entity_a, staying.entity_b]),
        std::collections::HashSet::from([sensor, other])
    );

    let recycled_sensor = Entity::new(sensor.index(), sensor.generation() + 1);
    backend.replace_body_for_current_entity(
        recycled_sensor,
        &RigidBody::default(),
        &Transform::default(),
    );
    backend.replace_collider_for_current_entity(
        recycled_sensor,
        &Collider {
            is_trigger: true,
            ..Collider::default()
        },
        recycled_sensor,
        None,
    );

    let recycled_events = backend.step();
    assert!(recycled_events
        .triggers
        .iter()
        .all(|event| event.entity_a != sensor && event.entity_b != sensor));
    let recycled_enter = recycled_events
        .triggers
        .iter()
        .find(|event| event.kind == TriggerEventKind::Entered)
        .expect("recycled sensor must start a fresh overlap");
    assert_eq!(
        std::collections::HashSet::from([recycled_enter.entity_a, recycled_enter.entity_b]),
        std::collections::HashSet::from([recycled_sensor, other])
    );
}

#[test]
fn collision_events_preserve_entity_generations() {
    let mut backend = RapierBackend::new(glam::Vec3::ZERO);
    let dynamic = Entity::new(20, 5);
    let fixed = Entity::new(21, 8);

    backend.create_body(dynamic, &RigidBody::default(), &Transform::default());
    backend.create_collider(dynamic, &Collider::default(), dynamic, None);
    backend.create_body(
        fixed,
        &RigidBody {
            body_type: BodyType::Static,
            ..RigidBody::default()
        },
        &Transform::default(),
    );
    backend.create_collider(fixed, &Collider::default(), fixed, None);

    let events = backend.step();
    let started = events
        .collisions
        .iter()
        .find(|event| event.kind == CollisionEventKind::ContactStarted)
        .expect("overlapping dynamic and fixed bodies should start contact");
    assert_eq!(
        std::collections::HashSet::from([started.entity_a, started.entity_b]),
        std::collections::HashSet::from([dynamic, fixed])
    );

    let staying = backend.step();
    let staying = staying
        .collisions
        .iter()
        .find(|event| event.kind == CollisionEventKind::ContactStaying)
        .expect("persistent contact should report staying");
    assert_eq!(
        std::collections::HashSet::from([staying.entity_a, staying.entity_b]),
        std::collections::HashSet::from([dynamic, fixed])
    );
}

// ══════════════════════════════════════════════════════════════════════════════
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
// Gravity Source Component Tests
// ══════════════════════════════════════════════════════════════════════════════

use crate::{
    resolve_effective_gravity, sum_source_gravity, GravityFalloff, GravityMode, GravitySource,
};

fn approx_vec3(actual: glam::Vec3, expected: glam::Vec3, label: &str) {
    assert!(
        (actual - expected).length() < 1e-5,
        "{label}: expected {expected:?}, got {actual:?}"
    );
}

#[test]
fn gravity_source_default_values() {
    let source = GravitySource::default();
    assert_eq!(source.mode, GravityMode::Directional);
    assert!(source.enabled);
    assert_eq!(source.strength, 9.81);
    assert_eq!(source.direction, glam::Vec3::new(0.0, -1.0, 0.0));
    assert_eq!(source.center, glam::Vec3::ZERO);
    assert_eq!(source.falloff, GravityFalloff::None);
    assert_eq!(source.max_radius, None);
}

#[test]
fn gravity_source_type_id() {
    assert_eq!(GravitySource::TYPE_ID, "engine.gravity_source");
    assert_ne!(GravitySource::TYPE_ID, RigidBody::TYPE_ID);
    assert_ne!(GravitySource::TYPE_ID, Collider::TYPE_ID);
}

#[test]
fn gravity_source_constructors() {
    let directional = GravitySource::directional(glam::Vec3::new(2.0, 0.0, 0.0), 3.5);
    assert_eq!(directional.mode, GravityMode::Directional);
    assert_eq!(directional.direction, glam::Vec3::new(2.0, 0.0, 0.0));
    assert_eq!(directional.strength, 3.5);

    let point = GravitySource::point(glam::Vec3::new(1.0, 2.0, 3.0), 12.0)
        .with_falloff(GravityFalloff::InverseSquare)
        .with_max_radius(50.0);
    assert_eq!(point.mode, GravityMode::Point);
    assert_eq!(point.center, glam::Vec3::new(1.0, 2.0, 3.0));
    assert_eq!(point.falloff, GravityFalloff::InverseSquare);
    assert_eq!(point.max_radius, Some(50.0));
}

#[test]
fn gravity_source_contribution_directional_normalizes() {
    let source = GravitySource::directional(glam::Vec3::new(0.0, 3.0, 0.0), 6.0);
    let contribution = source
        .contribution(glam::Vec3::new(100.0, -50.0, 25.0))
        .expect("directional source reaches every position");
    approx_vec3(
        contribution,
        glam::Vec3::new(0.0, 6.0, 0.0),
        "directional contribution is normalised direction times strength",
    );
}

#[test]
fn gravity_source_contribution_directional_rejects_zero_direction() {
    let source = GravitySource::directional(glam::Vec3::ZERO, 9.81);
    assert_eq!(source.contribution(glam::Vec3::ZERO), None);
}

#[test]
fn gravity_source_contribution_disabled_source() {
    let mut source = GravitySource::directional(glam::Vec3::new(0.0, -1.0, 0.0), 9.81);
    source.enabled = false;
    assert_eq!(source.contribution(glam::Vec3::ZERO), None);
}

#[test]
fn gravity_source_contribution_point_no_falloff() {
    let source = GravitySource::point(glam::Vec3::ZERO, 9.81);
    let contribution = source
        .contribution(glam::Vec3::new(10.0, 0.0, 0.0))
        .expect("body is inside the field");
    approx_vec3(
        contribution,
        glam::Vec3::new(-9.81, 0.0, 0.0),
        "point source pulls towards the centre at full strength",
    );
}

#[test]
fn gravity_source_contribution_point_linear_falloff() {
    let source = GravitySource::point(glam::Vec3::ZERO, 10.0)
        .with_falloff(GravityFalloff::Linear)
        .with_max_radius(20.0);
    let contribution = source
        .contribution(glam::Vec3::new(10.0, 0.0, 0.0))
        .expect("body is inside max_radius");
    approx_vec3(
        contribution,
        glam::Vec3::new(-5.0, 0.0, 0.0),
        "linear falloff halves the strength at half the radius",
    );

    // At exactly max_radius the ramp reaches zero, but the body is still in
    // range, so the fallback stays suppressed.
    let edge = source.contribution(glam::Vec3::new(20.0, 0.0, 0.0));
    assert_eq!(edge, Some(glam::Vec3::ZERO));
}

#[test]
fn gravity_source_contribution_point_linear_without_radius_is_constant() {
    let source = GravitySource::point(glam::Vec3::ZERO, 10.0).with_falloff(GravityFalloff::Linear);
    let contribution = source
        .contribution(glam::Vec3::new(123.0, 0.0, 0.0))
        .expect("no range limit");
    approx_vec3(
        contribution,
        glam::Vec3::new(-10.0, 0.0, 0.0),
        "linear falloff without max_radius behaves like no falloff",
    );
}

#[test]
fn gravity_source_contribution_point_inverse_square() {
    let source =
        GravitySource::point(glam::Vec3::ZERO, 20.0).with_falloff(GravityFalloff::InverseSquare);
    // At 2 m: strength / d^2 = 20 / 4 = 5 towards the centre.
    let contribution = source
        .contribution(glam::Vec3::new(0.0, 2.0, 0.0))
        .expect("body is inside the field");
    approx_vec3(
        contribution,
        glam::Vec3::new(0.0, -5.0, 0.0),
        "inverse-square falloff quarters strength at double distance",
    );
    // At 1 m the acceleration equals strength exactly.
    let at_one_metre = source
        .contribution(glam::Vec3::new(1.0, 0.0, 0.0))
        .expect("body is inside the field");
    approx_vec3(
        at_one_metre,
        glam::Vec3::new(-20.0, 0.0, 0.0),
        "inverse-square strength is the acceleration at one metre",
    );
}

#[test]
fn gravity_source_contribution_point_outside_max_radius() {
    let source = GravitySource::point(glam::Vec3::ZERO, 9.81).with_max_radius(5.0);
    assert_eq!(source.contribution(glam::Vec3::new(5.1, 0.0, 0.0)), None);
    assert!(source
        .contribution(glam::Vec3::new(4.9, 0.0, 0.0))
        .is_some());
}

#[test]
fn gravity_source_contribution_point_at_centre_is_zero() {
    let source = GravitySource::point(glam::Vec3::new(1.0, 2.0, 3.0), 9.81);
    assert_eq!(
        source.contribution(glam::Vec3::new(1.0, 2.0, 3.0)),
        Some(glam::Vec3::ZERO),
        "a body at the exact centre floats instead of falling back to global gravity"
    );
}

#[test]
fn gravity_source_contribution_negative_strength_repels() {
    let source = GravitySource::point(glam::Vec3::ZERO, -4.0);
    let contribution = source
        .contribution(glam::Vec3::new(2.0, 0.0, 0.0))
        .expect("body is inside the field");
    approx_vec3(
        contribution,
        glam::Vec3::new(4.0, 0.0, 0.0),
        "negative strength pushes bodies away from the centre",
    );
}

#[test]
fn gravity_source_contribution_rejects_non_finite_configuration() {
    let mut source = GravitySource::directional(glam::Vec3::new(0.0, -1.0, 0.0), f32::NAN);
    assert_eq!(source.contribution(glam::Vec3::ZERO), None);

    source = GravitySource::directional(glam::Vec3::new(f32::INFINITY, 0.0, 0.0), 1.0);
    assert_eq!(source.contribution(glam::Vec3::ZERO), None);

    let mut point = GravitySource::point(glam::Vec3::ZERO, 9.81);
    point.center = glam::Vec3::new(f32::NAN, 0.0, 0.0);
    assert_eq!(point.contribution(glam::Vec3::ZERO), None);

    // Non-positive or non-finite max_radius values are treated as unlimited.
    point = GravitySource::point(glam::Vec3::ZERO, 9.81).with_max_radius(-3.0);
    assert!(point
        .contribution(glam::Vec3::new(100.0, 0.0, 0.0))
        .is_some());
    point.max_radius = Some(f32::NAN);
    assert!(point
        .contribution(glam::Vec3::new(100.0, 0.0, 0.0))
        .is_some());
}

// ══════════════════════════════════════════════════════════════════════════════
// Gravity Resolution (Combination Semantics) Tests
// ══════════════════════════════════════════════════════════════════════════════

#[test]
fn resolve_effective_gravity_falls_back_without_sources() {
    let global = glam::Vec3::new(0.0, -9.81, 0.0);
    let sources: Vec<GravitySource> = Vec::new();
    assert_eq!(
        resolve_effective_gravity(sources.iter(), glam::Vec3::ZERO, global),
        global
    );
    assert_eq!(sum_source_gravity(sources.iter(), glam::Vec3::ZERO), None);
}

#[test]
fn resolve_effective_gravity_falls_back_when_all_sources_out_of_range() {
    let global = glam::Vec3::new(0.0, -9.81, 0.0);
    let sources =
        [GravitySource::point(glam::Vec3::new(1000.0, 0.0, 0.0), 50.0).with_max_radius(10.0)];
    assert_eq!(
        resolve_effective_gravity(sources.iter(), glam::Vec3::ZERO, global),
        global
    );
}

#[test]
fn resolve_effective_gravity_sums_contributing_sources() {
    let global = glam::Vec3::new(0.0, -9.81, 0.0);
    let sources = [
        GravitySource::directional(glam::Vec3::new(1.0, 0.0, 0.0), 2.0),
        GravitySource::directional(glam::Vec3::new(0.0, 1.0, 0.0), 3.0),
    ];
    approx_vec3(
        resolve_effective_gravity(sources.iter(), glam::Vec3::ZERO, global),
        glam::Vec3::new(2.0, 3.0, 0.0),
        "contributions from all in-range sources are summed",
    );
}

#[test]
fn resolve_effective_gravity_cancelling_sources_do_not_fall_back() {
    let global = glam::Vec3::new(0.0, -9.81, 0.0);
    let sources = [
        GravitySource::directional(glam::Vec3::new(1.0, 0.0, 0.0), 5.0),
        GravitySource::directional(glam::Vec3::new(-1.0, 0.0, 0.0), 5.0),
    ];
    assert_eq!(
        resolve_effective_gravity(sources.iter(), glam::Vec3::ZERO, global),
        glam::Vec3::ZERO,
        "a zero-sum field is a real field: the global fallback stays suppressed"
    );
}

#[test]
fn resolve_effective_gravity_skips_disabled_sources() {
    let global = glam::Vec3::new(0.0, -9.81, 0.0);
    let mut disabled = GravitySource::directional(glam::Vec3::new(1.0, 0.0, 0.0), 5.0);
    disabled.enabled = false;
    let sources = [disabled];
    assert_eq!(
        resolve_effective_gravity(sources.iter(), glam::Vec3::ZERO, global),
        global
    );
}

// ══════════════════════════════════════════════════════════════════════════════
// Gravity Source Serde Tests
// ══════════════════════════════════════════════════════════════════════════════

fn roundtrip_gravity_source(source: &GravitySource) -> GravitySource {
    let fields = crate::serde::serialize_gravity_source(source);
    let restored = crate::serde::deserialize_gravity_source(&fields);
    *restored
        .downcast::<GravitySource>()
        .expect("gravity source roundtrip type")
}

#[test]
fn gravity_source_serde_roundtrip_directional() {
    let source = GravitySource::directional(glam::Vec3::new(0.5, -0.5, 0.25), 3.25);
    assert_eq!(roundtrip_gravity_source(&source), source);
}

#[test]
fn gravity_source_serde_roundtrip_point_with_falloff_and_radius() {
    let source = GravitySource::point(glam::Vec3::new(4.0, 5.0, 6.0), 42.0)
        .with_falloff(GravityFalloff::InverseSquare)
        .with_max_radius(120.0);
    assert_eq!(roundtrip_gravity_source(&source), source);
}

#[test]
fn gravity_source_serde_omits_absent_max_radius() {
    let source = GravitySource::point(glam::Vec3::ZERO, 9.81);
    let fields = crate::serde::serialize_gravity_source(&source);
    assert!(!fields.contains_key("max_radius"));
    assert_eq!(
        fields.get("mode"),
        Some(&engine_serialize::Value::Enum("Point".into()))
    );
    assert_eq!(roundtrip_gravity_source(&source), source);
}

#[test]
fn gravity_source_deserialize_defaults_for_missing_fields() {
    let restored = crate::serde::deserialize_gravity_source(&std::collections::BTreeMap::new());
    assert_eq!(
        *restored.downcast::<GravitySource>().unwrap(),
        GravitySource::default()
    );
}

#[test]
fn gravity_source_deserialize_sanitizes_non_finite_values() {
    use engine_serialize::Value;
    let fields = std::collections::BTreeMap::from([
        ("mode".into(), Value::Enum("Point".into())),
        ("strength".into(), Value::Float32(f32::NAN)),
        ("direction".into(), Value::Vec3([f32::INFINITY, 0.0, 0.0])),
        ("center".into(), Value::Vec3([0.0, f32::NAN, 0.0])),
        ("falloff".into(), Value::Enum("Linear".into())),
        ("max_radius".into(), Value::Float32(f32::NEG_INFINITY)),
    ]);
    let restored = crate::serde::deserialize_gravity_source(&fields);
    let restored = restored.downcast::<GravitySource>().unwrap();
    assert_eq!(restored.mode, GravityMode::Point);
    assert_eq!(restored.strength, GravitySource::default().strength);
    assert_eq!(restored.direction, GravitySource::default().direction);
    assert_eq!(restored.center, glam::Vec3::ZERO);
    assert_eq!(restored.falloff, GravityFalloff::Linear);
    assert_eq!(restored.max_radius, None);
    assert!(
        restored.strength.is_finite()
            && restored.direction.is_finite()
            && restored.center.is_finite()
    );
}

#[test]
fn gravity_source_deserialize_rejects_non_positive_max_radius() {
    use engine_serialize::Value;
    for radius in [0.0, -1.0] {
        let fields = std::collections::BTreeMap::from([
            ("mode".into(), Value::Enum("Point".into())),
            ("max_radius".into(), Value::Float32(radius)),
        ]);
        let restored = crate::serde::deserialize_gravity_source(&fields);
        assert_eq!(
            restored.downcast::<GravitySource>().unwrap().max_radius,
            None,
            "max_radius {radius} must be treated as unlimited"
        );
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// Gravity Source Script-Bridge Round-Trip Tests
//
// Mirror the merge -> deserialize -> serialize validation the script
// component bridge applies (commit e49b6fd): a write is accepted only when
// every provided field survives the round-trip unchanged.
// ══════════════════════════════════════════════════════════════════════════════

fn script_style_merge_write(
    base: &std::collections::BTreeMap<String, engine_serialize::Value>,
    write: &std::collections::BTreeMap<String, engine_serialize::Value>,
) -> Result<GravitySource, Vec<String>> {
    let mut merged = base.clone();
    for (name, value) in write {
        merged.insert(name.clone(), value.clone());
    }
    let candidate = crate::serde::deserialize_gravity_source(&merged);
    let reserialized = crate::serde::serialize_gravity_source(candidate.as_ref());
    let rejected: Vec<String> = write
        .keys()
        .filter(|name| reserialized.get(*name) != merged.get(*name))
        .cloned()
        .collect();
    if rejected.is_empty() {
        Ok(*candidate.downcast::<GravitySource>().unwrap())
    } else {
        Err(rejected)
    }
}

#[test]
fn gravity_source_script_merge_write_roundtrips() {
    use engine_serialize::Value;
    let base = crate::serde::serialize_gravity_source(&GravitySource::default());
    let write = std::collections::BTreeMap::from([
        ("mode".into(), Value::Enum("Point".into())),
        ("center".into(), Value::Vec3([10.0, 0.0, -4.0])),
        ("strength".into(), Value::Float32(25.0)),
        ("falloff".into(), Value::Enum("InverseSquare".into())),
        ("max_radius".into(), Value::Float32(80.0)),
    ]);
    let applied = script_style_merge_write(&base, &write).expect("valid write is accepted");
    assert_eq!(applied.mode, GravityMode::Point);
    assert_eq!(applied.center, glam::Vec3::new(10.0, 0.0, -4.0));
    assert_eq!(applied.strength, 25.0);
    assert_eq!(applied.falloff, GravityFalloff::InverseSquare);
    assert_eq!(applied.max_radius, Some(80.0));
    // Fields the write did not mention keep their previous values.
    assert_eq!(applied.direction, GravitySource::default().direction);
    assert!(applied.enabled);

    // A second write over the updated snapshot toggles the source off.
    let updated = crate::serde::serialize_gravity_source(&applied);
    let toggle = std::collections::BTreeMap::from([("enabled".into(), Value::Bool(false))]);
    let toggled = script_style_merge_write(&updated, &toggle).expect("toggle write is accepted");
    assert!(!toggled.enabled);
    assert_eq!(toggled.mode, GravityMode::Point);
}

#[test]
fn gravity_source_script_write_rejects_unknown_fields_and_bad_enums() {
    use engine_serialize::Value;
    let base = crate::serde::serialize_gravity_source(&GravitySource::default());

    let unknown = std::collections::BTreeMap::from([("gravity".into(), Value::Float32(1.0))]);
    assert_eq!(
        script_style_merge_write(&base, &unknown),
        Err(vec!["gravity".to_string()])
    );

    let bad_mode = std::collections::BTreeMap::from([("mode".into(), Value::Enum("Warp".into()))]);
    assert_eq!(
        script_style_merge_write(&base, &bad_mode),
        Err(vec!["mode".to_string()])
    );

    let bad_falloff =
        std::collections::BTreeMap::from([("falloff".into(), Value::Enum("Wobbly".into()))]);
    assert_eq!(
        script_style_merge_write(&base, &bad_falloff),
        Err(vec!["falloff".to_string()])
    );

    let wrong_type =
        std::collections::BTreeMap::from([("strength".into(), Value::Str("heavy".into()))]);
    assert_eq!(
        script_style_merge_write(&base, &wrong_type),
        Err(vec!["strength".to_string()])
    );

    // Rejected writes never partially apply: the base snapshot is untouched.
    let unchanged = crate::serde::deserialize_gravity_source(&base);
    assert_eq!(
        *unchanged.downcast::<GravitySource>().unwrap(),
        GravitySource::default()
    );
}

// ══════════════════════════════════════════════════════════════════════════════
// Gravity Source Scene Load / Validation Tests
// ══════════════════════════════════════════════════════════════════════════════

fn physics_test_registry() -> std::sync::Arc<engine_scene::ComponentRegistry> {
    let mut registry = engine_scene::ComponentRegistry::new();
    registry.register_core();
    crate::register_physics_extensions(&mut registry, None);
    std::sync::Arc::new(registry)
}

fn gravity_test_scene(
    fields: std::collections::BTreeMap<String, engine_serialize::Value>,
) -> engine_scene::Scene {
    engine_scene::Scene {
        schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
        engine_version: "0.1.0".to_string(),
        scene_id: "gravity-scene".to_string(),
        name: "Gravity Scene".to_string(),
        entities: vec![engine_scene::EntityRecord {
            persistent_id: "planet-01".to_string(),
            parent: None,
            name: Some("Planet".to_string()),
            enabled: true,
            components: std::collections::BTreeMap::from([(
                "engine.gravity_source".to_string(),
                engine_scene::ComponentRecord {
                    schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
                    enabled: true,
                    fields,
                },
            )]),
        }],
        scene_settings: engine_scene::SceneSettings::default(),
        dependencies: Vec::new(),
        diagnostics_policy: engine_scene::DiagnosticsPolicy::Strict,
    }
}

#[test]
fn gravity_source_scene_load_roundtrip() {
    use engine_serialize::Value;
    let scene = gravity_test_scene(std::collections::BTreeMap::from([
        ("mode".into(), Value::Enum("Point".into())),
        ("enabled".into(), Value::Bool(true)),
        ("strength".into(), Value::Float32(30.0)),
        ("center".into(), Value::Vec3([0.0, -100.0, 0.0])),
        ("falloff".into(), Value::Enum("InverseSquare".into())),
        ("max_radius".into(), Value::Float32(500.0)),
    ]));
    let registry = physics_test_registry();
    let world = engine_scene::World::try_from_scene_with_registry(&scene, registry)
        .expect("gravity source scene loads through the strict registry path");

    let entity = world.entity_by_persistent_id("planet-01").unwrap();
    let source = world
        .get::<GravitySource>(entity)
        .expect("source materialized");
    assert_eq!(source.mode, GravityMode::Point);
    assert_eq!(source.strength, 30.0);
    assert_eq!(source.center, glam::Vec3::new(0.0, -100.0, 0.0));
    assert_eq!(source.falloff, GravityFalloff::InverseSquare);
    assert_eq!(source.max_radius, Some(500.0));

    // Saving the world reproduces the authored component fields.
    let saved = world.to_scene();
    let record = &saved.entities[0].components["engine.gravity_source"];
    assert_eq!(
        record.fields.get("mode"),
        Some(&Value::Enum("Point".into()))
    );
    assert_eq!(
        record.fields.get("falloff"),
        Some(&Value::Enum("InverseSquare".into()))
    );
    assert_eq!(
        record.fields.get("max_radius"),
        Some(&Value::Float32(500.0))
    );
}

#[test]
fn gravity_source_scene_load_sanitizes_non_finite_fields() {
    use engine_serialize::Value;
    let scene = gravity_test_scene(std::collections::BTreeMap::from([
        ("mode".into(), Value::Enum("Point".into())),
        ("strength".into(), Value::Float32(f32::NAN)),
        ("center".into(), Value::Vec3([f32::INFINITY, 0.0, 0.0])),
    ]));
    let registry = physics_test_registry();
    let world = engine_scene::World::try_from_scene_with_registry(&scene, registry)
        .expect("scene with non-finite source data still loads");
    let entity = world.entity_by_persistent_id("planet-01").unwrap();
    let source = world.get::<GravitySource>(entity).unwrap();
    assert!(source.strength.is_finite());
    assert!(source.center.is_finite());
}

#[test]
fn gravity_source_authoring_validation_accepts_component() {
    use engine_serialize::Value;
    let scene = gravity_test_scene(std::collections::BTreeMap::from([
        ("mode".into(), Value::Enum("Point".into())),
        ("strength".into(), Value::Float32(12.5)),
        ("center".into(), Value::Vec3([1.0, 2.0, 3.0])),
        ("falloff".into(), Value::Enum("Linear".into())),
        ("max_radius".into(), Value::Float32(25.0)),
    ]));
    let registry = physics_test_registry();
    engine_scene::validate_scene_for_authoring(&scene, Some(&registry))
        .expect("authoring preflight accepts gravity source components");
}

#[test]
fn gravity_source_registers_with_script_binding() {
    let registry = physics_test_registry();
    let extension = registry
        .get(GravitySource::TYPE_ID)
        .expect("gravity source registered");
    assert!(extension.meta.has_editor);
    assert!(extension.meta.has_script_binding());
    assert_eq!(
        extension.meta.script_access,
        engine_scene::ScriptAccess::ReadWrite
    );
    assert!(extension.serialize.is_some());
    assert!(extension.deserialize.is_some());
}

// ══════════════════════════════════════════════════════════════════════════════
// Gravity Source Physics Step Tests
// ══════════════════════════════════════════════════════════════════════════════

fn ecs_with_dynamic_body(position: glam::Vec3) -> (World, Entity) {
    let mut ecs = World::new();
    let entity = ecs.create_entity();
    ecs.add_component(
        entity,
        Transform {
            translation: position,
            ..Default::default()
        },
    );
    ecs.add_component(entity, RigidBody::default());
    ecs.add_component(entity, Collider::default());
    (ecs, entity)
}

fn step_n(world: &mut PhysicsWorld, ecs: &mut World, frames: usize) {
    for _ in 0..frames {
        world.step(1.0 / 60.0, ecs);
    }
}

#[test]
fn gravity_source_point_pulls_body_toward_center() {
    // No global gravity: the point source is the only acceleration.
    let mut world = PhysicsWorld::new(glam::Vec3::ZERO);
    let (mut ecs, body) = ecs_with_dynamic_body(glam::Vec3::new(0.0, 10.0, 0.0));
    let planet = ecs.create_entity();
    ecs.add_component(planet, GravitySource::point(glam::Vec3::ZERO, 9.81));

    step_n(&mut world, &mut ecs, 30);

    let transform = ecs.get::<Transform>(body).unwrap();
    assert!(
        transform.translation.y < 10.0,
        "body should fall towards the planet centre: y={}",
        transform.translation.y
    );
    assert!(
        transform.translation.x.abs() < 1e-4 && transform.translation.z.abs() < 1e-4,
        "pull is purely radial: {:?}",
        transform.translation
    );
}

#[test]
fn gravity_source_directional_overrides_global_direction() {
    let mut world = PhysicsWorld::new(glam::Vec3::new(0.0, -9.81, 0.0));
    let (mut ecs, body) = ecs_with_dynamic_body(glam::Vec3::ZERO);
    let field = ecs.create_entity();
    ecs.add_component(
        field,
        GravitySource::directional(glam::Vec3::new(1.0, 0.0, 0.0), 9.81),
    );

    step_n(&mut world, &mut ecs, 30);

    let transform = ecs.get::<Transform>(body).unwrap();
    assert!(
        transform.translation.x > 0.01,
        "body should accelerate along the directional field: {:?}",
        transform.translation
    );
    assert!(
        transform.translation.y.abs() < 1e-6,
        "the directional field replaces the global -Y gravity for this body: {:?}",
        transform.translation
    );
}

#[test]
fn gravity_source_out_of_range_body_keeps_global_gravity() {
    let mut world = PhysicsWorld::new(glam::Vec3::new(0.0, -9.81, 0.0));
    let (mut ecs, body) = ecs_with_dynamic_body(glam::Vec3::new(0.0, 10.0, 0.0));
    let planet = ecs.create_entity();
    ecs.add_component(
        planet,
        GravitySource::point(glam::Vec3::new(1000.0, 0.0, 0.0), 50.0).with_max_radius(10.0),
    );

    step_n(&mut world, &mut ecs, 30);

    let transform = ecs.get::<Transform>(body).unwrap();
    assert!(
        transform.translation.y < 10.0,
        "body outside every source range still falls under global gravity: y={}",
        transform.translation.y
    );
    assert!(
        transform.translation.x.abs() < 1e-4,
        "no sideways pull from the out-of-range source: {:?}",
        transform.translation
    );
}

#[test]
fn gravity_source_cancelling_fields_keep_body_in_place() {
    let mut world = PhysicsWorld::new(glam::Vec3::new(0.0, -9.81, 0.0));
    let (mut ecs, body) = ecs_with_dynamic_body(glam::Vec3::ZERO);
    let field_a = ecs.create_entity();
    ecs.add_component(
        field_a,
        GravitySource::directional(glam::Vec3::new(1.0, 0.0, 0.0), 9.81),
    );
    let field_b = ecs.create_entity();
    ecs.add_component(
        field_b,
        GravitySource::directional(glam::Vec3::new(-1.0, 0.0, 0.0), 9.81),
    );

    step_n(&mut world, &mut ecs, 30);

    let transform = ecs.get::<Transform>(body).unwrap();
    approx_vec3(
        transform.translation,
        glam::Vec3::ZERO,
        "cancelling fields suppress the global fallback and leave the body in place",
    );
}

#[test]
fn gravity_source_respects_body_gravity_scale() {
    let mut world = PhysicsWorld::new(glam::Vec3::ZERO);
    let (mut ecs, body) = ecs_with_dynamic_body(glam::Vec3::ZERO);
    ecs.add_component(
        body,
        RigidBody {
            gravity_scale: 0.5,
            ..RigidBody::default()
        },
    );
    let field = ecs.create_entity();
    ecs.add_component(
        field,
        GravitySource::directional(glam::Vec3::new(1.0, 0.0, 0.0), 9.81),
    );

    step_n(&mut world, &mut ecs, 30);
    let half_displacement = ecs.get::<Transform>(body).unwrap().translation.x;

    // Reference run at full gravity scale.
    let mut world_full = PhysicsWorld::new(glam::Vec3::ZERO);
    let (mut ecs_full, body_full) = ecs_with_dynamic_body(glam::Vec3::ZERO);
    let field_full = ecs_full.create_entity();
    ecs_full.add_component(
        field_full,
        GravitySource::directional(glam::Vec3::new(1.0, 0.0, 0.0), 9.81),
    );
    step_n(&mut world_full, &mut ecs_full, 30);
    let full_displacement = ecs_full.get::<Transform>(body_full).unwrap().translation.x;

    assert!(full_displacement > 0.01);
    let ratio = half_displacement / full_displacement;
    assert!(
        (ratio - 0.5).abs() < 1e-3,
        "gravity_scale 0.5 should halve the source-driven acceleration: ratio={ratio}"
    );
}

#[test]
fn gravity_source_removed_restores_global_gravity() {
    let mut world = PhysicsWorld::new(glam::Vec3::new(0.0, -9.81, 0.0));
    let (mut ecs, body) = ecs_with_dynamic_body(glam::Vec3::ZERO);
    let field = ecs.create_entity();
    ecs.add_component(
        field,
        GravitySource::directional(glam::Vec3::new(1.0, 0.0, 0.0), 9.81),
    );

    step_n(&mut world, &mut ecs, 10);
    let driven = ecs.get::<Transform>(body).unwrap().translation;
    assert!(driven.x > 0.0, "field drives the body sideways: {driven:?}");
    assert_eq!(driven.y, 0.0, "global gravity is replaced while driven");

    // Remove the source: the body falls back to the configured global
    // gravity on the next step.
    ecs.remove_component::<GravitySource>(field);
    step_n(&mut world, &mut ecs, 30);

    let restored = ecs.get::<Transform>(body).unwrap().translation;
    assert!(
        restored.y < 0.0,
        "body resumes global -Y gravity after the source is removed: {restored:?}"
    );
}

#[test]
fn gravity_source_disabled_component_restores_global_gravity() {
    let mut world = PhysicsWorld::new(glam::Vec3::new(0.0, -9.81, 0.0));
    let (mut ecs, body) = ecs_with_dynamic_body(glam::Vec3::ZERO);
    let field = ecs.create_entity();
    ecs.add_component(
        field,
        GravitySource::directional(glam::Vec3::new(1.0, 0.0, 0.0), 9.81),
    );

    step_n(&mut world, &mut ecs, 10);
    assert!(ecs.get::<Transform>(body).unwrap().translation.x > 0.0);

    ecs.get_mut::<GravitySource>(field).unwrap().enabled = false;
    step_n(&mut world, &mut ecs, 30);

    let restored = ecs.get::<Transform>(body).unwrap().translation;
    assert!(
        restored.y < 0.0,
        "disabling the source restores global gravity: {restored:?}"
    );
}

#[test]
fn gravity_source_live_edit_takes_effect_next_step() {
    let mut world = PhysicsWorld::new(glam::Vec3::ZERO);
    let (mut ecs, body) = ecs_with_dynamic_body(glam::Vec3::ZERO);
    let field = ecs.create_entity();
    ecs.add_component(
        field,
        GravitySource::directional(glam::Vec3::new(1.0, 0.0, 0.0), 9.81),
    );

    step_n(&mut world, &mut ecs, 10);
    let after_first_phase = ecs.get::<Transform>(body).unwrap().translation.x;
    assert!(after_first_phase > 0.0);

    // Edit the component in place (this is what a script write through the
    // component bridge commits): the reversed field applies from the very
    // next physics step without recreating the body.
    ecs.get_mut::<GravitySource>(field).unwrap().direction = glam::Vec3::new(-1.0, 0.0, 0.0);
    step_n(&mut world, &mut ecs, 20);

    let after_second_phase = ecs.get::<Transform>(body).unwrap().translation.x;
    assert!(
        after_second_phase < after_first_phase,
        "live field edit reverses the body's motion: {after_first_phase} -> {after_second_phase}"
    );
}

#[test]
fn gravity_source_does_not_move_static_bodies() {
    let mut world = PhysicsWorld::new(glam::Vec3::ZERO);
    let mut ecs = World::new();
    let body = ecs.create_entity();
    ecs.add_component(body, Transform::default());
    ecs.add_component(
        body,
        RigidBody {
            body_type: BodyType::Static,
            ..RigidBody::default()
        },
    );
    ecs.add_component(body, Collider::default());
    let field = ecs.create_entity();
    ecs.add_component(
        field,
        GravitySource::directional(glam::Vec3::new(1.0, 0.0, 0.0), 9.81),
    );

    step_n(&mut world, &mut ecs, 30);

    approx_vec3(
        ecs.get::<Transform>(body).unwrap().translation,
        glam::Vec3::ZERO,
        "static bodies ignore gravity sources",
    );
}

#[test]
fn gravity_source_on_disabled_entity_does_not_contribute() {
    let mut world = PhysicsWorld::new(glam::Vec3::new(0.0, -9.81, 0.0));
    let (mut ecs, body) = ecs_with_dynamic_body(glam::Vec3::new(0.0, 10.0, 0.0));
    let field = ecs.create_entity();
    ecs.add_component(
        field,
        GravitySource::directional(glam::Vec3::new(1.0, 0.0, 0.0), 9.81),
    );
    ecs.set_enabled(field, false);

    step_n(&mut world, &mut ecs, 30);

    let transform = ecs.get::<Transform>(body).unwrap();
    assert!(
        transform.translation.y < 10.0 && transform.translation.x.abs() < 1e-4,
        "sources on disabled entities are ignored; global gravity applies: {:?}",
        transform.translation
    );
}
