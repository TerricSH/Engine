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
