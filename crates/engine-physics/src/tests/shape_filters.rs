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
