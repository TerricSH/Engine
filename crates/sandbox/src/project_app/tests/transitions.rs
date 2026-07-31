#[test]
fn project_scene_transition_loads_catalog_scene_and_rejects_unknown_id() {
    let (_temp, project) = scene_project_fixture();
    let mut game_loop = GameLoop::new(EngineConfig::default());
    game_loop
        .load_scene(Scene::load_from_file(project.startup_scene_path()).unwrap())
        .unwrap();

    let unknown = SceneLoadRequest {
        scene_id: "missing".into(),
        requested_by: "cube-01".into(),
    };
    assert!(transition_to_project_scene(&mut game_loop, &project, &unknown).is_err());
    assert_eq!(
        game_loop
            .runtime
            .scene_ref()
            .map(|scene| scene.scene_id.as_str()),
        Some("main")
    );

    let request = SceneLoadRequest {
        scene_id: "level_two".into(),
        requested_by: "cube-01".into(),
    };
    transition_to_project_scene(&mut game_loop, &project, &request).unwrap();
    assert_eq!(
        game_loop
            .runtime
            .scene_ref()
            .map(|scene| scene.scene_id.as_str()),
        Some("level_two")
    );
    #[cfg(feature = "subsystem-physics")]
    assert!(game_loop.physics.is_some());
}

#[test]
fn failed_post_load_validation_restores_the_previous_scene() {
    let mut game_loop = GameLoop::new(EngineConfig::default());
    let mut previous = engine_scene::sample_scene();
    previous.scene_id = "previous".into();
    previous
        .entities
        .iter_mut()
        .find(|entity| entity.persistent_id == "cube-01")
        .expect("sample scene cube")
        .components
        .insert(
            "engine.transform".into(),
            engine_scene::ComponentRecord {
                schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: BTreeMap::from([
                    (
                        "translation".into(),
                        engine_serialize::Value::Vec3([0.0; 3]),
                    ),
                    (
                        "rotation".into(),
                        engine_serialize::Value::Quat([0.0, 0.0, 0.0, 1.0]),
                    ),
                    ("scale".into(), engine_serialize::Value::Vec3([1.0; 3])),
                ]),
            },
        );
    game_loop.load_scene(previous).unwrap();
    game_loop
        .runtime
        .with_world_mut(|world| {
            let entity = world.entity_by_persistent_id("cube-01").unwrap();
            world
                .get_mut::<engine_scene::components::Transform>(entity)
                .unwrap()
                .translation
                .x = 42.0;
        })
        .unwrap();
    let rollback_snapshot = capture_scene_transition_rollback(&game_loop.runtime).unwrap();

    let mut rejected = engine_scene::sample_scene();
    rejected.scene_id = "rejected".into();
    game_loop.load_scene(rejected).unwrap();

    let error = rollback_failed_scene_transition(
        &mut game_loop,
        Some(rollback_snapshot),
        "post-load validation failed".into(),
    )
    .unwrap_err();
    assert!(error.contains("previous scene was restored"));
    assert_eq!(
        game_loop
            .runtime
            .scene_ref()
            .map(|scene| scene.scene_id.as_str()),
        Some("previous")
    );
    assert_eq!(
        game_loop.runtime.with_world(|world| {
            let entity = world.entity_by_persistent_id("cube-01").unwrap();
            world
                .get::<engine_scene::components::Transform>(entity)
                .unwrap()
                .translation
                .x
        }),
        Some(42.0)
    );
}

#[test]
fn failed_rollback_is_classified_as_fatal() {
    let mut game_loop = GameLoop::new(EngineConfig::default());
    let mut active = engine_scene::sample_scene();
    active.scene_id = "rejected".into();
    game_loop.load_scene(active).unwrap();

    let mut invalid_previous = engine_scene::sample_scene();
    invalid_previous.scene_id = "previous".into();
    invalid_previous.entities[0].components.insert(
        "engine.unknown_component".into(),
        engine_scene::ComponentRecord {
            schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: BTreeMap::new(),
        },
    );
    let failure = rollback_failed_scene_transition_classified(
        &mut game_loop,
        Some(invalid_previous),
        "post-load validation failed".into(),
    )
    .unwrap_err();
    assert!(matches!(failure, SceneTransitionFailure::Fatal(_)));
    assert!(failure
        .into_message()
        .contains("restoring the previous scene also failed"));
    assert_eq!(
        game_loop
            .runtime
            .scene_ref()
            .map(|scene| scene.scene_id.as_str()),
        Some("rejected")
    );
}

