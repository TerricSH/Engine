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

#[test]
fn collider_shape_trimesh_round_trips_and_builds_rapier_shape() {
    let shape = ColliderShape::TriMesh {
        vertices: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        indices: vec![[0, 1, 2]],
    };
    let json = serde_json::to_string(&shape).unwrap();
    let back: ColliderShape = serde_json::from_str(&json).unwrap();
    assert_eq!(back, shape);
    assert!(crate::backend::to_rapier_shared_shape(&back).is_some());

    let collider = Collider {
        shape: shape.clone(),
        ..Collider::default()
    };
    let fields = crate::serde::serialize_collider(&collider);
    let restored = crate::serde::deserialize_collider(&fields)
        .downcast::<Collider>()
        .expect("collider component");
    assert_eq!(restored.shape, shape);

    let invalid = ColliderShape::TriMesh {
        vertices: vec![[0.0; 3]; 3],
        indices: vec![[0, 1, 3]],
    };
    assert!(crate::backend::to_rapier_shared_shape(&invalid).is_none());
}

// ══════════════════════════════════════════════════════════════════════════════
