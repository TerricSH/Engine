#[cfg(feature = "subsystem-scripting-csharp")]
fn script_spawn_test_transform_record(translation: [f32; 3]) -> engine_scene::ComponentRecord {
    engine_scene::ComponentRecord {
        schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
        enabled: true,
        fields: std::collections::BTreeMap::from([
            (
                "translation".to_string(),
                engine_serialize::Value::Vec3(translation),
            ),
            (
                "rotation".to_string(),
                engine_serialize::Value::Quat([
                    0.0,
                    0.0,
                    std::f32::consts::FRAC_1_SQRT_2,
                    std::f32::consts::FRAC_1_SQRT_2,
                ]),
            ),
            (
                "scale".to_string(),
                engine_serialize::Value::Vec3([2.0, 2.0, 2.0]),
            ),
        ]),
    }
}

/// Two-entity prefab: root `root` with a rotated/scaled Transform and a
/// child `bolt`, so tests can assert deterministic id assignment,
/// hierarchy parenting, and translation overrides.
#[cfg(feature = "subsystem-scripting-csharp")]
fn script_spawn_test_prefab(prefab_id: &str) -> engine_scene::Prefab {
    let mut prefab = engine_scene::Prefab::new(AssetId::new(prefab_id));
    prefab.add_entity(engine_scene::EntityRecord {
        persistent_id: "root".to_string(),
        parent: None,
        name: Some("Root".to_string()),
        enabled: true,
        components: std::collections::BTreeMap::from([(
            "engine.transform".to_string(),
            script_spawn_test_transform_record([1.0, 2.0, 3.0]),
        )]),
    });
    prefab.add_entity(engine_scene::EntityRecord {
        persistent_id: "bolt".to_string(),
        parent: Some("root".to_string()),
        name: Some("Bolt".to_string()),
        enabled: true,
        components: std::collections::BTreeMap::from([(
            "engine.transform".to_string(),
            script_spawn_test_transform_record([0.0, 1.0, 0.0]),
        )]),
    });
    prefab
}

/// Install a cooked prefab into the runtime exactly like the cooked-batch
/// loader does: typed payload in the asset registry plus the extension
/// type-id registration that `extension_asset::<Prefab>("prefab", ..)`
/// consults.
#[cfg(feature = "subsystem-scripting-csharp")]
fn register_script_prefab(
    runtime: &mut EngineRuntime,
    prefab_id: &str,
    prefab: engine_scene::Prefab,
) {
    let asset_id = AssetId::new(prefab_id);
    runtime
        .asset_registry_mut()
        .insert_typed(asset_id.clone(), prefab);
    runtime
        .loaded_extension_asset_ids
        .entry("prefab".to_string())
        .or_default()
        .insert(asset_id);
}

#[cfg(feature = "subsystem-scripting-csharp")]
fn spawn_prefab_command(
    owner: &str,
    prefab_id: &str,
    translation: Option<[f32; 3]>,
) -> engine_script::OwnedGameplayCommand {
    engine_script::OwnedGameplayCommand {
        entity_id: owner.to_string(),
        command: GameplayCommand::SpawnPrefab {
            prefab_id: prefab_id.to_string(),
            translation,
        },
    }
}

#[cfg(feature = "subsystem-scripting-csharp")]
#[test]
fn script_spawn_prefab_assigns_deterministic_ids_and_enters_next_snapshot() {
    let _guard = serial_ffi_world_test();
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.set_world(World::from_scene(&engine_scene::sample_scene()));
    register_script_prefab(
        &mut runtime,
        "prefab-x",
        script_spawn_test_prefab("prefab-x"),
    );

    let diagnostics = runtime.apply_script_gameplay_commands(vec![
        spawn_prefab_command("cube-01", "prefab-x", None),
        spawn_prefab_command("cube-01", "prefab-x", None),
    ]);

    assert!(
        diagnostics.is_empty(),
        "unexpected spawn diagnostics: {diagnostics:?}"
    );
    runtime
        .with_world(|world| {
            assert_eq!(world.alive_count(), 6);
            for id in ["prefab-x", "prefab-x.bolt", "prefab-x-2", "prefab-x-2.bolt"] {
                assert!(
                    world.entity_by_persistent_id(id).is_some(),
                    "missing spawned entity '{id}'"
                );
            }
            let root = world
                .entity_by_persistent_id("prefab-x")
                .expect("first spawn keeps the bare prefab id");
            let root_transform = world
                .get::<engine_scene::components::Transform>(root)
                .expect("spawned root must keep its Transform");
            assert_eq!(root_transform.translation.to_array(), [1.0, 2.0, 3.0]);
            let child = world
                .entity_by_persistent_id("prefab-x.bolt")
                .expect("child id derives from the prefab-local id");
            let child_transform = world
                .get::<engine_scene::components::Transform>(child)
                .expect("spawned child must keep its Transform");
            assert_eq!(child_transform.parent, Some(root));
        })
        .expect("runtime must keep an active World");
    let snapshots = runtime.script_gameplay_entity_snapshots();
    for id in ["prefab-x", "prefab-x.bolt", "prefab-x-2", "prefab-x-2.bolt"] {
        assert!(
            snapshots.contains_key(id),
            "next script context must include spawned entity '{id}'"
        );
    }
}

