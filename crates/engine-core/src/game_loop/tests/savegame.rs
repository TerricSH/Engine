#[cfg(test)]
mod savegame_tests {
    use std::collections::BTreeMap;

    use engine_scene::components::Transform;
    use engine_serialize::Value;

    use super::*;

    #[test]
    fn checkpoint_restores_live_scene_origin_and_project_state() {
        let mut game_loop = GameLoop::new(EngineConfig::default());
        game_loop.load_scene(engine_scene::sample_scene()).unwrap();
        game_loop
            .runtime
            .with_world_mut(|world| {
                let cube = world.entity_by_persistent_id("cube-01").unwrap();
                world.add_component(
                    cube,
                    Transform {
                        translation: Vec3::new(1010.0, 2.0, 3.0),
                        ..Transform::default()
                    },
                );
            })
            .unwrap();
        game_loop
            .shift_world_origin([1000.0, 0.0, 0.0])
            .expect("origin shift");
        let expected_relative = game_loop
            .runtime
            .with_world(|world| {
                let cube = world.entity_by_persistent_id("cube-01").unwrap();
                world.get::<Transform>(cube).unwrap().translation
            })
            .unwrap();
        let save = game_loop
            .capture_save_game(BTreeMap::from([
                ("chapter".into(), Value::UInt(4)),
                ("suit".into(), Value::Bool(true)),
            ]))
            .unwrap();

        game_loop
            .runtime
            .with_world_mut(|world| {
                let cube = world.entity_by_persistent_id("cube-01").unwrap();
                world.get_mut::<Transform>(cube).unwrap().translation = Vec3::splat(-99.0);
            })
            .unwrap();
        let report = game_loop.restore_save_game(save).unwrap();

        assert_eq!(game_loop.world_origin(), [1000.0, 0.0, 0.0]);
        assert_eq!(
            game_loop
                .runtime
                .with_world(|world| {
                    let cube = world.entity_by_persistent_id("cube-01").unwrap();
                    world.get::<Transform>(cube).unwrap().translation
                })
                .unwrap(),
            expected_relative
        );
        assert_eq!(report.custom_state["chapter"], Value::UInt(4));
        assert_eq!(report.custom_state["suit"], Value::Bool(true));
    }

    #[cfg(all(feature = "subsystem-physics", feature = "subsystem-gameplay"))]
    #[test]
    fn checkpoint_restores_transient_rigid_body_state_by_persistent_id() {
        let mut game_loop = GameLoop::new(EngineConfig::default());
        game_loop.load_scene(engine_scene::sample_scene()).unwrap();
        let cube = game_loop
            .runtime
            .with_world_mut(|world| {
                let cube = world.entity_by_persistent_id("cube-01").unwrap();
                world.add_component(cube, Transform::default());
                world.add_component(cube, engine_physics::RigidBody::default());
                world.add_component(cube, engine_physics::Collider::default());
                cube
            })
            .unwrap();
        game_loop.resync_physics_from_world();
        let expected = engine_physics::RigidBodyRuntimeState {
            position: [2.0, 3.0, 4.0],
            rotation: glam::Quat::from_rotation_y(0.5).to_array(),
            linear_velocity: [5.0, -1.0, 0.5],
            angular_velocity: [0.0, 2.0, 0.0],
            sleeping: false,
        };
        assert!(game_loop
            .physics
            .as_mut()
            .unwrap()
            .restore_runtime_body_state(cube, &expected));

        let save = game_loop.capture_save_game(BTreeMap::new()).unwrap();
        let report = game_loop.restore_save_game(save).unwrap();
        assert_eq!(report.restored_physics_bodies, 1);
        assert!(report.skipped_physics_bodies.is_empty());

        let restored = game_loop
            .physics
            .as_ref()
            .unwrap()
            .runtime_body_states()
            .into_iter()
            .find(|(entity, _)| {
                game_loop
                    .runtime
                    .with_world(|world| world.persistent_id(*entity) == Some("cube-01"))
                    == Some(true)
            })
            .expect("restored cube state")
            .1;
        assert_eq!(restored.linear_velocity, expected.linear_velocity);
        assert_eq!(restored.angular_velocity, expected.angular_velocity);
        assert_eq!(restored.position, expected.position);
    }

