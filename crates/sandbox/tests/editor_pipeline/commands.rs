#[test]
fn editor_scene_new_is_clean() {
    let es = EditorScene::new(sample_scene());
    assert!(!es.is_dirty(), "new EditorScene should not be dirty");
    assert!(es.selected_entity.is_none(), "no entity selected");
    assert!(!es.history.can_undo());
    assert!(!es.history.can_redo());
}

#[test]
fn editor_scene_command_marks_dirty() {
    let mut es = EditorScene::new(sample_scene());
    let cmd = Box::new(SetEntityName::new(
        "camera-main".to_string(),
        Some("Cam".to_string()),
    ));
    es.execute(cmd).unwrap();
    assert!(es.is_dirty(), "after command, EditorScene should be dirty");
}

// ============================================================================
// Command: SetEntityName
// ============================================================================

#[test]
fn set_entity_name_execute_undo_redo() {
    let mut es = EditorScene::new(sample_scene());

    let cmd = Box::new(SetEntityName::new(
        "camera-main".to_string(),
        Some("Renamed".to_string()),
    ));
    es.execute(cmd).unwrap();

    let entity = find_entity(&es, "camera-main");
    assert_eq!(entity.name.as_deref(), Some("Renamed"));

    es.undo().unwrap();
    let entity = find_entity(&es, "camera-main");
    assert_eq!(entity.name.as_deref(), Some("Main Camera"));

    es.redo().unwrap();
    let entity = find_entity(&es, "camera-main");
    assert_eq!(entity.name.as_deref(), Some("Renamed"));
}

fn find_entity<'a>(es: &'a EditorScene, id: &str) -> &'a EntityRecord {
    es.scene
        .entities
        .iter()
        .find(|e| e.persistent_id == id)
        .expect("entity not found")
}

// ============================================================================
// Command: SetComponentField
// ============================================================================

#[test]
fn set_component_field_execute_undo_redo() {
    let mut es = EditorScene::new(sample_scene());

    let cmd = Box::new(SetComponentField::new(
        "cube-01".to_string(),
        "engine.renderable".to_string(),
        "visible".to_string(),
        Value::Bool(false),
    ));
    es.execute(cmd).unwrap();

    let comp = find_component(&es, "cube-01", "engine.renderable");
    assert_eq!(comp.fields.get("visible"), Some(&Value::Bool(false)));

    es.undo().unwrap();
    let comp = find_component(&es, "cube-01", "engine.renderable");
    assert_eq!(comp.fields.get("visible"), Some(&Value::Bool(true)));

    es.redo().unwrap();
    let comp = find_component(&es, "cube-01", "engine.renderable");
    assert_eq!(comp.fields.get("visible"), Some(&Value::Bool(false)));
}

fn find_component<'a>(
    es: &'a EditorScene,
    entity_id: &str,
    comp_type: &str,
) -> &'a ComponentRecord {
    let entity = find_entity(es, entity_id);
    entity
        .components
        .get(comp_type)
        .expect("component not found")
}

// ============================================================================
// Command: AddEntity, RemoveEntity
// ============================================================================

#[test]
fn add_entity_execute_undo_redo() {
    let mut es = EditorScene::new(sample_scene());
    let count_before = es.scene.entities.len();

    let entity = make_entity("new-cube");
    let entity_id = entity.persistent_id.clone();

    let cmd = Box::new(AddEntity::new(entity));
    es.execute(cmd).unwrap();

    assert_eq!(
        es.scene.entities.len(),
        count_before + 1,
        "entity count should increase by 1"
    );
    assert!(
        es.scene
            .entities
            .iter()
            .any(|e| e.persistent_id == entity_id),
        "new entity should exist"
    );
    assert!(es.is_dirty());

    es.undo().unwrap();
    assert_eq!(
        es.scene.entities.len(),
        count_before,
        "after undo, entity count should be back to original"
    );
    assert!(
        !es.scene
            .entities
            .iter()
            .any(|e| e.persistent_id == entity_id),
        "new entity should be removed after undo"
    );

    es.redo().unwrap();
    assert_eq!(es.scene.entities.len(), count_before + 1);
    assert!(
        es.scene
            .entities
            .iter()
            .any(|e| e.persistent_id == entity_id),
        "new entity should reappear after redo"
    );
}

