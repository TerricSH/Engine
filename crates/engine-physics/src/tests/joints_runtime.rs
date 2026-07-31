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
