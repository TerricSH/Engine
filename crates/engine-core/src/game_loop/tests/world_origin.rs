#[cfg(test)]
mod world_origin_tests {
    use engine_scene::components::Transform;

    use super::*;

    /// Load the sample scene with the origin-shift trigger enabled and give
    /// the camera and cube explicit root transforms.
    fn shiftable_game_loop(threshold: f32) -> GameLoop {
        let mut game_loop = GameLoop::new(EngineConfig::default());
        game_loop.load_scene(engine_scene::sample_scene()).unwrap();
        game_loop
            .runtime
            .with_world_mut(|world| {
                world.scene_settings_mut().origin_shift.enabled = true;
                world.scene_settings_mut().origin_shift.threshold = threshold;
                let camera = world.entity_by_persistent_id("camera-main").unwrap();
                world.add_component(camera, Transform::default());
                let cube = world.entity_by_persistent_id("cube-01").unwrap();
                world.add_component(cube, Transform::default());
            })
            .unwrap();
        game_loop
    }

    fn set_translation(game_loop: &GameLoop, persistent_id: &str, translation: Vec3) {
        game_loop
            .runtime
            .with_world_mut(|world| {
                let entity = world.entity_by_persistent_id(persistent_id).unwrap();
                world.get_mut::<Transform>(entity).unwrap().translation = translation;
            })
            .unwrap();
    }

    fn translation_of(game_loop: &GameLoop, persistent_id: &str) -> Vec3 {
        game_loop
            .runtime
            .with_world(|world| {
                let entity = world.entity_by_persistent_id(persistent_id).unwrap();
                world.get::<Transform>(entity).unwrap().translation
            })
            .unwrap()
    }

    /// Logical position: `world_origin + world_position`. This is the
    /// invariant every origin shift must preserve.
    fn logical_position(game_loop: &GameLoop, persistent_id: &str) -> [f64; 3] {
        game_loop
            .runtime
            .with_world(|world| {
                let entity = world.entity_by_persistent_id(persistent_id).unwrap();
                let position = engine_scene::entity_world_position(world, entity).unwrap();
                let origin = world.world_origin();
                [
                    origin[0] + f64::from(position.x),
                    origin[1] + f64::from(position.y),
                    origin[2] + f64::from(position.z),
                ]
            })
            .unwrap()
    }

    #[test]
    fn origin_shift_is_disabled_by_default() {
        let mut game_loop = GameLoop::new(EngineConfig::default());
        game_loop.load_scene(engine_scene::sample_scene()).unwrap();
        game_loop
            .runtime
            .with_world_mut(|world| {
                let camera = world.entity_by_persistent_id("camera-main").unwrap();
                world.add_component(camera, Transform::default());
            })
            .unwrap();
        // Past the default 8 km threshold, but the opt-in flag stays off.
        set_translation(&game_loop, "camera-main", Vec3::new(9000.0, 0.0, 0.0));

        assert!(game_loop.tick_world_origin_shift().is_none());
        assert_eq!(game_loop.world_origin(), [0.0; 3]);
        assert_eq!(game_loop.world_origin_shift_count(), 0);
        assert_eq!(game_loop.last_world_origin_shift(), None);
        assert_eq!(
            translation_of(&game_loop, "camera-main"),
            Vec3::new(9000.0, 0.0, 0.0)
        );
    }

    #[test]
    fn origin_shift_triggers_past_threshold_and_preserves_logical_positions() {
        let mut game_loop = shiftable_game_loop(100.0);
        set_translation(&game_loop, "camera-main", Vec3::new(150.0, 0.0, 0.0));
        set_translation(&game_loop, "cube-01", Vec3::new(160.0, 5.0, -20.0));
        let cube_logical_before = logical_position(&game_loop, "cube-01");

        let shift = game_loop.tick_world_origin_shift().expect("shift runs");

        assert_eq!(shift.delta, [150.0, 0.0, 0.0]);
        assert_eq!(shift.origin, [150.0, 0.0, 0.0]);
        assert_eq!(shift.transforms, 2);
        assert_eq!(game_loop.world_origin(), [150.0, 0.0, 0.0]);
        assert_eq!(game_loop.world_origin_shift_count(), 1);
        assert_eq!(game_loop.last_world_origin_shift(), Some(shift));
        // The reference camera lands back on the relative origin and every
        // logical position is unchanged.
        assert!(translation_of(&game_loop, "camera-main").length() < 1e-4);
        assert_eq!(logical_position(&game_loop, "cube-01"), cube_logical_before);
        assert_eq!(
            logical_position(&game_loop, "camera-main"),
            [150.0, 0.0, 0.0]
        );
    }

