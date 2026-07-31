// ── Script subsystem tests ──────────────────────────────────────────

#[cfg(feature = "subsystem-scripting-csharp")]
#[test]
fn in_process_csharp_bridge_installs_the_native_cdylib() {
    let _guard = serial_ffi_world_test();
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.set_world(World::new());

    runtime
        .install_in_process_csharp_ffi()
        .expect("matching engine_ffi cdylib should install");

    let path =
        engine_ffi::host_bridge::loaded_cdylib_path().expect("installed native library path");
    assert!(path.exists());
    assert_eq!(
        std::env::var("ENGINE_FFI_HOST_PID").ok(),
        Some(std::process::id().to_string())
    );
}

#[cfg(feature = "subsystem-scripting-csharp")]
#[test]
fn engine_runtime_script_host_registration() {
    use engine_script::MockHost;

    let config = EngineConfig::default();
    let mut runtime = EngineRuntime::new(config);

    assert_eq!(runtime.scripting.engine.host_count(), 0);
    runtime.register_script_host(Box::new(MockHost::new()));
    assert_eq!(runtime.scripting.engine.host_count(), 1);
}

#[cfg(feature = "subsystem-scripting-csharp")]
#[test]
fn engine_runtime_exposes_only_host_verified_script_classes() {
    use engine_script::MockHost;

    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.register_script_host(Box::new(
        MockHost::new().with_verified_classes("game", ["Game.Player"]),
    ));
    runtime
        .load_script_assembly("game", "mock", b"managed")
        .unwrap();

    let classes = runtime.verified_script_classes();
    assert_eq!(classes.len(), 1);
    assert_eq!(classes[0].assembly_id, "game");
    assert_eq!(classes[0].class_name, "Game.Player");
}

#[cfg(feature = "subsystem-scripting-csharp")]
#[test]
fn script_engine_replacement_is_atomic_and_does_not_accumulate_hosts() {
    use engine_script::{MockHost, ScriptEngine};

    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.register_script_host(Box::new(MockHost::new()));
    runtime
        .load_script_assembly("old", "mock", b"old")
        .expect("old runtime assembly");

    let invalid_candidate = ScriptEngine::new();
    let error = runtime
        .replace_script_engine(invalid_candidate, "mock")
        .expect_err("candidate without the selected host must be rejected");
    assert!(error.to_string().contains("exactly one host"));
    assert_eq!(runtime.script_engine().host_count(), 1);
    assert_eq!(runtime.script_engine().managers()[0].assembly_count(), 1);

    let mut duplicate_candidate = ScriptEngine::new();
    duplicate_candidate.register_host(Box::new(MockHost::new()));
    duplicate_candidate.register_host(Box::new(MockHost::new()));
    runtime
        .replace_script_engine(duplicate_candidate, "mock")
        .expect_err("duplicate selected hosts must be rejected");
    assert_eq!(runtime.script_engine().host_count(), 1);
    assert_eq!(runtime.script_engine().managers()[0].assembly_count(), 1);

    let mut candidate = ScriptEngine::new();
    candidate.register_host(Box::new(MockHost::new()));
    candidate
        .load_script("new-dependency", "mock", b"dependency")
        .expect("candidate dependency");
    candidate
        .load_script("new-game", "mock", b"game")
        .expect("candidate game assembly");

    runtime
        .replace_script_engine(candidate, "mock")
        .expect("valid candidate should replace the runtime");
    assert_eq!(runtime.script_engine().host_count(), 1);
    assert_eq!(runtime.script_engine().managers()[0].assembly_count(), 2);
}

#[cfg(feature = "subsystem-scripting-csharp")]
#[test]
fn engine_runtime_tick_scripts_no_panic() {
    let config = EngineConfig::default();
    let mut runtime = EngineRuntime::new(config);

    // Tick with no hosts registered — should not panic
    runtime.tick_scripts(0.016);
}

#[cfg(feature = "subsystem-scripting-csharp")]
#[test]
fn script_create_entity_is_transactional_first_wins_and_enters_next_snapshot() {
    let _guard = serial_ffi_world_test();
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.set_world(World::from_scene(&engine_scene::sample_scene()));
    let first_transform = ScriptTransform {
        translation: [7.0, 8.0, 9.0],
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale: [2.0, 3.0, 4.0],
    };
    let commands = vec![
        engine_script::OwnedGameplayCommand {
            entity_id: "cube-01".into(),
            command: GameplayCommand::CreateEntity {
                entity_id: "spawned-01".into(),
                transform: first_transform.clone(),
            },
        },
        engine_script::OwnedGameplayCommand {
            entity_id: "cube-01".into(),
            command: GameplayCommand::CreateEntity {
                entity_id: "spawned-01".into(),
                transform: ScriptTransform {
                    translation: [100.0, 100.0, 100.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0; 3],
                },
            },
        },
    ];

    let diagnostics = runtime.apply_script_gameplay_commands(commands);

    assert_eq!(
        diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "SCRIPT_ENTITY_CREATE_CONFLICT")
            .count(),
        1
    );
    runtime
        .with_world(|world| {
            assert_eq!(world.alive_count(), 3);
            let entity = world
                .entity_by_persistent_id("spawned-01")
                .expect("first creation must persist");
            let transform = world
                .get::<engine_scene::components::Transform>(entity)
                .expect("created entity must have Transform");
            assert_eq!(
                transform.translation.to_array(),
                first_transform.translation
            );
            assert_eq!(transform.rotation.to_array(), first_transform.rotation);
            assert_eq!(transform.scale.to_array(), first_transform.scale);
        })
        .expect("runtime must keep an active World");
    let snapshots = runtime.script_gameplay_entity_snapshots();
    assert_eq!(
        snapshots["spawned-01"].transform,
        Some(first_transform),
        "the next script context must include the newly-created entity"
    );
}

