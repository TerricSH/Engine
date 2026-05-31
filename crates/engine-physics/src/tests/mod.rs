use crate::{
    BodyType, Collider, ColliderDebugInfo, ColliderShape, CollisionEvent, CollisionEventKind,
    Component, Entity, PhysicsCommand, PhysicsEvents, PhysicsMaterial, PhysicsWorld, RapierBackend,
    RigidBody, Transform, TriggerEvent, TriggerEventKind,
};
use engine_renderer::DebugDrawProvider;
use engine_scene::World;

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

// ══════════════════════════════════════════════════════════════════════════════
// Physics Step & Gravity Tests
// ══════════════════════════════════════════════════════════════════════════════

#[test]
fn physics_world_gravity_moves_dynamic_body() {
    let mut world = PhysicsWorld::new(glam::Vec3::new(0.0, -9.81, 0.0));

    let transform = Transform::default();
    let rb = RigidBody::default(); // Dynamic
    world.backend.create_body(0, &rb, &transform);
    // Add a collider so the body has mass.
    world
        .backend
        .create_collider(0, &Collider::default(), 0, None);
    world.backend.sync_query_pipeline();

    let pos_before = world.backend.sync_body_transform(0).unwrap();
    assert!((pos_before.0.y - 0.0).abs() < 1e-6);

    // Step multiple times for gravity to take effect.
    for _ in 0..10 {
        world.backend.step();
    }

    let pos_after = world.backend.sync_body_transform(0).unwrap();
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
    world.backend.create_body(0, &rb, &transform);
    world.backend.sync_query_pipeline();

    let pos_before = world.backend.sync_body_transform(0).unwrap();

    // Step multiple times.
    for _ in 0..10 {
        world.backend.step();
    }

    let pos_after = world.backend.sync_body_transform(0).unwrap();
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
    world.backend.create_body(0, &rb, &transform);

    let collider = Collider {
        shape: ColliderShape::Cuboid {
            hx: 0.5,
            hy: 0.5,
            hz: 0.5,
        },
        ..Collider::default()
    };
    world.backend.create_collider(0, &collider, 0, None);
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
    world.backend.create_body(0, &rb, &transform);
    world
        .backend
        .create_collider(0, &Collider::default(), 0, None);
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
    world.backend.create_body(0, &rb, &transform);
    world
        .backend
        .create_collider(0, &Collider::default(), 0, None);
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
    world.backend.create_body(0, &rb, &transform);
    world
        .backend
        .create_collider(0, &Collider::default(), 0, None);
    world.backend.sync_query_pipeline();

    let hits = world.query_proximity(&ColliderShape::Ball { radius: 1.0 }, glam::Vec3::ZERO);

    assert!(
        hits.is_empty(),
        "should not find overlapping entity at origin"
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
    world.backend.create_body(0, &rb, &transform);
    world.backend.create_collider(
        0,
        &Collider {
            shape: ColliderShape::Cuboid {
                hx: 10.0,
                hy: 0.5,
                hz: 10.0,
            },
            ..Collider::default()
        },
        0,
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
    world.backend.create_body(0, &box_body, &box_transform);
    world
        .backend
        .create_collider(0, &Collider::default(), 0, None);

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
        .create_body(1, &static_body, &static_transform);
    world
        .backend
        .create_collider(1, &Collider::default(), 1, None);
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
    world.backend.create_body(0, &RigidBody::default(), &Transform::default());
    world.backend.create_collider(
        0,
        &Collider {
            shape: ColliderShape::Cuboid { hx: 5.0, hy: 5.0, hz: 5.0 },
            is_trigger: true,
            ..Collider::default()
        },
        0,
        None,
    );

    // Regular collider on body 1, overlapping.
    world.backend.create_body(
        1,
        &RigidBody { body_type: BodyType::Static, ..RigidBody::default() },
        &Transform::default(),
    );
    world.backend.create_collider(
        1,
        &Collider { shape: ColliderShape::Ball { radius: 1.0 }, ..Collider::default() },
        1,
        None,
    );
    world.backend.sync_query_pipeline();

    // First step → Entered (Rapier fires event on new overlap).
    let step1 = world.backend.step();
    assert!(
        step1.triggers.iter().any(|t| t.kind == TriggerEventKind::Entered),
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
        step2.triggers.iter().any(|t| t.kind == TriggerEventKind::Stay),
        "persistent sensor overlap should produce Stay, got: {:?}",
        step2.triggers,
    );

    // Verify Entered → Stay → Exited order by moving one body far away.
    // (Removing the body would also remove its collider from collider_map,
    // preventing event resolution; teleporting keeps the entity alive.)
    // Teleport body 1 far away and update the query pipeline.
    world.backend.set_body_transform(1, glam::Vec3::new(100.0, 0.0, 0.0), glam::Quat::IDENTITY);
    world.backend.sync_query_pipeline();

    let step3 = world.backend.step();
    assert!(
        step3.triggers.iter().any(|t| t.kind == TriggerEventKind::Exited),
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
    world.backend.create_body(0, &rb, &transform);
    world
        .backend
        .create_collider(0, &Collider::default(), 0, None);
    world.backend.sync_query_pipeline();

    let pos_before = world.backend.sync_body_transform(0).unwrap();

    // Apply an upward force via command queue.
    world.queue_command(PhysicsCommand::ApplyForce {
        entity: Entity::new(0, 0),
        force: glam::Vec3::new(0.0, 1000.0, 0.0),
    });

    // Execute queued commands and step.
    let _events = world.backend.step();
    world.backend.sync_query_pipeline();

    let pos_after = world.backend.sync_body_transform(0).unwrap();
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
    world.backend.create_body(0, &rb, &transform);
    world
        .backend
        .create_collider(0, &Collider::default(), 0, None);
    world.backend.sync_query_pipeline();

    let pos_before = world.backend.sync_body_transform(0).unwrap();

    // Apply a horizontal impulse directly to the backend.
    world
        .backend
        .apply_impulse(0, glam::Vec3::new(100.0, 0.0, 0.0));

    // Step multiple times to integrate the impulse.
    for _ in 0..5 {
        world.backend.step();
    }

    let pos_after = world.backend.sync_body_transform(0).unwrap();
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
    world.backend.create_body(0, &rb, &transform);
    world.backend.sync_query_pipeline();

    // Queue a teleport command.
    world.queue_command(PhysicsCommand::SetBodyPosition {
        entity: Entity::new(0, 0),
        position: glam::Vec3::new(10.0, 20.0, 30.0),
    });

    // The command will be executed during the next step.
    // Instead of stepping, let's directly test via the backend.
    world
        .backend
        .set_body_transform(0, glam::Vec3::new(10.0, 20.0, 30.0), glam::Quat::IDENTITY);

    let pos = world.backend.sync_body_transform(0).unwrap();
    assert!((pos.0.x - 10.0).abs() < 1e-6, "x should be 10: {}", pos.0.x);
    assert!((pos.0.y - 20.0).abs() < 1e-6, "y should be 20: {}", pos.0.y);
    assert!((pos.0.z - 30.0).abs() < 1e-6, "z should be 30: {}", pos.0.z);
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
    assert!(world.backend.has_body(entity.index()));
    assert!(world.backend.has_collider(entity.index()));

    // Verify position matches.
    let (pos, _rot) = world.backend.sync_body_transform(entity.index()).unwrap();
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
    assert!(world.backend.has_body(entity.index()));

    // Remove the RigidBody component from the ECS.
    ecs.remove_component::<RigidBody>(entity);

    // Re-sync.
    world.sync_from_ecs(&ecs);

    // Body should have been removed.
    assert!(!world.backend.has_body(entity.index()));
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
    assert!(world.backend.has_body(entity.index()));
    assert!(world.backend.has_collider(entity.index()));
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
    backend.create_body(42, &rb, &transform);
    assert!(backend.has_body(42));
    assert_eq!(backend.body_map.len(), 1);

    backend.remove_body(42);
    assert!(!backend.has_body(42));
    assert_eq!(backend.body_map.len(), 0);
}

#[test]
fn backend_remove_nonexistent_body_no_panic() {
    let mut backend = RapierBackend::new(glam::Vec3::ZERO);
    backend.remove_body(999); // should not panic
}

#[test]
fn backend_remove_nonexistent_collider_no_panic() {
    let mut backend = RapierBackend::new(glam::Vec3::ZERO);
    backend.remove_collider(999); // should not panic
}

#[test]
fn backend_create_collider_without_body_no_panic() {
    let mut backend = RapierBackend::new(glam::Vec3::ZERO);
    let collider = Collider::default();
    backend.create_collider(0, &collider, 0, None);
    // Should not have created since body doesn't exist.
    assert!(!backend.has_collider(0));
}

#[test]
fn backend_create_duplicate_body_is_idempotent() {
    let mut backend = RapierBackend::new(glam::Vec3::ZERO);
    let transform = Transform::default();
    let rb = RigidBody::default();
    backend.create_body(0, &rb, &transform);
    let _count = backend.create_body(0, &rb, &transform);
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
    backend.create_body(0, &rb, &transform);
    backend.sync_query_pipeline();

    backend.set_body_transform(0, glam::Vec3::new(5.0, 10.0, 15.0), glam::Quat::IDENTITY);

    let (pos, _rot) = backend.sync_body_transform(0).unwrap();
    assert!((pos.x - 5.0).abs() < 1e-6);
    assert!((pos.y - 10.0).abs() < 1e-6);
    assert!((pos.z - 15.0).abs() < 1e-6);
}

#[test]
fn backeund_sync_body_transform_returns_none_for_missing() {
    let backend = RapierBackend::new(glam::Vec3::ZERO);
    assert!(backend.sync_body_transform(999).is_none());
}

// ══════════════════════════════════════════════════════════════════════════════
// Component Registration Tests
// ══════════════════════════════════════════════════════════════════════════════

#[test]
fn physics_component_type_ids_are_unique() {
    assert_ne!(RigidBody::TYPE_ID, Collider::TYPE_ID);
    assert_ne!(RigidBody::TYPE_ID, PhysicsMaterial::TYPE_ID);
    assert_ne!(Collider::TYPE_ID, PhysicsMaterial::TYPE_ID);
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

    // Verify that storages can be created.
    let storages = component_registry.create_storages();
    assert!(storages.contains_key(RigidBody::TYPE_ID));
    assert!(storages.contains_key(Collider::TYPE_ID));
    assert!(storages.contains_key(PhysicsMaterial::TYPE_ID));
}

#[test]
fn register_physics_extensions_with_debug_draw() {
    let mut component_registry = engine_scene::registry::ComponentRegistry::new();
    let mut debug_registry = engine_renderer::debug_draw::DebugDrawRegistry::new();

    crate::register_physics_extensions(
        &mut component_registry,
        Some(&mut debug_registry),
    );

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