#[cfg(feature = "subsystem-scripting-csharp")]
#[test]
fn script_spawn_prefab_unknown_id_reports_actionable_diagnostic() {
    let _guard = serial_ffi_world_test();
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.set_world(World::from_scene(&engine_scene::sample_scene()));
    register_script_prefab(
        &mut runtime,
        "prefab-x",
        script_spawn_test_prefab("prefab-x"),
    );

    let diagnostics = runtime.apply_script_gameplay_commands(vec![spawn_prefab_command(
        "cube-01",
        "prefab-missing",
        None,
    )]);

    assert_eq!(diagnostics.len(), 1);
    let diagnostic = &diagnostics[0];
    assert_eq!(diagnostic.code, "SCRIPT_PREFAB_UNKNOWN");
    assert_eq!(diagnostic.entity.as_deref(), Some("cube-01"));
    runtime
        .with_world(|world| {
            assert_eq!(world.alive_count(), 2);
        })
        .expect("runtime must keep an active World");
}

#[cfg(feature = "subsystem-scripting-csharp")]
#[test]
fn script_spawn_prefab_invalid_requests_never_partially_spawn() {
    let _guard = serial_ffi_world_test();
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.set_world(World::from_scene(&engine_scene::sample_scene()));
    register_script_prefab(
        &mut runtime,
        "prefab-x",
        script_spawn_test_prefab("prefab-x"),
    );

    let diagnostics = runtime.apply_script_gameplay_commands(vec![
        spawn_prefab_command("cube-01", "../invalid", None),
        spawn_prefab_command("cube-01", "prefab-x", Some([f32::NAN, 0.0, 0.0])),
        spawn_prefab_command("missing-owner", "prefab-x", None),
    ]);

    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "SCRIPT_PREFAB_ID_INVALID"));
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "SCRIPT_PREFAB_TRANSFORM_INVALID"));
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "SCRIPT_COMMAND_OWNER_MISSING"));
    runtime
        .with_world(|world| {
            assert_eq!(world.alive_count(), 2);
            assert!(world.entity_by_persistent_id("prefab-x").is_none());
        })
        .expect("runtime must keep an active World");
}

#[cfg(feature = "subsystem-scripting-csharp")]
#[test]
fn script_spawn_prefab_translation_override_preserves_prefab_rotation_and_scale() {
    let _guard = serial_ffi_world_test();
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.set_world(World::from_scene(&engine_scene::sample_scene()));
    register_script_prefab(
        &mut runtime,
        "prefab-x",
        script_spawn_test_prefab("prefab-x"),
    );

    let diagnostics = runtime.apply_script_gameplay_commands(vec![spawn_prefab_command(
        "cube-01",
        "prefab-x",
        Some([7.0, 8.0, 9.0]),
    )]);

    assert!(
        diagnostics.is_empty(),
        "unexpected spawn diagnostics: {diagnostics:?}"
    );
    runtime
        .with_world(|world| {
            let root = world
                .entity_by_persistent_id("prefab-x")
                .expect("spawned root must exist");
            let transform = world
                .get::<engine_scene::components::Transform>(root)
                .expect("spawned root must keep its Transform");
            assert_eq!(transform.translation.to_array(), [7.0, 8.0, 9.0]);
            assert_eq!(
                transform.rotation.to_array(),
                [
                    0.0,
                    0.0,
                    std::f32::consts::FRAC_1_SQRT_2,
                    std::f32::consts::FRAC_1_SQRT_2
                ],
                "the override must not reset the prefab rotation"
            );
            assert_eq!(
                transform.scale.to_array(),
                [2.0, 2.0, 2.0],
                "the override must not reset the prefab scale"
            );
            let child = world
                .entity_by_persistent_id("prefab-x.bolt")
                .expect("spawned child must exist");
            let child_transform = world
                .get::<engine_scene::components::Transform>(child)
                .expect("spawned child must keep its Transform");
            assert_eq!(
                child_transform.translation.to_array(),
                [0.0, 1.0, 0.0],
                "the override only applies to the root"
            );
        })
        .expect("runtime must keep an active World");
}

