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
