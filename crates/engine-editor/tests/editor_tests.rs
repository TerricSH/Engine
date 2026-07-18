use engine_editor::*;

// ── EditorError tests ────────────────────────────────────────────────

#[test]
fn editor_error_scene_not_found_display() {
    let err = EditorError::SceneNotFound;
    assert_eq!(err.to_string(), "scene not found");
}

#[test]
fn editor_error_init_failed_display() {
    let err = EditorError::InitFailed("missing config".to_string());
    assert_eq!(err.to_string(), "init failed: missing config");
}

// Cross-platform editor state tests

#[cfg(feature = "tooling-editor")]
#[test]
fn scene_view_state_is_model_only() {
    let mut panel = SceneViewPanel::new("Scene");
    panel.apply_action(SceneViewAction::SetDistance(25.0));
    assert_eq!(panel.camera_orbit().2, 25.0);
}

#[cfg(feature = "tooling-editor")]
#[test]
fn project_asset_browser_model_starts_empty() {
    let panel = engine_editor::asset_browser::AssetBrowserPanel::new();
    assert!(panel.assets().is_empty());
}
// ── Command tests ───────────────────────────────────────────────────

#[cfg(feature = "tooling-editor")]
#[test]
fn command_history_new_is_not_dirty() {
    let history = CommandHistory::new();
    assert!(!history.is_dirty());
    assert!(!history.can_undo());
    assert!(!history.can_redo());
}

#[cfg(feature = "tooling-editor")]
#[test]
fn set_entity_name_execute_and_undo() {
    let mut scene = engine_scene::sample_scene();
    let entity_id = "camera-main".to_string();

    let mut cmd = SetEntityName::new(entity_id.clone(), Some("Renamed".to_string()));
    cmd.execute(&mut scene).unwrap();

    let entity = scene
        .entities
        .iter()
        .find(|e| e.persistent_id == entity_id)
        .unwrap();
    assert_eq!(entity.name.as_deref(), Some("Renamed"));

    cmd.undo(&mut scene).unwrap();
    let entity = scene
        .entities
        .iter()
        .find(|e| e.persistent_id == entity_id)
        .unwrap();
    assert_eq!(entity.name.as_deref(), Some("Main Camera"));
}

#[cfg(feature = "tooling-editor")]
#[test]
fn set_component_field_execute_and_undo() {
    use engine_serialize::Value;

    let mut scene = engine_scene::sample_scene();
    let entity_id = "cube-01".to_string();
    let comp_type = "engine.renderable".to_string();

    let mut cmd = SetComponentField::new(
        entity_id.clone(),
        comp_type.clone(),
        "visible".to_string(),
        Value::Bool(false),
    );
    cmd.execute(&mut scene).unwrap();

    let entity = scene
        .entities
        .iter()
        .find(|e| e.persistent_id == entity_id)
        .unwrap();
    let comp = entity.components.get(&comp_type).unwrap();
    assert_eq!(comp.fields.get("visible"), Some(&Value::Bool(false)));

    cmd.undo(&mut scene).unwrap();
    let entity = scene
        .entities
        .iter()
        .find(|e| e.persistent_id == entity_id)
        .unwrap();
    let comp = entity.components.get(&comp_type).unwrap();
    assert_eq!(comp.fields.get("visible"), Some(&Value::Bool(true)));
}

#[cfg(feature = "tooling-editor")]
#[test]
fn add_entity_execute_and_undo() {
    use engine_scene::EntityRecord;
    use std::collections::BTreeMap;

    let mut scene = engine_scene::sample_scene();
    let count_before = scene.entities.len();

    let entity = EntityRecord {
        persistent_id: "new-entity".to_string(),
        parent: None,
        name: Some("New".to_string()),
        enabled: true,
        components: BTreeMap::new(),
    };

    let mut cmd = AddEntity::new(entity);
    cmd.execute(&mut scene).unwrap();
    assert_eq!(scene.entities.len(), count_before + 1);
    assert!(scene
        .entities
        .iter()
        .any(|e| e.persistent_id == "new-entity"));

    cmd.undo(&mut scene).unwrap();
    assert_eq!(scene.entities.len(), count_before);
    assert!(!scene
        .entities
        .iter()
        .any(|e| e.persistent_id == "new-entity"));
}