#[cfg(feature = "subsystem-scripting-csharp")]
#[test]
fn script_spawn_prefab_attaches_scene_only_scripts_and_creates_instances() {
    let _guard = serial_ffi_world_test();
    use engine_script::MockHost;

    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.register_script_host(Box::new(MockHost::new()));
    runtime.set_script_host_name("mock");
    runtime
        .load_script_assembly("game", "mock", b"managed")
        .expect("mock assembly should load");
    runtime.set_world(World::from_scene(&engine_scene::sample_scene()));

    let mut prefab = engine_scene::Prefab::new(AssetId::new("prefab-scripted"));
    prefab.add_entity(engine_scene::EntityRecord {
        persistent_id: "root".to_string(),
        parent: None,
        name: Some("Root".to_string()),
        enabled: true,
        components: std::collections::BTreeMap::from([
            (
                "engine.transform".to_string(),
                script_spawn_test_transform_record([0.0; 3]),
            ),
            (
                "engine.script".to_string(),
                engine_scene::ComponentRecord {
                    schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
                    enabled: true,
                    fields: std::collections::BTreeMap::from([
                        (
                            "assembly_id".to_string(),
                            engine_serialize::Value::Str("game".to_string()),
                        ),
                        (
                            "class_name".to_string(),
                            engine_serialize::Value::Str("Game.Spawned".to_string()),
                        ),
                    ]),
                },
            ),
        ]),
    });
    register_script_prefab(&mut runtime, "prefab-scripted", prefab);

    assert_eq!(runtime.scripting.engine.managers()[0].instance_count(), 0);
    let diagnostics = runtime.apply_script_gameplay_commands(vec![spawn_prefab_command(
        "cube-01",
        "prefab-scripted",
        None,
    )]);

    assert!(
        diagnostics.is_empty(),
        "unexpected spawn diagnostics: {diagnostics:?}"
    );
    assert_eq!(
        runtime.scripting.engine.managers()[0].instance_count(),
        1,
        "the scene-only engine.script record must attach to the spawned entity"
    );

    let diagnostics = runtime.apply_script_gameplay_commands(vec![spawn_prefab_command(
        "cube-01",
        "prefab-scripted",
        None,
    )]);
    assert!(
        diagnostics.is_empty(),
        "unexpected spawn diagnostics: {diagnostics:?}"
    );
    assert_eq!(runtime.scripting.engine.managers()[0].instance_count(), 2);
    runtime
        .with_world(|world| {
            assert!(world.entity_by_persistent_id("prefab-scripted").is_some());
            assert!(world.entity_by_persistent_id("prefab-scripted-2").is_some());
        })
        .expect("runtime must keep an active World");
}

#[cfg(feature = "subsystem-scripting-csharp")]
#[test]
fn script_character_control_queues_intent_on_the_target_controller() {
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.set_world(World::from_scene(&engine_scene::sample_scene()));
    runtime.with_world_mut(|world| {
        let entity = world.entity_by_persistent_id("cube-01").unwrap();
        world.add_component(entity, engine_character::CharacterController::new());
    });

    let diagnostics =
        runtime.apply_script_gameplay_commands(vec![engine_script::OwnedGameplayCommand {
            entity_id: "cube-01".into(),
            command: GameplayCommand::CharacterControl {
                entity_id: "cube-01".into(),
                direction: [0.6, 0.0, -0.8],
                jump: true,
                speed: Some(7.5),
            },
        }]);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    runtime.with_world(|world| {
        let entity = world.entity_by_persistent_id("cube-01").unwrap();
        let controller = world
            .get::<engine_character::CharacterController>(entity)
            .unwrap();
        assert_eq!(controller.pending_commands.len(), 1);
        let command = controller.pending_commands[0];
        assert_eq!(command.direction.to_array(), [0.6, 0.0, -0.8]);
        assert_eq!(command.desired_speed, 7.5);
        assert!(command.jump_requested);
    });
}
