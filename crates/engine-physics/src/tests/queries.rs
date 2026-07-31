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