#[cfg(feature = "tooling-editor")]
#[test]
fn remove_entity_execute_and_undo() {
    let mut scene = engine_scene::sample_scene();
    let count_before = scene.entities.len();

    let mut cmd = RemoveEntity::new("cube-01".to_string());
    cmd.execute(&mut scene).unwrap();
    assert_eq!(scene.entities.len(), count_before - 1);
    assert!(!scene.entities.iter().any(|e| e.persistent_id == "cube-01"));

    cmd.undo(&mut scene).unwrap();
    assert_eq!(scene.entities.len(), count_before);
    assert!(scene.entities.iter().any(|e| e.persistent_id == "cube-01"));
}

#[cfg(feature = "tooling-editor")]
#[test]
fn add_component_execute_and_undo() {
    use engine_scene::ComponentRecord;
    use engine_serialize::{SchemaVersion, Value};
    use std::collections::BTreeMap;

    let mut scene = engine_scene::sample_scene();
    let entity_id = "camera-main".to_string();
    let comp_type = "test.component".to_string();

    let mut fields = BTreeMap::new();
    fields.insert("value".to_string(), Value::Int(42));
    let comp = ComponentRecord {
        schema_version: SchemaVersion::new(0, 1, 0),
        enabled: true,
        fields,
    };

    let mut cmd = AddComponent::new(entity_id.clone(), comp_type.clone(), comp);
    cmd.execute(&mut scene).unwrap();

    let entity = scene
        .entities
        .iter()
        .find(|e| e.persistent_id == entity_id)
        .unwrap();
    assert!(entity.components.contains_key(&comp_type));

    cmd.undo(&mut scene).unwrap();
    let entity = scene
        .entities
        .iter()
        .find(|e| e.persistent_id == entity_id)
        .unwrap();
    assert!(!entity.components.contains_key(&comp_type));
}

#[cfg(feature = "tooling-editor")]
#[test]
fn remove_component_execute_and_undo() {
    let mut scene = engine_scene::sample_scene();
    let entity_id = "cube-01".to_string();
    let comp_type = "engine.renderable".to_string();

    let mut cmd = RemoveComponent::new(entity_id.clone(), comp_type.clone());
    cmd.execute(&mut scene).unwrap();

    let entity = scene
        .entities
        .iter()
        .find(|e| e.persistent_id == entity_id)
        .unwrap();
    assert!(!entity.components.contains_key(&comp_type));

    cmd.undo(&mut scene).unwrap();
    let entity = scene
        .entities
        .iter()
        .find(|e| e.persistent_id == entity_id)
        .unwrap();
    assert!(entity.components.contains_key(&comp_type));
}

// ── CommandHistory integration tests ─────────────────────────────────

#[cfg(feature = "tooling-editor")]
#[test]
fn command_history_push_and_undo() {
    let mut scene = engine_scene::sample_scene();
    let mut history = CommandHistory::new();

    let cmd = Box::new(SetEntityName::new(
        "camera-main".to_string(),
        Some("Cam".to_string()),
    ));
    history.push(cmd, &mut scene).unwrap();
    assert!(history.can_undo());
    assert!(!history.can_redo());
    assert!(history.is_dirty());

    let entity = scene
        .entities
        .iter()
        .find(|e| e.persistent_id == "camera-main")
        .unwrap();
    assert_eq!(entity.name.as_deref(), Some("Cam"));

    history.undo(&mut scene).unwrap();
    let entity = scene
        .entities
        .iter()
        .find(|e| e.persistent_id == "camera-main")
        .unwrap();
    assert_eq!(entity.name.as_deref(), Some("Main Camera"));

    assert!(history.can_redo());
}

