#[test]
fn game_loop_advances_non_primary_character_controllers() {
    let mut scene = engine_scene::sample_scene();
    for entity in &mut scene.entities {
        entity.components.insert(
            "engine.transform".into(),
            engine_scene::ComponentRecord {
                schema_version: SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: BTreeMap::new(),
            },
        );
        entity.components.insert(
            "engine.character_controller".into(),
            engine_scene::ComponentRecord {
                schema_version: SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: BTreeMap::from([("gravity_scale".into(), Value::Float32(0.0))]),
            },
        );
    }
    let mut game_loop = GameLoop::new(EngineConfig::default());
    game_loop.load_scene(scene).unwrap();
    let primary = game_loop.character_entity.unwrap();
    let secondary = game_loop
        .runtime
        .with_world(|world| {
            world
                .query::<CharacterController>()
                .map(|(entity, _)| entity)
                .find(|entity| *entity != primary)
                .unwrap()
        })
        .unwrap();
    game_loop
        .runtime
        .with_world_mut(|world| {
            world
                .get_mut::<CharacterController>(secondary)
                .unwrap()
                .push_command(engine_character::CharacterCommand::move_towards(Vec3::X));
        })
        .unwrap();

    game_loop.update(0.1);

    assert!(
        game_loop
            .runtime
            .with_world(|world| world
                .get::<engine_scene::components::Transform>(secondary)
                .unwrap()
                .translation
                .x)
            .unwrap()
            > 0.0
    );
}

#[cfg(all(
    feature = "subsystem-animation",
    feature = "subsystem-audio",
    feature = "subsystem-navigation",
    feature = "subsystem-ui"
))]
#[test]
fn loaded_navmesh_drives_a_scene_character_through_the_standard_game_loop() {
    use engine_asset::cook::{registered_asset_type_id, AssetType};

    let cooked = std::env::temp_dir().join(format!(
        "engine_core_game_loop_navigation_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&cooked);
    std::fs::create_dir_all(&cooked).unwrap();

    let mut game_loop = GameLoop::new(EngineConfig::default());
    let mut navmesh = engine_nav::NavMesh::new();
    let a = navmesh.add_vertex(Vec3::new(-10.0, 0.0, -10.0));
    let b = navmesh.add_vertex(Vec3::new(10.0, 0.0, -10.0));
    let c = navmesh.add_vertex(Vec3::new(0.0, 0.0, 10.0));
    navmesh.add_polygon(&[a, b, c], 1.0);
    navmesh.rebuild_bvh();
    let extension = game_loop
        .runtime
        .asset_type_registry()
        .get(registered_asset_type_id(&AssetType::NavMesh).unwrap())
        .unwrap();
    let mut payload = Vec::new();
    extension.cooker.unwrap()(&bincode::serialize(&navmesh).unwrap(), &mut payload).unwrap();
    engine_asset::cook::write_cooked_artifact(
        &cooked.join("level.navmesh.cooked"),
        AssetType::NavMesh.kind_code(),
        &payload,
        SchemaVersion::new(0, 1, 0),
    )
    .unwrap();
    game_loop.runtime.load_cooked_assets(&cooked).unwrap();

    let mut scene = engine_scene::sample_scene();
    let cube = scene
        .entities
        .iter_mut()
        .find(|entity| entity.persistent_id == "cube-01")
        .unwrap();
    cube.components.insert(
        "engine.transform".into(),
        engine_scene::ComponentRecord {
            schema_version: SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: BTreeMap::new(),
        },
    );
    cube.components.insert(
        "engine.character_controller".into(),
        engine_scene::ComponentRecord {
            schema_version: SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: BTreeMap::from([("gravity_scale".into(), Value::Float32(0.0))]),
        },
    );
    game_loop.load_scene(scene).unwrap();
    game_loop
        .runtime
        .with_world_mut(|world| {
            let entity = world.entity_by_persistent_id("cube-01").unwrap();
            let mut agent = engine_nav::AiAgent::new();
            agent.navmesh_ref = Some("level.navmesh".into());
            agent.target = Some(Vec3::new(0.0, 0.0, 5.0));
            agent.speed = 2.0;
            world.add_component(entity, agent);
        })
        .unwrap();

    game_loop.update(0.1);

    let translation = game_loop
        .runtime
        .with_world(|world| {
            let entity = world.entity_by_persistent_id("cube-01").unwrap();
            world
                .get::<engine_scene::components::Transform>(entity)
                .unwrap()
                .translation
        })
        .unwrap();
    assert!(
        translation.x * translation.x + translation.z * translation.z > 0.0,
        "navigation intent did not move the character: {translation:?}"
    );
    let _ = std::fs::remove_dir_all(cooked);
}