#[cfg(all(
    feature = "subsystem-scripting-csharp",
    feature = "subsystem-animation"
))]
#[test]
fn script_animation_command_uses_dedicated_player_mutation() {
    let _guard = serial_ffi_world_test();
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.set_world(World::from_scene(&engine_scene::sample_scene()));
    runtime
        .with_world_mut(|world| {
            let entity = world.entity_by_persistent_id("cube-01").unwrap();
            world.add_component(entity, engine_animation::AnimationPlayer::new());
        })
        .unwrap();

    let diagnostics =
        runtime.apply_script_gameplay_commands(vec![engine_script::OwnedGameplayCommand {
            entity_id: "cube-01".into(),
            command: GameplayCommand::PlayAnimation {
                entity_id: "cube-01".into(),
                clip_asset: "battle.attack".into(),
                looping: false,
                speed: 1.5,
                restart: true,
            },
        }]);

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    runtime
        .with_world(|world| {
            let entity = world.entity_by_persistent_id("cube-01").unwrap();
            let player = world
                .get::<engine_animation::AnimationPlayer>(entity)
                .unwrap();
            assert_eq!(player.clip_asset.as_deref(), Some("battle.attack"));
            assert!(player.playing);
            assert!(!player.looping);
            assert_eq!(player.speed, 1.5);
        })
        .unwrap();
}

#[cfg(feature = "subsystem-scripting-csharp")]
#[test]
fn script_create_entity_validation_and_missing_owner_never_partially_create() {
    let _guard = serial_ffi_world_test();
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.set_world(World::from_scene(&engine_scene::sample_scene()));
    let valid_transform = ScriptTransform {
        translation: [0.0; 3],
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale: [1.0; 3],
    };
    let commands = vec![
        engine_script::OwnedGameplayCommand {
            entity_id: "cube-01".into(),
            command: GameplayCommand::CreateEntity {
                entity_id: "../invalid".into(),
                transform: valid_transform.clone(),
            },
        },
        engine_script::OwnedGameplayCommand {
            entity_id: "cube-01".into(),
            command: GameplayCommand::CreateEntity {
                entity_id: "invalid-transform".into(),
                transform: ScriptTransform {
                    rotation: [0.0; 4],
                    ..valid_transform.clone()
                },
            },
        },
        engine_script::OwnedGameplayCommand {
            entity_id: "missing-owner".into(),
            command: GameplayCommand::CreateEntity {
                entity_id: "orphan".into(),
                transform: valid_transform,
            },
        },
    ];

    let diagnostics = runtime.apply_script_gameplay_commands(commands);

    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "SCRIPT_ENTITY_CREATE_ID_INVALID"));
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "SCRIPT_ENTITY_CREATE_TRANSFORM_INVALID"));
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "SCRIPT_COMMAND_OWNER_MISSING"));
    runtime
        .with_world(|world| {
            assert_eq!(world.alive_count(), 2);
            assert!(world.entity_by_persistent_id("invalid-transform").is_none());
            assert!(world.entity_by_persistent_id("orphan").is_none());
        })
        .expect("runtime must keep an active World");
}

#[cfg(feature = "subsystem-scripting-csharp")]
#[test]
fn script_create_entity_rechecks_owner_after_prior_same_frame_destroy() {
    let _guard = serial_ffi_world_test();
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.set_world(World::from_scene(&engine_scene::sample_scene()));
    let commands = vec![
        engine_script::OwnedGameplayCommand {
            entity_id: "cube-01".into(),
            command: GameplayCommand::DestroySelf,
        },
        engine_script::OwnedGameplayCommand {
            entity_id: "cube-01".into(),
            command: GameplayCommand::CreateEntity {
                entity_id: "after-destroy".into(),
                transform: ScriptTransform {
                    translation: [0.0; 3],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0; 3],
                },
            },
        },
    ];

    let diagnostics = runtime.apply_script_gameplay_commands(commands);

    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "SCRIPT_COMMAND_OWNER_MISSING"));
    runtime
        .with_world(|world| {
            assert!(world.entity_by_persistent_id("cube-01").is_none());
            assert!(world.entity_by_persistent_id("after-destroy").is_none());
        })
        .expect("runtime must keep an active World");
}
