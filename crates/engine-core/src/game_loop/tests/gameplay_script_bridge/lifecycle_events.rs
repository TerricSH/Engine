#[test]
fn explicit_destroy_runs_target_ondestroy_and_detaches_only_that_entity() {
    let destroy_count = Arc::new(AtomicUsize::new(0));
    let mut game_loop = GameLoop::new(EngineConfig::default());
    game_loop
        .runtime
        .register_script_host(Box::new(InputDrivenHost::new(Arc::clone(&destroy_count))));
    game_loop.runtime.set_script_host_name("bridge-test");
    game_loop
        .runtime
        .load_script_assembly("game", "bridge-test", b"test")
        .unwrap();

    let mut scene = engine_scene::sample_scene();
    for entity in &mut scene.entities {
        entity.components.insert(
            "engine.script".into(),
            engine_scene::ComponentRecord {
                schema_version: SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: BTreeMap::from([
                    ("assembly_id".into(), Value::Str("game".into())),
                    ("class_name".into(), Value::Str("Actor".into())),
                ]),
            },
        );
    }
    game_loop.load_scene(scene).unwrap();
    assert_eq!(
        game_loop.runtime.script_engine().managers()[0].instance_count(),
        2
    );

    let mut destroy_camera = InputAction::new("destroy_camera", InputValueType::Digital);
    destroy_camera.current_value = InputValue::Bool(true);
    game_loop.input_map.add_action(destroy_camera);
    game_loop.update(1.0 / 60.0);

    game_loop
        .runtime
        .with_world(|world| {
            assert!(world.entity_by_persistent_id("camera-main").is_none());
            assert!(world.entity_by_persistent_id("cube-01").is_some());
        })
        .unwrap();
    assert_eq!(destroy_count.load(Ordering::SeqCst), 1);
    assert_eq!(
        game_loop.runtime.script_engine().managers()[0].instance_count(),
        1
    );
    assert!(game_loop
        .runtime
        .diagnostics_collector()
        .script_diagnostics
        .is_empty());
}

#[test]
fn invalid_script_transform_reports_a_specific_diagnostic() {
    assert_eq!(
        crate::runtime::validate_script_transform(&ScriptTransform {
            translation: [f32::NAN, 0.0, 0.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0, 1.0, 1.0],
        }),
        Err("translation, rotation, and scale must contain only finite values")
    );
}

#[test]
fn physics_contacts_are_mapped_symmetrically_to_persistent_script_entities() {
    use engine_physics::{CollisionEvent, CollisionEventKind};
    use engine_script::{GameplayPhysicsEvent, GameplayPhysicsEventKind};

    let mut game_loop = GameLoop::new(EngineConfig::default());
    game_loop.load_scene(engine_scene::sample_scene()).unwrap();
    let (cube, camera) = game_loop
        .runtime
        .with_world(|world| {
            (
                world.entity_by_persistent_id("cube-01").unwrap(),
                world.entity_by_persistent_id("camera-main").unwrap(),
            )
        })
        .unwrap();
    game_loop.physics_events.collisions.push(CollisionEvent {
        kind: CollisionEventKind::ContactStarted,
        entity_a: cube,
        entity_b: camera,
    });

    let events = game_loop.resolved_script_physics_events();

    assert_eq!(
        events.get("cube-01"),
        Some(&vec![GameplayPhysicsEvent {
            kind: GameplayPhysicsEventKind::CollisionEntered,
            other_entity_id: "camera-main".into(),
            joint_id: None,
            force: None,
            torque: None,
        }])
    );
    assert_eq!(
        events.get("camera-main"),
        Some(&vec![GameplayPhysicsEvent {
            kind: GameplayPhysicsEventKind::CollisionEntered,
            other_entity_id: "cube-01".into(),
            joint_id: None,
            force: None,
            torque: None,
        }])
    );
}