#[test]
fn remove_entity_execute_undo_redo() {
    let mut es = EditorScene::new(sample_scene());
    let count_before = es.scene.entities.len();

    let cmd = Box::new(RemoveEntity::new("cube-01".to_string()));
    es.execute(cmd).unwrap();

    assert_eq!(
        es.scene.entities.len(),
        count_before - 1,
        "entity count should decrease by 1"
    );
    assert!(
        !es.scene
            .entities
            .iter()
            .any(|e| e.persistent_id == "cube-01"),
        "removed entity should not exist"
    );

    es.undo().unwrap();
    assert_eq!(
        es.scene.entities.len(),
        count_before,
        "after undo, entity count should be restored"
    );
    assert!(
        es.scene
            .entities
            .iter()
            .any(|e| e.persistent_id == "cube-01"),
        "removed entity should be restored after undo"
    );

    es.redo().unwrap();
    assert_eq!(es.scene.entities.len(), count_before - 1);
}

// ============================================================================
// Command: AddComponent, RemoveComponent
// ============================================================================

#[test]
fn add_component_execute_undo_redo() {
    let mut es = EditorScene::new(sample_scene());

    let cmd = Box::new(AddComponent::new(
        "camera-main".to_string(),
        "test.custom".to_string(),
        ComponentRecord {
            schema_version: SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: {
                let mut f = BTreeMap::new();
                f.insert("value".to_string(), Value::Int(42));
                f
            },
        },
    ));
    es.execute(cmd).unwrap();

    assert!(find_component(&es, "camera-main", "test.custom")
        .fields
        .contains_key("value"));

    es.undo().unwrap();
    assert!(!find_entity(&es, "camera-main")
        .components
        .contains_key("test.custom"));

    es.redo().unwrap();
    assert!(find_component(&es, "camera-main", "test.custom")
        .fields
        .contains_key("value"));
}

#[test]
fn remove_component_execute_undo_redo() {
    let mut es = EditorScene::new(sample_scene());

    assert!(find_entity(&es, "cube-01")
        .components
        .contains_key("engine.renderable"));

    let cmd = Box::new(RemoveComponent::new(
        "cube-01".to_string(),
        "engine.renderable".to_string(),
    ));
    es.execute(cmd).unwrap();

    assert!(!find_entity(&es, "cube-01")
        .components
        .contains_key("engine.renderable"));

    es.undo().unwrap();
    assert!(find_entity(&es, "cube-01")
        .components
        .contains_key("engine.renderable"));

    es.redo().unwrap();
    assert!(!find_entity(&es, "cube-01")
        .components
        .contains_key("engine.renderable"));
}

// ============================================================================
// CommandHistory: multi-command undo chain
// ============================================================================

#[test]
fn command_history_multi_step_undo_chain() {
    let mut es = EditorScene::new(sample_scene());
    let count_before = es.scene.entities.len();

    // Execute 3 commands
    let cmd1 = Box::new(SetEntityName::new(
        "camera-main".to_string(),
        Some("Step1".to_string()),
    ));
    es.execute(cmd1).unwrap();

    let e1 = make_entity("step2");
    let e1_id = e1.persistent_id.clone();
    let cmd2 = Box::new(AddEntity::new(e1));
    es.execute(cmd2).unwrap();

    let cmd3 = Box::new(SetEntityName::new(
        "cube-01".to_string(),
        Some("Step3".to_string()),
    ));
    es.execute(cmd3).unwrap();

    // Verify all 3 applied
    assert_eq!(
        find_entity(&es, "camera-main").name.as_deref(),
        Some("Step1")
    );
    assert!(es.scene.entities.iter().any(|e| e.persistent_id == e1_id));
    assert_eq!(find_entity(&es, "cube-01").name.as_deref(), Some("Step3"));

    // Undo 2
    es.undo().unwrap(); // undo cmd3
    es.undo().unwrap(); // undo cmd2
    assert_eq!(
        find_entity(&es, "camera-main").name.as_deref(),
        Some("Step1"),
        "cmd1 should still be applied"
    );
    assert_eq!(
        es.scene.entities.len(),
        count_before,
        "step2 entity should be gone"
    );
    assert!(!es.scene.entities.iter().any(|e| e.persistent_id == e1_id));

    // Redo 1
    es.redo().unwrap(); // redo cmd2
    assert_eq!(
        es.scene.entities.len(),
        count_before + 1,
        "step2 entity should be back"
    );
    assert!(es.scene.entities.iter().any(|e| e.persistent_id == e1_id));
}

// ============================================================================
// Extraction after editor mutations
// ============================================================================
