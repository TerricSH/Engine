use std::collections::BTreeMap;

use engine_gameplay::{InputAction, InputValue, InputValueType};
use engine_serialize::{SchemaVersion, Value};

use super::*;

#[test]
fn scene_character_component_binds_and_uses_standard_movement_actions() {
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
            fields: BTreeMap::from([
                ("translation".into(), Value::Vec3([3.0, 0.0, 2.0])),
                ("rotation".into(), Value::Quat([0.0, 0.0, 0.0, 1.0])),
                ("scale".into(), Value::Vec3([1.0; 3])),
            ]),
        },
    );
    target.components.insert(
        "engine.character_controller".into(),
        engine_scene::ComponentRecord {
            schema_version: SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: BTreeMap::from([
                ("position".into(), Value::Vec3([0.0; 3])),
                ("gravity_scale".into(), Value::Float32(0.0)),
                ("air_acceleration".into(), Value::Float32(10.0)),
                ("state".into(), Value::Enum("Falling".into())),
            ]),
        },
    );

    let mut game_loop = GameLoop::new(EngineConfig::default());
    game_loop.load_scene(scene).unwrap();
    assert!(game_loop.character.is_some());
    assert_eq!(
        game_loop.runtime.with_world(|world| game_loop
            .character_entity
            .and_then(|entity| world.persistent_id(entity).map(str::to_string))),
        Some(Some("cube-01".into()))
    );

    let mut forward = InputAction::new("move_forward", InputValueType::Digital);
    forward.current_value = InputValue::Bool(true);
    game_loop.input_map.add_action(forward);
    game_loop.update(0.1);

    let (transform_position, component_position) = game_loop
        .runtime
        .with_world(|world| {
            let entity = world.entity_by_persistent_id("cube-01").unwrap();
            let transform = world
                .get::<engine_scene::components::Transform>(entity)
                .unwrap();
            let controller = world.get::<CharacterController>(entity).unwrap();
            (transform.translation, controller.position())
        })
        .unwrap();
    assert!(transform_position.z < 2.0, "{transform_position:?}");
    assert_eq!(component_position, transform_position);
}