#[cfg(feature = "tooling-editor")]
#[test]
fn command_history_redo() {
    let mut scene = engine_scene::sample_scene();
    let mut history = CommandHistory::new();

    let cmd = Box::new(SetEntityName::new(
        "camera-main".to_string(),
        Some("Cam".to_string()),
    ));
    history.push(cmd, &mut scene).unwrap();

    history.undo(&mut scene).unwrap();
    history.redo(&mut scene).unwrap();

    let entity = scene
        .entities
        .iter()
        .find(|e| e.persistent_id == "camera-main")
        .unwrap();
    assert_eq!(entity.name.as_deref(), Some("Cam"));
}

#[cfg(feature = "tooling-editor")]
#[test]
fn command_history_mark_clean() {
    let mut scene = engine_scene::sample_scene();
    let mut history = CommandHistory::new();

    let cmd = Box::new(SetEntityName::new(
        "camera-main".to_string(),
        Some("Cam".to_string()),
    ));
    history.push(cmd, &mut scene).unwrap();
    assert!(history.is_dirty());

    history.mark_clean();
    assert!(!history.is_dirty());
}

#[cfg(feature = "tooling-editor")]
#[test]
fn command_history_clear() {
    let mut scene = engine_scene::sample_scene();
    let mut history = CommandHistory::new();

    let cmd = Box::new(SetEntityName::new(
        "camera-main".to_string(),
        Some("Cam".to_string()),
    ));
    history.push(cmd, &mut scene).unwrap();
    history.clear();

    assert!(!history.can_undo());
    assert!(!history.can_redo());
    assert!(!history.is_dirty());
}

// ── EditorScene tests ────────────────────────────────────────────────

#[cfg(feature = "tooling-editor")]
#[test]
fn editor_scene_new_not_dirty() {
    let scene = engine_scene::sample_scene();
    let editor_scene = EditorScene::new(scene);
    assert!(!editor_scene.is_dirty());
    assert!(editor_scene.selected_entity.is_none());
}

#[cfg(feature = "tooling-editor")]
#[test]
fn editor_scene_execute_and_undo() {
    let scene = engine_scene::sample_scene();
    let mut editor_scene = EditorScene::new(scene);

    let cmd = Box::new(SetEntityName::new(
        "camera-main".to_string(),
        Some("Renamed".to_string()),
    ));

    editor_scene.execute(cmd).unwrap();
    assert!(editor_scene.is_dirty());

    let entity = editor_scene
        .scene
        .entities
        .iter()
        .find(|e| e.persistent_id == "camera-main")
        .unwrap();
    assert_eq!(entity.name.as_deref(), Some("Renamed"));

    editor_scene.undo().unwrap();
    let entity = editor_scene
        .scene
        .entities
        .iter()
        .find(|e| e.persistent_id == "camera-main")
        .unwrap();
    assert_eq!(entity.name.as_deref(), Some("Main Camera"));

    editor_scene.redo().unwrap();
    let entity = editor_scene
        .scene
        .entities
        .iter()
        .find(|e| e.persistent_id == "camera-main")
        .unwrap();
    assert_eq!(entity.name.as_deref(), Some("Renamed"));
}

// ── EditorError extra variant tests ──────────────────────────────────

#[cfg(feature = "tooling-editor")]
#[test]
fn editor_error_entity_not_found_display() {
    let err = EditorError::EntityNotFound("missing-entity".to_string());
    assert_eq!(err.to_string(), "entity not found: missing-entity");
}

#[cfg(feature = "tooling-editor")]
#[test]
fn editor_error_component_not_found_display() {
    let err = EditorError::ComponentNotFound("missing-comp".to_string());
    assert_eq!(err.to_string(), "component not found: missing-comp");
}

#[cfg(feature = "tooling-editor")]
#[test]
fn editor_error_io_failed_display() {
    let err = EditorError::IoFailed("permission denied".to_string());
    assert_eq!(err.to_string(), "I/O error: permission denied");
}