#[test]
fn runtime_dependency_check_includes_extension_asset_fields() {
    let mut scene = engine_scene::sample_scene();
    scene.entities[0].components.insert(
        "engine.audio_source".into(),
        engine_scene::ComponentRecord {
            schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: BTreeMap::from([(
                "clip_asset".into(),
                engine_serialize::Value::Asset(engine_serialize::AssetId::new("audio.missing")),
            )]),
        },
    );
    scene.entities[0].components.insert(
        "engine.canvas".into(),
        engine_scene::ComponentRecord {
            schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: BTreeMap::from([(
                "elements".into(),
                engine_serialize::Value::List(vec![engine_serialize::Value::Map(BTreeMap::from(
                    [(
                        "texture".into(),
                        engine_serialize::Value::Asset(engine_serialize::AssetId::new("ui.image")),
                    )],
                ))]),
            )]),
        },
    );
    let mut runtime = EngineRuntime::new(EngineConfig::default());

    let missing = missing_runtime_asset_dependencies(&runtime, &scene);
    assert!(missing.iter().any(|asset| asset.id == "audio.missing"));
    assert!(missing.iter().any(|asset| asset.id == "ui.image"));

    runtime
        .asset_registry_mut()
        .insert_typed(engine_serialize::AssetId::new("audio.missing"), vec![0u8]);
    runtime
        .asset_registry_mut()
        .insert_typed(engine_serialize::AssetId::new("ui.image"), vec![0u8]);
    assert!(missing_runtime_asset_dependencies(&runtime, &scene).is_empty());
}

#[cfg(feature = "subsystem-terrain")]
fn configure_planet_scene_transition(game_loop: &mut GameLoop) {
    game_loop
        .runtime
        .with_world_mut(|world| {
            let planet = world.create_persistent_entity("planet-a").unwrap();
            world.add_component(
                planet,
                engine_terrain::TerrainVolume {
                    enabled: true,
                    topology: engine_terrain::TerrainTopology::CubeSphere,
                    base_resolution: 3,
                    height_scale: 0.0,
                    planet_radius: 1_000.0,
                    planet_max_lod: 0,
                    lod_distances: vec![1_000.0],
                    ..engine_terrain::TerrainVolume::default()
                },
            );
            let controller = world
                .create_persistent_entity("planet-a-transition")
                .unwrap();
            world.add_component(
                controller,
                engine_terrain::PlanetSceneTransitionConfig {
                    enabled: true,
                    terrain_volume_id: "planet-a".into(),
                    orbit_scene_id: "main".into(),
                    surface_scene_id: "level_two".into(),
                    enter_surface_altitude: 150.0,
                    exit_surface_altitude: 250.0,
                    minimum_dwell_seconds: 0.0,
                },
            );
            let camera = world.entity_by_persistent_id("camera-main").unwrap();
            world.add_component(
                camera,
                engine_scene::components::Transform {
                    translation: glam::Vec3::new(0.0, 1_100.0, 0.0),
                    ..engine_scene::components::Transform::default()
                },
            );
        })
        .unwrap();
}

