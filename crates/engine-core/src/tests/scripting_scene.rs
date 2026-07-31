#[cfg(feature = "subsystem-scripting-csharp")]
#[test]
fn engine_runtime_load_scene_with_scripts() {
    let _guard = serial_ffi_world_test();
    use engine_scene::ComponentRecord;
    use engine_script::MockHost;
    use engine_serialize::SchemaVersion;
    use std::collections::BTreeMap;

    let config = EngineConfig::default();
    let mut runtime = EngineRuntime::new(config);
    runtime.register_script_host(Box::new(MockHost::new()));
    // Match the host name used by MockHost
    runtime.set_script_host_name("mock");

    // Create a minimal scene with a script component
    let mut script_fields = BTreeMap::new();
    script_fields.insert(
        "assembly_id".into(),
        engine_serialize::Value::Str("asm".into()),
    );
    script_fields.insert(
        "class_name".into(),
        engine_serialize::Value::Str("MyScript".into()),
    );

    let mut components = BTreeMap::new();
    components.insert(
        "engine.script".to_string(),
        ComponentRecord {
            schema_version: SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: script_fields,
        },
    );

    let scene = engine_scene::Scene {
        schema_version: SchemaVersion::new(0, 1, 0),
        engine_version: "0.1.0".to_string(),
        scene_id: "test".to_string(),
        name: "test".to_string(),
        entities: vec![engine_scene::EntityRecord {
            persistent_id: "ent-1".to_string(),
            parent: None,
            name: Some("Entity".to_string()),
            enabled: true,
            components,
        }],
        scene_settings: engine_scene::SceneSettings::default(),
        dependencies: vec![],
        diagnostics_policy: engine_scene::DiagnosticsPolicy::Strict,
    };

    // Pre-load the assembly that the script references
    runtime
        .load_script_assembly("asm", "mock", b"mock_data")
        .unwrap();

    // Load scene — should attach scripts
    runtime
        .load_scene(scene.clone())
        .expect("engine.script metadata should be allowed");

    // After load_scene, the script engine should have an instance
    assert_eq!(runtime.scripting.engine.host_count(), 1);
    let after = runtime.scripting.engine.managers()[0].instance_count();
    assert_eq!(after, 1, "script instance should have been created");

    runtime
        .load_scene(scene)
        .expect("reloading a scripted scene should replace its instances");
    assert_eq!(
        runtime.scripting.engine.managers()[0].instance_count(),
        1,
        "scene reload must not accumulate duplicate script instances"
    );

    // Tick should not produce errors
    runtime.tick_scripts(0.016);
}

#[cfg(feature = "subsystem-scripting-csharp")]
#[test]
fn script_feature_does_not_ignore_other_unknown_component_types() {
    let _guard = serial_ffi_world_test();
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    let mut scene = engine_scene::sample_scene();
    insert_empty_component(&mut scene, "engine.script::assembly");

    let diagnostics = runtime
        .load_scene(scene)
        .expect_err("only the exact engine.script type is scene-only");

    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "SC0030"
            && diagnostic
                .fields
                .get("component_type_id")
                .is_some_and(|type_id| type_id == "engine.script::assembly")
    }));
    assert!(!runtime.has_world());
    assert!(runtime.scene_ref().is_none());
}

#[cfg(feature = "subsystem-scripting-csharp")]
#[test]
fn script_component_access_levels_drive_query_and_write_diagnostics() {
    let _guard = serial_ffi_world_test();
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.set_world(World::from_scene(&engine_scene::sample_scene()));

    let commands = vec![
        // ReadOnly component: the write is rejected with the distinct
        // read-only diagnostic, never applied.
        engine_script::OwnedGameplayCommand {
            entity_id: "cube-01".into(),
            command: GameplayCommand::SetComponent {
                entity_id: "cube-01".into(),
                component_type: "engine.character_controller".into(),
                fields: std::collections::BTreeMap::from([(
                    "move_speed".to_string(),
                    engine_script::GameplayComponentValue::Float(9.0),
                )]),
            },
        },
        // DedicatedApi component: same stable unknown-component
        // diagnostic as unregistered keys.
        engine_script::OwnedGameplayCommand {
            entity_id: "cube-01".into(),
            command: GameplayCommand::SetComponent {
                entity_id: "cube-01".into(),
                component_type: "engine.transform".into(),
                fields: std::collections::BTreeMap::from([(
                    "translation".to_string(),
                    engine_script::GameplayComponentValue::Vec3([0.0; 3]),
                )]),
            },
        },
        // ReadOnly component queries are accepted: no diagnostic.
        engine_script::OwnedGameplayCommand {
            entity_id: "cube-01".into(),
            command: GameplayCommand::ComponentQuery {
                query: engine_script::GameplayComponentQuery {
                    query_id: 7,
                    entity_id: "cube-01".into(),
                    component_type: "engine.character_controller".into(),
                },
            },
        },
    ];

    let diagnostics = runtime.apply_script_gameplay_commands(commands);

    let read_only = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "SCRIPT_COMPONENT_READ_ONLY")
        .collect::<Vec<_>>();
    assert_eq!(read_only.len(), 1, "{diagnostics:?}");
    assert!(read_only[0].message.contains("engine.character_controller"));

    let unknown = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "SCRIPT_COMPONENT_UNKNOWN")
        .collect::<Vec<_>>();
    assert_eq!(unknown.len(), 1, "{diagnostics:?}");
    assert!(unknown[0].message.contains("engine.transform"));
    // The supported list in the diagnostic is registry-driven and now
    // includes the read-only character controller.
    assert!(unknown[0].message.contains("engine.character_controller"));
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != "SCRIPT_COMPONENT_PAYLOAD_INVALID"),
        "{diagnostics:?}"
    );
}
