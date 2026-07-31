// ── EngineConfig tests ───────────────────────────────────────────────

#[test]
fn engine_config_defaults() {
    let config = EngineConfig::default();
    assert_eq!(config.application_name, "engine");
}

#[test]
fn engine_config_debug() {
    let config = EngineConfig::default();
    let debug = format!("{:?}", config);
    assert!(debug.contains("EngineConfig"));
}

#[test]
fn engine_config_partial_eq() {
    let a = EngineConfig::default();
    let b = EngineConfig::default();
    let c = EngineConfig {
        application_name: "custom".to_string(),
        gpu_timestamps: true,
    };
    assert_eq!(a, b);
    assert_ne!(a, c);
}

#[test]
fn engine_config_clone() {
    let config = EngineConfig::default();
    let cloned = config.clone();
    assert_eq!(config, cloned);
}

// ── EngineRuntime tests ──────────────────────────────────────────────

#[test]
fn engine_runtime_creation() {
    let config = EngineConfig::default();
    let runtime = EngineRuntime::new(config.clone());
    assert_eq!(*runtime.config(), config);
}

#[test]
fn runtime_builder_registers_character_extensions_by_default() {
    let builder = EngineRuntimeBuilder::default();
    assert!(builder
        .component_registry()
        .is_registered("engine.character_controller"));
}

#[test]
fn runtime_builder_registers_vfx_extensions_by_default() {
    let builder = EngineRuntimeBuilder::default();
    assert!(builder
        .component_registry()
        .is_registered("engine.vfx.particle_emitter"));
    assert!(builder
        .component_registry()
        .is_registered("engine.vfx.decal"));
}

#[test]
fn runtime_extracts_vfx_and_syncs_builtin_surface_assets() {
    let _guard = serial_ffi_world_test();
    let uploads = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.set_renderer_backend(Box::new(RecordingBackend {
        uploads: std::sync::Arc::clone(&uploads),
        rendered_ui_batch_counts: None,
    }));
    runtime
        .load_scene(engine_scene::sample_scene())
        .expect("sample scene loads");
    runtime
        .with_world_mut(|world| {
            let cube = world.entity_by_persistent_id("cube-01").unwrap();
            world.add_component(cube, engine_scene::components::Transform::default());
            world.add_component(cube, engine_vfx::Decal::default());
        })
        .unwrap();

    runtime.render_frame(0).expect("VFX frame renders");

    let uploads = uploads
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert!(uploads.iter().any(|entry| entry == "mesh:mesh-vfx-quad"));
    assert!(uploads
        .iter()
        .any(|entry| entry == "material:mat-vfx-default"));
}

#[cfg(feature = "subsystem-physics")]
#[test]
fn runtime_builder_registers_physics_extensions_with_physics_leaf() {
    let builder = EngineRuntimeBuilder::default();
    assert!(builder
        .component_registry()
        .is_registered("engine.physics.rigid_body"));
    assert!(builder
        .component_registry()
        .is_registered("engine.physics.collider"));
    assert!(builder
        .component_registry()
        .is_registered("engine.physics.physics_material"));
}

#[cfg(all(
    feature = "subsystem-animation",
    feature = "subsystem-audio",
    feature = "subsystem-navigation",
    feature = "subsystem-ui"
))]
#[test]
fn runtime_builder_registers_runtime_subsystem_extensions() {
    let builder = EngineRuntimeBuilder::default();
    for component in [
        "engine.canvas",
        "engine.audio_source",
        "engine.audio_listener",
        "engine.animation_player",
        "engine.skeleton",
        "engine.ik_target",
        "engine.nav_agent",
    ] {
        assert!(
            builder.component_registry().is_registered(component),
            "missing component extension {component}"
        );
    }
    for asset_type in [
        "audio_clip",
        "skeleton",
        "animation_clip",
        "navmesh",
        "behavior",
    ] {
        assert!(
            builder.asset_type_registry().get(asset_type).is_some(),
            "missing asset type extension {asset_type}"
        );
    }
    assert_eq!(builder.render_extension_registry().producer_count(), 1);
    assert!(builder.debug_draw_registry().provider_count() >= 3);
    assert_eq!(
        builder
            .animation_extension_handles()
            .skinned_extract
            .pending_count(),
        0
    );
}

#[cfg(all(
    feature = "subsystem-animation",
    feature = "subsystem-audio",
    feature = "subsystem-navigation",
    feature = "subsystem-ui"
))]
#[test]
fn runtime_subsystem_components_survive_strict_scene_loading() {
    let _guard = serial_ffi_world_test();
    let mut scene = engine_scene::sample_scene();
    for component in [
        "engine.canvas",
        "engine.audio_source",
        "engine.audio_listener",
        "engine.animation_player",
        "engine.skeleton",
        "engine.ik_target",
        "engine.nav_agent",
    ] {
        insert_empty_component(&mut scene, component);
    }

    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime
        .load_scene(scene)
        .expect("registered runtime subsystem components should load strictly");

    runtime
        .with_world(|world| {
            assert_eq!(world.query::<engine_ui::Canvas>().count(), 1);
            assert_eq!(
                world
                    .query::<engine_audio::components::AudioSourceComponent>()
                    .count(),
                1
            );
            assert_eq!(
                world.query::<engine_animation::AnimationPlayer>().count(),
                1
            );
            assert_eq!(world.query::<engine_nav::AiAgent>().count(), 1);
        })
        .expect("strict load should install a World");
}

#[cfg(not(any(
    feature = "subsystem-animation",
    feature = "subsystem-audio",
    feature = "subsystem-navigation",
    feature = "subsystem-ui"
)))]
#[test]
fn minimal_runtime_does_not_install_optional_subsystems() {
    let builder = EngineRuntimeBuilder::default();
    assert!(!builder.component_registry().is_registered("engine.canvas"));
    assert!(!builder
        .component_registry()
        .is_registered("engine.audio_source"));
    assert!(!builder
        .component_registry()
        .is_registered("engine.animation_player"));
    assert!(!builder
        .component_registry()
        .is_registered("engine.nav_agent"));
    assert!(builder.asset_type_registry().get("audio_clip").is_none());
    assert_eq!(builder.render_extension_registry().producer_count(), 0);
}

#[test]
fn runtime_invokes_registered_render_extensions_before_drawing() {
    let _guard = serial_ffi_world_test();
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut builder = EngineRuntimeBuilder::default();
    builder
        .render_extension_registry_mut()
        .register(Box::new(CountingRenderExtension {
            calls: std::sync::Arc::clone(&calls),
        }));
    let mut runtime = builder.build();
    runtime.set_renderer_backend(Box::new(RecordingBackend {
        uploads: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
        rendered_ui_batch_counts: None,
    }));
    runtime
        .load_scene(engine_scene::sample_scene())
        .expect("sample scene should load");

    runtime.render_frame(17).expect("frame should render");

    assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 1);
}