#[test]
fn joint_breaks_reach_both_script_bodies_with_constraint_and_load() {
    use engine_physics::{JointBreakEvent, JointHandle};
    use engine_script::{GameplayPhysicsEvent, GameplayPhysicsEventKind};

    let mut game_loop = GameLoop::new(EngineConfig::default());
    game_loop.load_scene(engine_scene::sample_scene()).unwrap();
    let (cube, camera, constraint) = game_loop
        .runtime
        .with_world_mut(|world| {
            (
                world.entity_by_persistent_id("cube-01").unwrap(),
                world.entity_by_persistent_id("camera-main").unwrap(),
                world.create_persistent_entity("cube-tether").unwrap(),
            )
        })
        .unwrap();
    game_loop.physics_events.joint_breaks.push(JointBreakEvent {
        handle: JointHandle(7),
        joint_entity: Some(constraint),
        entity_a: cube,
        entity_b: camera,
        force: 1250.0,
        torque: 75.0,
    });

    let events = game_loop.resolved_script_physics_events();
    let expected_for_cube = GameplayPhysicsEvent {
        kind: GameplayPhysicsEventKind::JointBroken,
        other_entity_id: "camera-main".into(),
        joint_id: Some("cube-tether".into()),
        force: Some(1250.0),
        torque: Some(75.0),
    };
    let expected_for_camera = GameplayPhysicsEvent {
        other_entity_id: "cube-01".into(),
        ..expected_for_cube.clone()
    };
    assert_eq!(events.get("cube-01"), Some(&vec![expected_for_cube]));
    assert_eq!(events.get("camera-main"), Some(&vec![expected_for_camera]));
}

#[test]
fn entity_relative_physics_events_reach_the_script_gameplay_context() {
    use engine_script::{GameplayPhysicsEvent, GameplayPhysicsEventKind};

    let contexts = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut game_loop = GameLoop::new(EngineConfig::default());
    game_loop
        .runtime
        .register_script_host(Box::new(ContextRecordingHost {
            contexts: Arc::clone(&contexts),
        }));
    game_loop.runtime.set_script_host_name("context-recording");
    game_loop
        .runtime
        .load_script_assembly("game", "context-recording", b"test")
        .unwrap();

    let mut scene = engine_scene::sample_scene();
    let target = scene
        .entities
        .iter_mut()
        .find(|entity| entity.persistent_id == "cube-01")
        .unwrap();
    target.components.insert(
        "engine.transform".into(),
        engine_scene::ComponentRecord {
            schema_version: SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: BTreeMap::new(),
        },
    );
    target.components.insert(
        "engine.script".into(),
        engine_scene::ComponentRecord {
            schema_version: SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: BTreeMap::from([
                ("assembly_id".into(), Value::Str("game".into())),
                ("class_name".into(), Value::Str("Player".into())),
            ]),
        },
    );
    game_loop.load_scene(scene).unwrap();

    let expected = GameplayPhysicsEvent {
        kind: GameplayPhysicsEventKind::TriggerEntered,
        other_entity_id: "camera-main".into(),
        joint_id: None,
        force: None,
        torque: None,
    };
    game_loop.runtime.tick_scripts_with_input_and_physics(
        1.0 / 60.0,
        &BTreeMap::new(),
        &BTreeMap::from([("cube-01".into(), vec![expected.clone()])]),
    );

    let contexts = contexts.lock().unwrap();
    let latest = contexts.last().expect("script gameplay context");
    assert_eq!(latest.entity_id, "cube-01");
    assert_eq!(latest.physics_events, vec![expected]);
}

