use super::*;
use engine_scene::ComponentRecord;
use engine_serialize::SchemaVersion;

fn install_transform(scene: &mut Scene, entity_id: &str, fields: &[(&str, Value)]) {
    let entity = scene
        .entities
        .iter_mut()
        .find(|entity| entity.persistent_id == entity_id)
        .unwrap();
    entity.components.insert(
        TRANSFORM_COMPONENT_TYPE.into(),
        ComponentRecord {
            schema_version: SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: fields
                .iter()
                .map(|(name, value)| ((*name).to_string(), value.clone()))
                .collect(),
        },
    );
}

fn editor_scene_with_transform(fields: &[(&str, Value)]) -> EditorScene {
    let mut scene = engine_scene::sample_scene();
    install_transform(&mut scene, "cube-01", fields);
    let mut editor = EditorScene::new(scene);
    editor.selected_entity = Some("cube-01".into());
    editor
}

fn transform_field<'a>(editor: &'a EditorScene, field: &str) -> Option<&'a Value> {
    editor
        .scene
        .entities
        .iter()
        .find(|entity| entity.persistent_id == "cube-01")
        .and_then(|entity| entity.components.get(TRANSFORM_COMPONENT_TYPE))
        .and_then(|component| component.fields.get(field))
}

// ── GizmoSystem construction and field access ───────────────────

#[path = "tests/interaction.rs"]
mod interaction;
#[path = "tests/projection.rs"]
mod projection;
#[path = "tests/scene_gesture.rs"]
mod scene_gesture;
#[path = "tests/state.rs"]
mod state;
