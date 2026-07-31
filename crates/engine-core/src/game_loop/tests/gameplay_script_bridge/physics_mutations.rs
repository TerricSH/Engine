#[test]
fn script_impulse_resolves_persistent_id_and_reaches_physics_step() {
    use engine_physics::{Collider, RigidBody};
    use engine_scene::components::Transform;

    let mut game_loop = GameLoop::new(EngineConfig::default());
    game_loop.load_scene(engine_scene::sample_scene()).unwrap();
    game_loop.runtime.with_world_mut(|world| {
        let cube = world.entity_by_persistent_id("cube-01").unwrap();
        world.add_component(cube, Transform::default());
        world.add_component(cube, RigidBody::default());
        world.add_component(cube, Collider::default());
    });
    game_loop.init_physics();

    let diagnostics = game_loop.runtime.apply_script_gameplay_commands(vec![
        engine_script::OwnedGameplayCommand {
            entity_id: "cube-01".into(),
            command: GameplayCommand::PhysicsMutation {
                mutation: engine_script::GameplayPhysicsMutation::ApplyImpulse {
                    entity_id: "cube-01".into(),
                    impulse: [12.0, 0.0, 0.0],
                },
            },
        },
    ]);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    game_loop.queue_script_physics_mutations();
    game_loop.update(1.0 / 60.0);

    let cube = game_loop
        .runtime
        .with_world(|world| world.entity_by_persistent_id("cube-01").unwrap())
        .unwrap();
    let state = game_loop
        .physics
        .as_ref()
        .unwrap()
        .runtime_body_states()
        .into_iter()
        .find(|(entity, _)| *entity == cube)
        .unwrap()
        .1;
    assert!(state.linear_velocity[0] > 0.0, "{state:?}");
}

#[test]
fn script_joint_mutations_create_update_and_remove_a_persistent_constraint() {
    use engine_physics::{BodyType, Collider, PhysicsJoint, RigidBody};
    use engine_scene::components::Transform;
    use engine_script::{GameplayJointLimits, GameplayJointType, GameplayPhysicsMutation};

    let mut game_loop = GameLoop::new(EngineConfig::default());
    game_loop.load_scene(engine_scene::sample_scene()).unwrap();
    game_loop.runtime.with_world_mut(|world| {
        let cube = world.entity_by_persistent_id("cube-01").unwrap();
        let camera = world.entity_by_persistent_id("camera-main").unwrap();
        world.add_component(cube, Transform::default());
        world.add_component(cube, RigidBody::default());
        world.add_component(cube, Collider::default());
        world.add_component(camera, Transform::default());
        world.add_component(
            camera,
            RigidBody {
                body_type: BodyType::Static,
                ..RigidBody::default()
            },
        );
    });
    game_loop.init_physics();

    let create = |max: f32, break_force: f32| GameplayCommand::PhysicsMutation {
        mutation: GameplayPhysicsMutation::CreateJoint {
            joint_id: "script-hinge".into(),
            body_a: "camera-main".into(),
            body_b: "cube-01".into(),
            joint_type: GameplayJointType::Revolute,
            anchor_a: [0.0; 3],
            anchor_b: [0.0; 3],
            axis: [0.0, 1.0, 0.0],
            limits: Some(GameplayJointLimits {
                min: -max,
                max,
                stiffness: 20.0,
                damping: 2.0,
            }),
            motor: None,
            break_force,
            break_torque: 0.0,
        },
    };

    for command in [create(1.0, 1000.0), create(0.5, 500.0)] {
        let diagnostics = game_loop.runtime.apply_script_gameplay_commands(vec![
            engine_script::OwnedGameplayCommand {
                entity_id: "cube-01".into(),
                command,
            },
        ]);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        game_loop.queue_script_physics_mutations();
        game_loop.update(0.0);
        assert_eq!(game_loop.physics.as_ref().unwrap().joint_count(), 1);
    }

    game_loop
        .runtime
        .with_world(|world| {
            let constraint = world.entity_by_persistent_id("script-hinge").unwrap();
            let joint = world.get::<PhysicsJoint>(constraint).unwrap();
            assert_eq!(joint.break_force, 500.0);
            assert_eq!(joint.limits.as_ref().unwrap().max, 0.5);
        })
        .unwrap();

    let diagnostics = game_loop.runtime.apply_script_gameplay_commands(vec![
        engine_script::OwnedGameplayCommand {
            entity_id: "cube-01".into(),
            command: GameplayCommand::PhysicsMutation {
                mutation: GameplayPhysicsMutation::RemoveJoint {
                    joint_id: "script-hinge".into(),
                },
            },
        },
    ]);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    game_loop.queue_script_physics_mutations();
    game_loop.update(0.0);
    assert_eq!(game_loop.physics.as_ref().unwrap().joint_count(), 0);
    game_loop
        .runtime
        .with_world(|world| {
            let constraint = world.entity_by_persistent_id("script-hinge").unwrap();
            assert!(world.get::<PhysicsJoint>(constraint).is_none());
        })
        .unwrap();
}