#[test]
fn script_input_transitions_fire_once_for_press_and_release_edges() {
    let contexts = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut game_loop = GameLoop::new(EngineConfig::default());
    game_loop
        .runtime
        .register_script_host(Box::new(ContextRecordingHost {
            contexts: Arc::clone(&contexts),
        }));
    game_loop.runtime.set_script_host_name("context-recording");
    game_loop
        .runtime
        .load_script_assembly("game", "context-recording", b"test")
        .unwrap();

    let mut scene = engine_scene::sample_scene();
    scene
        .entities
        .iter_mut()
        .find(|entity| entity.persistent_id == "cube-01")
        .unwrap()
        .components
        .insert(
            "engine.script".into(),
            engine_scene::ComponentRecord {
                schema_version: SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: BTreeMap::from([
                    ("assembly_id".into(), Value::Str("game".into())),
                    ("class_name".into(), Value::Str("Player".into())),
                ]),
            },
        );
    game_loop.load_scene(scene).unwrap();

    let mut jump = InputAction::new("jump", InputValueType::Digital);
    jump.current_value = InputValue::Bool(true);
    game_loop.input_map.add_action(jump);
    game_loop.update(1.0 / 60.0);
    let pressed = contexts.lock().unwrap().last().unwrap().clone();
    assert!(pressed.input_transitions.was_pressed("jump"));
    assert!(!pressed.input_transitions.was_released("jump"));

    game_loop.update(1.0 / 60.0);
    let held = contexts.lock().unwrap().last().unwrap().clone();
    assert!(!held.input_transitions.was_pressed("jump"));
    assert!(!held.input_transitions.was_released("jump"));

    game_loop
        .input_map
        .action_mut("jump")
        .unwrap()
        .current_value = InputValue::Bool(false);
    game_loop.update(1.0 / 60.0);
    let released = contexts.lock().unwrap().last().unwrap().clone();
    assert!(!released.input_transitions.was_pressed("jump"));
    assert!(released.input_transitions.was_released("jump"));

    game_loop.update(1.0 / 60.0);
    let idle = contexts.lock().unwrap().last().unwrap().clone();
    assert_eq!(
        idle.input_transitions,
        engine_script::GameplayInputTransitions::default()
    );
}

#[test]
fn scalar_and_vector_script_actions_use_the_documented_edge_threshold() {
    use engine_script::GameplayInputValue;

    assert!(!script_input_value_is_active(&GameplayInputValue::Float(
        0.5
    )));
    assert!(script_input_value_is_active(&GameplayInputValue::Float(
        0.51
    )));
    assert!(script_input_value_is_active(&GameplayInputValue::Float(
        -0.51
    )));
    assert!(!script_input_value_is_active(&GameplayInputValue::Vec2([
        0.29, 0.4
    ])));
    assert!(script_input_value_is_active(&GameplayInputValue::Vec2([
        0.31, 0.4
    ])));
}

#[test]
fn script_scene_command_is_deferred_until_the_host_frame_boundary() {
    let mut game_loop = GameLoop::new(EngineConfig::default());
    let destroy_count = Arc::new(AtomicUsize::new(0));
    game_loop
        .runtime
        .register_script_host(Box::new(InputDrivenHost::new(destroy_count)));
    game_loop.runtime.set_script_host_name("bridge-test");
    game_loop
        .runtime
        .load_script_assembly("game", "bridge-test", b"test")
        .unwrap();

    let mut scene = engine_scene::sample_scene();
    let target = scene
        .entities
        .iter_mut()
        .find(|entity| entity.persistent_id == "cube-01")
        .unwrap();
    target.components.insert(
        "engine.transform".into(),
        engine_scene::ComponentRecord {
            schema_version: SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: BTreeMap::new(),
        },
    );
    target.components.insert(
        "engine.script".into(),
        engine_scene::ComponentRecord {
            schema_version: SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: BTreeMap::from([
                ("assembly_id".into(), Value::Str("game".into())),
                ("class_name".into(), Value::Str("Player".into())),
            ]),
        },
    );
    game_loop.load_scene(scene).unwrap();

    let mut load = InputAction::new("load_level", InputValueType::Digital);
    load.current_value = InputValue::Bool(true);
    game_loop.input_map.add_action(load);
    game_loop.update(1.0 / 60.0);

    game_loop
        .input_map
        .action_mut("load_level")
        .unwrap()
        .current_value = InputValue::Bool(false);
    let mut load_other = InputAction::new("load_other", InputValueType::Digital);
    load_other.current_value = InputValue::Bool(true);
    game_loop.input_map.add_action(load_other);
    game_loop.update(1.0 / 60.0);

    assert_eq!(
        game_loop.runtime.take_pending_scene_request(),
        Some(crate::SceneLoadRequest {
            scene_id: "level_two".into(),
            requested_by: "cube-01".into(),
        })
    );
    assert_eq!(game_loop.runtime.take_pending_scene_request(), None);
    assert!(game_loop
        .runtime
        .diagnostics_collector()
        .script_diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "SCRIPT_SCENE_REQUEST_CONFLICT"));
    assert_eq!(
        game_loop
            .runtime
            .scene_ref()
            .map(|scene| scene.scene_id.as_str()),
        Some("scene-gate04-valid")
    );
}