    #[test]
    fn origin_shift_stays_put_below_threshold() {
        let mut game_loop = shiftable_game_loop(100.0);
        set_translation(&game_loop, "camera-main", Vec3::new(50.0, 0.0, 0.0));

        assert!(game_loop.tick_world_origin_shift().is_none());
        assert_eq!(game_loop.world_origin(), [0.0; 3]);
        assert_eq!(game_loop.world_origin_shift_count(), 0);
        assert_eq!(
            translation_of(&game_loop, "camera-main"),
            Vec3::new(50.0, 0.0, 0.0)
        );
    }

    #[test]
    fn origin_shift_runs_at_most_once_per_tick() {
        let mut game_loop = shiftable_game_loop(100.0);
        set_translation(&game_loop, "camera-main", Vec3::new(150.0, 0.0, 0.0));

        assert!(game_loop.tick_world_origin_shift().is_some());
        // After the shift the reference sits at the relative origin, so a
        // second evaluation in the same frame must not shift again.
        assert!(game_loop.tick_world_origin_shift().is_none());
        assert_eq!(game_loop.world_origin_shift_count(), 1);
    }

    #[test]
    fn origin_shift_watches_the_configured_reference_entity() {
        let mut game_loop = shiftable_game_loop(100.0);
        game_loop
            .runtime
            .with_world_mut(|world| {
                world.scene_settings_mut().origin_shift.reference_entity =
                    Some("cube-01".to_string());
            })
            .unwrap();
        set_translation(&game_loop, "camera-main", Vec3::new(10.0, 0.0, 0.0));
        set_translation(&game_loop, "cube-01", Vec3::new(220.0, 0.0, 30.0));

        let shift = game_loop.tick_world_origin_shift().expect("shift runs");

        assert_eq!(shift.delta, [220.0, 0.0, 30.0]);
        // The reference entity lands on the relative origin while the camera
        // keeps its logical position.
        assert!(translation_of(&game_loop, "cube-01").length() < 1e-4);
        assert_eq!(
            logical_position(&game_loop, "camera-main"),
            [10.0, 0.0, 0.0]
        );
    }

    #[test]
    fn origin_shift_waits_for_the_frame_boundary() {
        let mut game_loop = shiftable_game_loop(100.0);
        set_translation(&game_loop, "camera-main", Vec3::new(150.0, 0.0, 0.0));

        // `update()` never triggers a shift mid-frame; the host calls
        // `tick_world_origin_shift` at the frame boundary.
        game_loop.update(0.1);
        assert_eq!(game_loop.world_origin(), [0.0; 3]);
        assert_eq!(game_loop.world_origin_shift_count(), 0);

        assert!(game_loop.tick_world_origin_shift().is_some());
        assert_eq!(game_loop.world_origin_shift_count(), 1);
    }

    #[test]
    fn origin_shift_moves_character_controllers_and_the_primary_mirror() {
        let mut game_loop = shiftable_game_loop(100.0);
        game_loop
            .runtime
            .with_world_mut(|world| {
                let cube = world.entity_by_persistent_id("cube-01").unwrap();
                let mut controller = CharacterController::new();
                controller.set_position(Vec3::new(200.0, 1.0, 0.0));
                world.add_component(cube, controller);
            })
            .unwrap();
        let mut mirror = CharacterController::new();
        mirror.set_position(Vec3::new(200.0, 1.0, 0.0));
        game_loop.character = Some(mirror);
        set_translation(&game_loop, "camera-main", Vec3::new(150.0, 0.0, 0.0));

        let shift = game_loop.tick_world_origin_shift().expect("shift runs");

        assert_eq!(shift.character_controllers, 1);
        game_loop
            .runtime
            .with_world(|world| {
                let cube = world.entity_by_persistent_id("cube-01").unwrap();
                assert_eq!(
                    world.get::<CharacterController>(cube).unwrap().position(),
                    Vec3::new(50.0, 1.0, 0.0)
                );
            })
            .unwrap();
        // The primary mirror moves with the component so a same-frame read
        // cannot observe the pre-shift position.
        assert_eq!(
            game_loop.character.as_ref().unwrap().position(),
            Vec3::new(50.0, 1.0, 0.0)
        );
    }

