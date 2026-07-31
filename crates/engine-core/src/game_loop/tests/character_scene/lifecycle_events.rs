#[test]
fn loading_a_scene_without_a_character_clears_previous_binding() {
    let mut scene = engine_scene::sample_scene();
    scene.entities[0].components.insert(
        "engine.character_controller".into(),
        engine_scene::ComponentRecord {
            schema_version: SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: BTreeMap::new(),
        },
    );
    let mut game_loop = GameLoop::new(EngineConfig::default());
    game_loop.load_scene(scene).unwrap();
    assert!(game_loop.character.is_some());

    game_loop.load_scene(engine_scene::sample_scene()).unwrap();
    assert!(game_loop.character.is_none());
    assert!(game_loop.character_entity.is_none());
}

#[test]
fn failed_scene_load_keeps_previous_gameplay_bindings() {
    let mut scene = engine_scene::sample_scene();
    scene.entities[0].components.insert(
        "engine.character_controller".into(),
        engine_scene::ComponentRecord {
            schema_version: SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: BTreeMap::new(),
        },
    );
    let mut game_loop = GameLoop::new(EngineConfig::default());
    game_loop.load_scene(scene).unwrap();
    let previous_entity = game_loop.character_entity;
    let previous_position = game_loop.character.as_ref().unwrap().position();
    assert!(game_loop.physics.is_some());

    let mut invalid = engine_scene::sample_scene();
    invalid.entities.push(invalid.entities[0].clone());
    assert!(game_loop.load_scene(invalid).is_err());

    assert_eq!(game_loop.character_entity, previous_entity);
    assert_eq!(
        game_loop.character.as_ref().unwrap().position(),
        previous_position
    );
    assert!(game_loop.physics.is_some());
    assert_eq!(
        game_loop
            .runtime
            .with_world(|world| world.entity_by_persistent_id("camera-main").is_some()),
        Some(true)
    );
}

#[test]
fn physics_events_are_exposed_for_one_frame_and_drained_from_the_backend() {
    use engine_physics::{BodyType, Collider, RigidBody};
    use engine_scene::components::Transform;

    let mut game_loop = GameLoop::new(EngineConfig::default());
    game_loop.load_scene(engine_scene::sample_scene()).unwrap();
    game_loop.runtime.with_world_mut(|world| {
        let dynamic = world.entity_by_persistent_id("cube-01").unwrap();
        let fixed = world.entity_by_persistent_id("camera-main").unwrap();
        world.add_component(dynamic, Transform::default());
        world.add_component(fixed, Transform::default());
        world.add_component(dynamic, RigidBody::default());
        world.add_component(dynamic, Collider::default());
        world.add_component(
            fixed,
            RigidBody {
                body_type: BodyType::Static,
                ..RigidBody::default()
            },
        );
        world.add_component(fixed, Collider::default());
    });
    game_loop.init_physics();

    game_loop.update(1.0 / 30.0);

    assert!(!game_loop.physics_events().is_empty());
    assert!(game_loop
        .physics
        .as_ref()
        .unwrap()
        .pending_events()
        .is_empty());
    assert!(game_loop
        .physics
        .as_ref()
        .unwrap()
        .pending_triggers()
        .is_empty());
    let events = game_loop.take_physics_events();
    assert!(!events.is_empty());
    assert!(game_loop.physics_events().is_empty());

    game_loop.update(0.0);
    assert!(game_loop.physics_events().is_empty());
}