#[cfg(feature = "subsystem-terrain")]
#[test]
fn project_host_commits_landing_and_ascent_only_after_scene_loads() {
    let (_temp, project) = scene_project_fixture();
    let mut game_loop = GameLoop::new(EngineConfig::default());
    game_loop
        .load_scene(Scene::load_from_file(project.startup_scene_path()).unwrap())
        .unwrap();
    configure_planet_scene_transition(&mut game_loop);
    let mut current_scene_id = "main".to_string();

    game_loop.tick_planet_scene_transitions(0.0);
    assert_eq!(
        process_pending_scene_transitions(&mut game_loop, &project, &mut current_scene_id).unwrap(),
        1
    );
    assert_eq!(current_scene_id, "level_two");
    assert_eq!(
        game_loop
            .runtime
            .scene_ref()
            .map(|scene| scene.scene_id.as_str()),
        Some("level_two")
    );

    game_loop
        .runtime
        .with_world_mut(|world| {
            let camera = world.entity_by_persistent_id("camera-main").unwrap();
            world.add_component(
                camera,
                engine_scene::components::Transform {
                    translation: glam::Vec3::new(0.0, 1_300.0, 0.0),
                    ..engine_scene::components::Transform::default()
                },
            );
        })
        .unwrap();
    game_loop.tick_planet_scene_transitions(0.0);
    assert_eq!(
        process_pending_scene_transitions(&mut game_loop, &project, &mut current_scene_id).unwrap(),
        1
    );
    assert_eq!(current_scene_id, "main");
    assert_eq!(
        game_loop
            .runtime
            .scene_ref()
            .map(|scene| scene.scene_id.as_str()),
        Some("main")
    );
}

#[cfg(feature = "subsystem-terrain")]
#[test]
fn failed_planet_scene_load_preserves_current_scene_and_retries() {
    let (_temp, project) = scene_project_fixture();
    let mut game_loop = GameLoop::new(EngineConfig::default());
    game_loop
        .load_scene(Scene::load_from_file(project.startup_scene_path()).unwrap())
        .unwrap();
    configure_planet_scene_transition(&mut game_loop);
    let mut current_scene_id = "main".to_string();
    let surface_path = project.scene_path("level_two").unwrap();
    std::fs::write(&surface_path, "not a scene").unwrap();

    game_loop.tick_planet_scene_transitions(0.0);
    assert_eq!(
        process_pending_scene_transitions(&mut game_loop, &project, &mut current_scene_id).unwrap(),
        0
    );
    assert_eq!(current_scene_id, "main");
    assert_eq!(
        game_loop
            .runtime
            .scene_ref()
            .map(|scene| scene.scene_id.as_str()),
        Some("main")
    );

    let mut repaired = engine_scene::sample_scene();
    repaired.scene_id = "level_two".into();
    repaired.save_to_file(&surface_path).unwrap();
    game_loop.tick_planet_scene_transitions(0.0);
    assert_eq!(
        process_pending_scene_transitions(&mut game_loop, &project, &mut current_scene_id).unwrap(),
        1
    );
    assert_eq!(current_scene_id, "level_two");
    assert_eq!(
        game_loop
            .runtime
            .scene_ref()
            .map(|scene| scene.scene_id.as_str()),
        Some("level_two")
    );
}

#[cfg(feature = "subsystem-terrain")]
#[test]
fn fatal_planet_transition_failure_is_rejected_and_propagated() {
    let (_temp, project) = scene_project_fixture();
    let mut game_loop = GameLoop::new(EngineConfig::default());
    game_loop
        .load_scene(Scene::load_from_file(project.startup_scene_path()).unwrap())
        .unwrap();
    configure_planet_scene_transition(&mut game_loop);
    game_loop.tick_planet_scene_transitions(0.0);
    let ticket = game_loop
        .take_pending_planet_scene_transition()
        .expect("landing ticket");
    let mut current_scene_id = "main".to_string();

    let error = settle_planet_scene_transition(
        &mut game_loop,
        &mut current_scene_id,
        ticket.clone(),
        Err(SceneTransitionFailure::Fatal("rollback failed".into())),
    )
    .unwrap_err();
    assert!(error.contains("left scene state uncertain"));
    assert_eq!(current_scene_id, "main");

    game_loop.tick_planet_scene_transitions(0.0);
    let retry = game_loop
        .take_pending_planet_scene_transition()
        .expect("fatal outcome still rejected the stale ticket");
    assert_ne!(retry.transaction_id, ticket.transaction_id);
}