    #[cfg(all(feature = "subsystem-physics", feature = "subsystem-gameplay"))]
    #[test]
    fn origin_shift_teleports_physics_bodies_and_sweeps_gravity_sources() {
        use engine_physics::{Collider, GravityMode, GravitySource, RigidBody};

        let mut game_loop = shiftable_game_loop(100.0);
        game_loop
            .runtime
            .with_world_mut(|world| {
                let cube = world.entity_by_persistent_id("cube-01").unwrap();
                world.add_component(cube, RigidBody::default());
                world.add_component(cube, Collider::default());
                world.add_component(
                    cube,
                    GravitySource {
                        mode: GravityMode::Point,
                        center: Vec3::new(400.0, 0.0, 0.0),
                        ..GravitySource::default()
                    },
                );
            })
            .unwrap();
        set_translation(&game_loop, "cube-01", Vec3::new(300.0, 0.0, 0.0));
        game_loop.init_physics();
        set_translation(&game_loop, "camera-main", Vec3::new(150.0, 0.0, 0.0));

        let shift = game_loop.tick_world_origin_shift().expect("shift runs");

        assert_eq!(shift.physics_bodies, 1);
        assert_eq!(shift.gravity_sources, 1);
        // The point gravity centre moved by -delta.
        game_loop
            .runtime
            .with_world(|world| {
                let cube = world.entity_by_persistent_id("cube-01").unwrap();
                assert_eq!(
                    world.get::<GravitySource>(cube).unwrap().center,
                    Vec3::new(250.0, 0.0, 0.0)
                );
            })
            .unwrap();
        // Queries observe the teleported body immediately: a ray over the
        // shifted position hits, a ray over the stale position misses.
        let physics = game_loop.physics.as_ref().unwrap();
        assert!(physics
            .raycast(Vec3::new(150.0, 10.0, 0.0), Vec3::NEG_Y, 100.0)
            .is_some());
        assert!(physics
            .raycast(Vec3::new(300.0, 10.0, 0.0), Vec3::NEG_Y, 100.0)
            .is_none());

        // A subsequent frame keeps the entity at its logical position: the
        // physics -> ECS resync must not yank the body back to pre-shift
        // coordinates.
        game_loop.update(0.0);
        let cube_x = translation_of(&game_loop, "cube-01").x;
        assert!((cube_x - 150.0).abs() < 1e-3, "{cube_x}");
        assert_eq!(logical_position(&game_loop, "cube-01")[0], 300.0);
    }

    #[cfg(all(
        feature = "subsystem-animation",
        feature = "subsystem-audio",
        feature = "subsystem-navigation",
        feature = "subsystem-ui"
    ))]
    #[test]
    fn origin_shift_moves_nav_agent_targets() {
        let mut game_loop = shiftable_game_loop(100.0);
        game_loop
            .runtime
            .with_world_mut(|world| {
                let cube = world.entity_by_persistent_id("cube-01").unwrap();
                let mut agent = engine_nav::AiAgent::new();
                agent.target = Some(Vec3::new(500.0, 0.0, -100.0));
                world.add_component(cube, agent);
            })
            .unwrap();
        set_translation(&game_loop, "camera-main", Vec3::new(150.0, 0.0, 0.0));

        let shift = game_loop.tick_world_origin_shift().expect("shift runs");

        assert_eq!(shift.nav_agents, 1);
        game_loop
            .runtime
            .with_world(|world| {
                let cube = world.entity_by_persistent_id("cube-01").unwrap();
                assert_eq!(
                    world.get::<engine_nav::AiAgent>(cube).unwrap().target,
                    Some(Vec3::new(350.0, 0.0, -100.0))
                );
            })
            .unwrap();
    }

    #[cfg(feature = "runtime-audio-output")]
    #[test]
    fn origin_shift_moves_audio_snapshot_positions() {
        let mut game_loop = shiftable_game_loop(100.0);
        game_loop
            .runtime
            .with_world_mut(|world| {
                let cube = world.entity_by_persistent_id("cube-01").unwrap();
                let source = engine_audio::AudioSourceComponent {
                    spatial: true,
                    ..engine_audio::AudioSourceComponent::default()
                };
                world.add_component(cube, source);
                let camera = world.entity_by_persistent_id("camera-main").unwrap();
                world.add_component(camera, engine_audio::AudioListenerComponent::default());
            })
            .unwrap();
        set_translation(&game_loop, "camera-main", Vec3::new(150.0, 0.0, 0.0));
        set_translation(&game_loop, "cube-01", Vec3::new(300.0, 0.0, 0.0));

        game_loop.tick_world_origin_shift().expect("shift runs");

        // Audio state is rebuilt from ECS transforms every frame, so the
        // next snapshot already observes the shifted positions.
        let frame = game_loop.runtime_audio_frame();
        assert_eq!(frame.sources.len(), 1);
        assert_eq!(
            frame.sources[0].emitter.as_ref().unwrap().position,
            Vec3::new(150.0, 0.0, 0.0)
        );
        let listener = frame.listener.as_ref().unwrap();
        assert!(listener.position.length() < 1e-4, "{listener:?}");
    }
}
