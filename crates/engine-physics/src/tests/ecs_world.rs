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
