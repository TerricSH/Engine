#[test]
fn engine_runtime_diagnostics_collector() {
    let config = EngineConfig::default();
    let runtime = EngineRuntime::new(config);
    let collector = runtime.diagnostics_collector();
    assert!(collector.all().is_empty());
}

#[test]
fn engine_runtime_runtime_diagnostics() {
    let config = EngineConfig::default();
    let runtime = EngineRuntime::new(config);
    let rd = runtime.runtime_diagnostics();
    assert!(
        rd.script_engine_state.contains("coroutines=0"),
        "missing coroutines=0"
    );
    assert!(rd.reload_queue.is_none());
}

#[test]
fn strict_scene_load_installs_the_runtime_registry() {
    let _guard = serial_ffi_world_test();
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    let runtime_registry = std::sync::Arc::clone(runtime.component_registry());

    runtime
        .load_scene(engine_scene::sample_scene())
        .expect("sample scene should load");

    assert_eq!(
        runtime.with_world(|world| {
            std::sync::Arc::ptr_eq(
                world.component_registry().expect("world registry"),
                &runtime_registry,
            )
        }),
        Some(true)
    );
}

#[test]
fn unknown_component_failure_keeps_active_world_and_scene() {
    let _guard = serial_ffi_world_test();
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    let mut old_world = World::new();
    old_world.create_entity();
    runtime.set_world(old_world);
    let old_scene = runtime.scene_ref().cloned().expect("old scene snapshot");

    let mut invalid_scene = engine_scene::sample_scene();
    let entity_id = insert_empty_component(&mut invalid_scene, "third.party.missing");
    let diagnostics = runtime
        .load_scene(invalid_scene)
        .expect_err("unknown component must fail strict loading");

    assert_eq!(runtime.with_world(World::alive_count), Some(1));
    assert_eq!(runtime.scene_ref(), Some(&old_scene));
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "SC0030")
        .expect("mapped unknown-component diagnostic");
    assert_eq!(diagnostic.entity.as_deref(), Some(entity_id.as_str()));
    assert_eq!(
        diagnostic
            .fields
            .get("component_type_id")
            .map(String::as_str),
        Some("third.party.missing")
    );
    assert_eq!(
        diagnostic.path.as_deref(),
        Some(format!("entities[{entity_id}].components[third.party.missing]").as_str())
    );

    // The process-wide FFI bridge must still target the previous World.
    let spawned = engine_ffi::world_bridge::entity_spawn();
    assert_ne!(spawned, engine_ffi::types::FfiEntityId::INVALID);
    assert_eq!(runtime.with_world(World::alive_count), Some(2));
}

#[test]
fn validation_failures_keep_active_world_and_scene() {
    let _guard = serial_ffi_world_test();
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    let mut old_world = World::new();
    old_world.create_entity();
    runtime.set_world(old_world);
    let old_scene = runtime.scene_ref().cloned().expect("old scene snapshot");

    let mut duplicate = engine_scene::sample_scene();
    duplicate.entities.push(duplicate.entities[0].clone());
    let duplicate_diagnostics = runtime
        .load_scene(duplicate)
        .expect_err("duplicate entity must fail validation");
    assert!(duplicate_diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "SC0015"));
    assert_eq!(runtime.with_world(World::alive_count), Some(1));
    assert_eq!(runtime.scene_ref(), Some(&old_scene));

    let mut missing_parent = engine_scene::sample_scene();
    let mut orphan = missing_parent.entities[0].clone();
    orphan.persistent_id = "orphan".to_string();
    orphan.parent = Some("missing-parent".to_string());
    missing_parent.entities.push(orphan);
    let parent_diagnostics = runtime
        .load_scene(missing_parent)
        .expect_err("missing parent must fail validation");
    assert!(parent_diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "SC0016"));
    assert_eq!(runtime.with_world(World::alive_count), Some(1));
    assert_eq!(runtime.scene_ref(), Some(&old_scene));
}

#[test]
fn set_world_installs_runtime_registry_when_missing() {
    let _guard = serial_ffi_world_test();
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    let runtime_registry = std::sync::Arc::clone(runtime.component_registry());

    runtime.set_world(World::new());

    assert_eq!(
        runtime.with_world(|world| {
            std::sync::Arc::ptr_eq(
                world.component_registry().expect("world registry"),
                &runtime_registry,
            )
        }),
        Some(true)
    );
}

#[test]
fn set_world_preserves_an_existing_registry() {
    let _guard = serial_ffi_world_test();
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    let mut custom_registry = ComponentRegistry::new();
    register_a_only(&mut custom_registry);
    let custom_registry = std::sync::Arc::new(custom_registry);
    let mut world = World::new();
    world.set_shared_component_registry(std::sync::Arc::clone(&custom_registry));

    runtime.set_world(world);

    assert_eq!(
        runtime.with_world(|world| {
            std::sync::Arc::ptr_eq(
                world.component_registry().expect("world registry"),
                &custom_registry,
            )
        }),
        Some(true)
    );
    assert!(engine_ffi::component::lookup_component_type("A Only").is_some());
    assert!(engine_ffi::component::lookup_component_type("Character Controller").is_none());
}

#[test]
fn engine_runtime_can_replace_the_active_world_repeatedly() {
    let _guard = serial_ffi_world_test();
    let mut runtime = EngineRuntime::new(EngineConfig::default());

    let mut first = World::new();
    first.create_entity();
    runtime.set_world(first);

    runtime.set_world(World::new());
    let spawned = engine_ffi::world_bridge::entity_spawn();
    assert_ne!(spawned, engine_ffi::types::FfiEntityId::INVALID);
    assert_eq!(runtime.with_world(World::alive_count), Some(1));
}

#[test]
fn moving_runtime_keeps_ffi_world_binding_valid() {
    let _guard = serial_ffi_world_test();
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.set_world(World::new());

    let mut runtimes = vec![runtime];
    let moved_runtime = runtimes.pop().expect("moved runtime");

    let spawned = engine_ffi::world_bridge::entity_spawn();
    assert_ne!(spawned, engine_ffi::types::FfiEntityId::INVALID);
    assert_eq!(moved_runtime.with_world(World::alive_count), Some(1));
}

#[test]
fn dropping_runtime_makes_its_ffi_world_unavailable() {
    let _guard = serial_ffi_world_test();
    {
        let mut runtime = EngineRuntime::new(EngineConfig::default());
        runtime.set_world(World::new());
    }

    assert_eq!(
        engine_ffi::world_bridge::entity_spawn(),
        engine_ffi::types::FfiEntityId::INVALID
    );
}

#[test]
fn dropping_old_runtime_does_not_deactivate_new_runtime() {
    let _guard = serial_ffi_world_test();
    let mut old_runtime = EngineRuntime::new(EngineConfig::default());
    old_runtime.set_world(World::new());
    let mut current_runtime = EngineRuntime::new(EngineConfig::default());
    current_runtime.set_world(World::new());

    drop(old_runtime);
    let spawned = engine_ffi::world_bridge::entity_spawn();
    assert_ne!(spawned, engine_ffi::types::FfiEntityId::INVALID);
    assert_eq!(current_runtime.with_world(World::alive_count), Some(1));
}

#[test]
fn canonical_scene_load_replaces_and_activates_the_world() {
    let _guard = serial_ffi_world_test();
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.set_world(World::new());
    runtime
        .load_scene(engine_scene::sample_scene())
        .expect("sample scene should load into a World");

    assert!(runtime.has_world());
    assert_ne!(
        engine_ffi::world_bridge::entity_spawn(),
        engine_ffi::types::FfiEntityId::INVALID
    );
}
