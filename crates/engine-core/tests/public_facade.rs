use engine_core::game_loop::GameLoop;
use engine_core::{EngineConfig, EngineRuntime, EngineRuntimeBuilder, SceneLoadRequest};
use std::path::{Path, PathBuf};

fn collect_rust_sources(directory: &Path, sources: &mut Vec<PathBuf>) {
    let mut entries = std::fs::read_dir(directory)
        .expect("engine-core test module directory must be readable")
        .map(|entry| entry.expect("engine-core test module entry").path())
        .collect::<Vec<_>>();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect_rust_sources(&path, sources);
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("rs") {
            sources.push(path);
        }
    }
}

#[test]
fn root_runtime_facade_builds_and_loads_a_scene() {
    let config = EngineConfig {
        application_name: "public-facade-test".to_string(),
        ..EngineConfig::default()
    };
    let builder: EngineRuntimeBuilder = EngineRuntime::builder(config.clone());
    assert!(builder
        .component_registry()
        .is_registered("engine.transform"));

    let mut runtime = builder.build();
    assert_eq!(runtime.config(), &config);
    runtime
        .load_scene(engine_scene::sample_scene())
        .expect("the public runtime facade must load a valid scene");

    assert!(runtime.has_world());
    assert_eq!(
        runtime.with_world(|world| world.alive_count()),
        Some(engine_scene::sample_scene().entities.len())
    );

    let request = SceneLoadRequest {
        scene_id: "next".to_string(),
        requested_by: "public-facade-test".to_string(),
    };
    assert_eq!(request.scene_id, "next");
}

#[test]
fn game_loop_module_path_remains_a_public_host_api() {
    let mut game_loop = GameLoop::new(EngineConfig::default());
    game_loop
        .load_scene(engine_scene::sample_scene())
        .expect("the public game-loop facade must load a valid scene");

    game_loop
        .validate_ready()
        .expect("a loaded public game loop must be render-ready");
    assert_eq!(game_loop.world_origin(), [0.0; 3]);
    assert_eq!(game_loop.world_origin_shift_count(), 0);
}

#[test]
fn engine_core_test_contracts_stay_below_the_source_budget() {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for facade in [
        "game_loop/tests.rs",
        "cell_stream/tests.rs",
        "cooked_assets/tests.rs",
        "runtime_mesh_tests.rs",
    ] {
        let facade = source_root.join(facade);
        let mut sources = vec![facade.clone()];
        let fragment_directory = facade.with_extension("");
        assert!(
            fragment_directory.is_dir(),
            "{} must remain a facade over contract-focused test fragments",
            facade.display()
        );
        collect_rust_sources(&fragment_directory, &mut sources);

        for source in sources {
            let line_count = std::fs::read_to_string(&source)
                .unwrap_or_else(|error| panic!("could not read {}: {error}", source.display()))
                .lines()
                .count();
            assert!(
                line_count < 500,
                "{} grew to {line_count} lines; test files must remain below 500 lines",
                source.display()
            );
        }
    }
}

#[cfg(feature = "subsystem-ui")]
#[test]
fn retained_ui_event_types_remain_reexported_from_the_crate_root() {
    let value = engine_core::RuntimeUiValue::Bool(true);
    let event = engine_core::RuntimeUiEvent {
        canvas_id: "hud".to_string(),
        element_id: 1,
        callback_id: Some("start".to_string()),
        value: Some(value),
    };
    assert_eq!(event.callback_id.as_deref(), Some("start"));
}