    #[cfg(all(feature = "subsystem-physics", feature = "subsystem-gameplay"))]
    #[test]
    fn checkpoint_rebuilds_persistent_joint_without_serializing_backend_handles() {
        use engine_physics::{BodyType, PhysicsJoint, RigidBody};

        let mut game_loop = GameLoop::new(EngineConfig::default());
        game_loop.load_scene(engine_scene::sample_scene()).unwrap();
        game_loop
            .runtime
            .with_world_mut(|world| {
                let cube = world.entity_by_persistent_id("cube-01").unwrap();
                let camera = world.entity_by_persistent_id("camera-main").unwrap();
                world.add_component(cube, Transform::default());
                world.add_component(cube, RigidBody::default());
                world.add_component(camera, Transform::default());
                world.add_component(
                    camera,
                    RigidBody {
                        body_type: BodyType::Static,
                        ..RigidBody::default()
                    },
                );
                let constraint = world.create_persistent_entity("save-tether").unwrap();
                world.add_component(
                    constraint,
                    PhysicsJoint {
                        body_a: "camera-main".into(),
                        body_b: "cube-01".into(),
                        break_force: 2500.0,
                        ..PhysicsJoint::default()
                    },
                );
            })
            .unwrap();
        game_loop.resync_physics_from_world();
        assert_eq!(game_loop.physics.as_ref().unwrap().joint_count(), 1);

        let save = game_loop.capture_save_game(BTreeMap::new()).unwrap();
        game_loop
            .runtime
            .with_world_mut(|world| {
                let constraint = world.entity_by_persistent_id("save-tether").unwrap();
                world.remove_component::<PhysicsJoint>(constraint);
            })
            .unwrap();
        game_loop.resync_physics_from_world();
        assert_eq!(game_loop.physics.as_ref().unwrap().joint_count(), 0);

        game_loop.restore_save_game(save).unwrap();
        assert_eq!(game_loop.physics.as_ref().unwrap().joint_count(), 1);
        game_loop
            .runtime
            .with_world(|world| {
                let constraint = world.entity_by_persistent_id("save-tether").unwrap();
                assert_eq!(
                    world.get::<PhysicsJoint>(constraint).unwrap().break_force,
                    2500.0
                );
            })
            .unwrap();
    }

    #[cfg(all(feature = "subsystem-physics", feature = "subsystem-gameplay"))]
    #[test]
    fn checkpoint_restores_destructible_health_and_break_state() {
        let mut game_loop = GameLoop::new(EngineConfig::default());
        game_loop.load_scene(engine_scene::sample_scene()).unwrap();
        game_loop
            .runtime
            .with_world_mut(|world| {
                let cube = world.entity_by_persistent_id("cube-01").unwrap();
                world.add_component(
                    cube,
                    engine_physics::Destructible {
                        max_health: 75.0,
                        health: 0.0,
                        minimum_damage: 4.0,
                        replacement_prefab: Some(engine_serialize::AssetId::new("crate-fracture")),
                        broken: true,
                        ..Default::default()
                    },
                );
            })
            .unwrap();

        let save = game_loop.capture_save_game(BTreeMap::new()).unwrap();
        game_loop
            .runtime
            .with_world_mut(|world| {
                let cube = world.entity_by_persistent_id("cube-01").unwrap();
                *world.get_mut::<engine_physics::Destructible>(cube).unwrap() =
                    engine_physics::Destructible::default();
            })
            .unwrap();
        game_loop.restore_save_game(save).unwrap();

        game_loop
            .runtime
            .with_world(|world| {
                let cube = world.entity_by_persistent_id("cube-01").unwrap();
                let destructible = world.get::<engine_physics::Destructible>(cube).unwrap();
                assert_eq!(destructible.max_health, 75.0);
                assert_eq!(destructible.health, 0.0);
                assert_eq!(destructible.minimum_damage, 4.0);
                assert_eq!(
                    destructible.replacement_prefab.as_ref().unwrap().id,
                    "crate-fracture"
                );
                assert!(destructible.broken);
            })
            .unwrap();
    }
}
