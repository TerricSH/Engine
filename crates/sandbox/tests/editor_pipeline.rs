//! Editor pipeline integration tests.
//!
//! These tests exercise the full editor → scene → extraction pipeline,
//! verifying that editor operations on a scene produce correct rendering
//! input after extraction.  All tests require the `tooling-editor` feature.
//!
//! # Test categories
//!
//! | Area | Files | What it covers |
//! |------|-------|----------------|
//! | EditorScene lifecycle | `editor_pipeline.rs` | Command execute/undo/redo, save/load round-trip, extraction after editing |
//! | Cross-crate | `editor_pipeline.rs` | Editor → extraction → renderer validation |
//!
//! The canonical test fixture is `engine_scene::sample_scene()` (2 entities:
//! `camera-main` + `cube-01`).

#![cfg(feature = "tooling-editor")]

use std::collections::BTreeMap;

use engine_editor::commands::{
    AddComponent, AddEntity, RemoveComponent, RemoveEntity, SetComponentField, SetEntityName,
};
use engine_editor::io;
use engine_editor::{EditorError, EditorScene};
use engine_renderer::{validate_frame_input, RenderFrameInput};
use engine_scene::{
    extract_renderer_input, sample_scene, validate_scene, ComponentRecord, EntityRecord, Scene,
};
use engine_serialize::{AssetId, SchemaVersion, Value};

// ============================================================================
// Helpers
// ============================================================================

/// A unique persistent ID generator for test entities.
fn unique_id(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    format!("{prefix}-{n}")
}

/// Create a minimal entity record with just a name.
fn make_entity(name: &str) -> EntityRecord {
    EntityRecord {
        persistent_id: unique_id(name),
        parent: None,
        name: Some(name.to_string()),
        enabled: true,
        components: BTreeMap::new(),
    }
}

/// Create a renderable component record for a cube.
fn make_renderable(visible: bool) -> ComponentRecord {
    let mut fields = BTreeMap::new();
    fields.insert("mesh".to_string(), Value::Asset(AssetId::new("mesh-cube")));
    fields.insert(
        "material".to_string(),
        Value::Asset(AssetId::new("mat-default")),
    );
    fields.insert("visible".to_string(), Value::Bool(visible));
    fields.insert("cast_shadows".to_string(), Value::Bool(true));
    fields.insert(
        "render_layer".to_string(),
        Value::Str("Default".to_string()),
    );
    ComponentRecord {
        schema_version: SchemaVersion::new(0, 1, 0),
        enabled: true,
        fields,
    }
}

/// Create a camera component record.
fn make_camera() -> ComponentRecord {
    let mut fields = BTreeMap::new();
    fields.insert(
        "projection".to_string(),
        Value::Str("perspective".to_string()),
    );
    fields.insert("near".to_string(), Value::Float32(0.1));
    fields.insert("far".to_string(), Value::Float32(100.0));
    fields.insert("fov_y".to_string(), Value::Float32(1.0472)); // 60°
    fields.insert(
        "clear_color".to_string(),
        Value::Color([0.0, 0.0, 0.0, 1.0]),
    );
    ComponentRecord {
        schema_version: SchemaVersion::new(0, 1, 0),
        enabled: true,
        fields,
    }
}

/// Extract renderer input from an EditorScene.
fn extract(scene: &Scene) -> Result<RenderFrameInput, Vec<engine_serialize::Diagnostic>> {
    extract_renderer_input(scene, 0)
}

// ============================================================================
// EditorScene lifecycle
// ============================================================================

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

    let cmd = Box::new(RemoveEntity::new(&"cube-01".to_string(), &es.scene));
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

#[test]
fn extraction_reflects_entity_add() {
    let mut es = EditorScene::new(sample_scene());

    // Add a second renderable cube
    let mut entity = make_entity("extra-cube");
    entity
        .components
        .insert("engine.renderable".to_string(), make_renderable(true));
    es.execute(Box::new(AddEntity::new(entity))).unwrap();

    let input = extract(&es.scene).expect("extraction should succeed");
    let renderable_count = input
        .drawables
        .iter()
        .filter(|d| d.material.id != "shadow-only")
        .count();
    assert_eq!(
        renderable_count, 2,
        "should have 2 renderable drawables (original cube + extra cube)"
    );
}

#[test]
fn extraction_reflects_visibility_change() {
    let mut es = EditorScene::new(sample_scene());

    // Hide the cube
    let cmd = Box::new(SetComponentField::new(
        "cube-01".to_string(),
        "engine.renderable".to_string(),
        "visible".to_string(),
        Value::Bool(false),
    ));
    es.execute(cmd).unwrap();

    let input = extract(&es.scene).expect("extraction should succeed");
    let visible = input
        .drawables
        .iter()
        .filter(|d| d.material.id != "shadow-only")
        .count();
    assert_eq!(visible, 0, "no visible drawables when cube is hidden");
}

#[test]
fn extraction_after_undo_restores_original() {
    let mut es = EditorScene::new(sample_scene());

    // Hide the cube
    let cmd = Box::new(SetComponentField::new(
        "cube-01".to_string(),
        "engine.renderable".to_string(),
        "visible".to_string(),
        Value::Bool(false),
    ));
    es.execute(cmd).unwrap();

    let input = extract(&es.scene).expect("extraction should succeed");
    let visible = input
        .drawables
        .iter()
        .filter(|d| d.material.id != "shadow-only")
        .count();
    assert_eq!(visible, 0);

    // Undo — should become visible again
    es.undo().unwrap();
    let input = extract(&es.scene).expect("extraction after undo should succeed");
    let visible = input
        .drawables
        .iter()
        .filter(|d| d.material.id != "shadow-only")
        .count();
    assert_eq!(visible, 1, "cube should be visible again after undo");
}

// ============================================================================
// Scene save → load round-trip through Editor
// ============================================================================

#[test]
fn editor_save_load_roundtrip() {
    let mut es = EditorScene::new(sample_scene());

    // Modify scene
    let cmd = Box::new(SetEntityName::new(
        "camera-main".to_string(),
        Some("EditedCam".to_string()),
    ));
    es.execute(cmd).unwrap();

    // Also add a new entity
    let mut new_entity = make_entity("roundtrip-entity");
    new_entity
        .components
        .insert("engine.renderable".to_string(), make_renderable(true));
    es.execute(Box::new(AddEntity::new(new_entity))).unwrap();

    // Save
    let dir = std::env::temp_dir().join("sandbox-editor-tests");
    let path = dir.join("test_roundtrip.scene.ron");
    let _ = std::fs::remove_file(&path);
    es.save(Some(&path)).expect("save should succeed");

    // Load into a fresh EditorScene
    let loaded_scene = io::load_scene(&path).expect("load should succeed");
    let loaded_es = EditorScene::new(loaded_scene);

    // Verify scene-level fields
    assert_eq!(loaded_es.scene.name, es.scene.name);
    assert_eq!(loaded_es.scene.entities.len(), es.scene.entities.len());

    // Verify entity data
    assert_eq!(
        find_entity(&loaded_es, "camera-main").name.as_deref(),
        Some("EditedCam"),
        "renamed entity should persist after save/load"
    );
    assert!(
        loaded_es
            .scene
            .entities
            .iter()
            .any(|e| e.components.contains_key("engine.renderable")),
        "should contain at least one renderable entity"
    );

    // Verify loaded scene validates
    let diags = validate_scene(&loaded_es.scene);
    assert!(
        diags.is_empty(),
        "loaded scene should validate cleanly: {:?}",
        diags
    );

    // Verify extraction works on loaded scene
    let input = extract(&loaded_es.scene).expect("extraction on loaded scene should succeed");
    assert!(!input.views.is_empty(), "loaded scene should have views");
    assert!(
        !input.drawables.is_empty(),
        "loaded scene should have drawables"
    );

    // Clean up
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir(&dir);
}

// ============================================================================
// Full editor pipeline: load → edit → extract → validate → renderer
// ============================================================================

#[test]
fn full_editor_pipeline() {
    // 1. Start with sample scene
    let mut es = EditorScene::new(sample_scene());
    assert!(validate_scene(&es.scene).is_empty());

    // 2. Edit: add a second camera + renderable entity
    let mut second_cam = make_entity("cam-2");
    second_cam
        .components
        .insert("engine.camera".to_string(), make_camera());
    es.execute(Box::new(AddEntity::new(second_cam))).unwrap();

    // 3. Extract renderer input
    let input = extract(&es.scene).expect("extraction after edit should succeed");

    // 4. Validate renderer input
    let render_diags = validate_frame_input(&input);
    assert!(
        render_diags.is_empty(),
        "renderer input should validate: {:?}",
        render_diags
    );

    // 5. Verify data flows through to renderer
    //    (Renderer is constructed as a mock — no GPU needed)
    let mut renderer = engine_renderer::Renderer::new();
    let stats = renderer
        .draw_scene(&input)
        .expect("draw_scene should succeed with valid input");
    assert_eq!(stats.draw_calls, 1, "expected 1 draw call");

    // 6. Undo the add, verify extraction changes
    es.undo().unwrap();
    let input_after_undo = extract(&es.scene).expect("extraction after undo should succeed");
    let cams_after = input_after_undo.views.len();
    assert_eq!(cams_after, 1, "after undo, should have 1 camera (original)");
}

// ============================================================================
// Error handling
// ============================================================================

#[test]
fn editor_scene_execute_unknown_entity_entity_not_found() {
    let mut es = EditorScene::new(sample_scene());
    let cmd = Box::new(SetEntityName::new(
        "non-existent".to_string(),
        Some("Nope".to_string()),
    ));
    let result = es.execute(cmd);
    assert!(
        matches!(result, Err(EditorError::EntityNotFound(_))),
        "expected EntityNotFound, got: {:?}",
        result
    );
}

#[test]
fn editor_scene_undo_when_empty_is_noop() {
    let mut es = EditorScene::new(sample_scene());
    let count_before = es.scene.entities.len();
    es.undo().expect("undo on empty history should be a no-op");
    assert_eq!(
        es.scene.entities.len(),
        count_before,
        "undo on empty history should not change scene"
    );
}

#[test]
fn editor_scene_redo_when_empty_is_noop() {
    let mut es = EditorScene::new(sample_scene());
    let count_before = es.scene.entities.len();
    es.redo().expect("redo on empty history should be a no-op");
    assert_eq!(
        es.scene.entities.len(),
        count_before,
        "redo on empty history should not change scene"
    );
}

// ============================================================================
// Scene validation integration
// ============================================================================

#[test]
fn editor_mutations_never_produce_invalid_scene() {
    // Run a randomized sequence of commands and verify the scene always
    // passes validation.
    let mut es = EditorScene::new(sample_scene());
    let original_id = "camera-main".to_string();

    // Sequence: rename → add entity → rename → undo → undo
    let cmds: Vec<Box<dyn engine_editor::commands::Command>> = vec![
        Box::new(SetEntityName::new(
            original_id.clone(),
            Some("Cam".to_string()),
        )),
        Box::new(AddEntity::new(make_entity("temp"))),
        Box::new(SetComponentField::new(
            "cube-01".to_string(),
            "engine.renderable".to_string(),
            "visible".to_string(),
            Value::Bool(false),
        )),
    ];

    for cmd in cmds {
        es.execute(cmd).unwrap();
        let diags = validate_scene(&es.scene);
        assert!(
            diags.is_empty(),
            "scene should stay valid after command: {:?}",
            diags
        );
    }

    es.undo().unwrap();
    let diags = validate_scene(&es.scene);
    assert!(diags.is_empty(), "scene valid after 1 undo: {:?}", diags);

    es.undo().unwrap();
    let diags = validate_scene(&es.scene);
    assert!(diags.is_empty(), "scene valid after 2 undo: {:?}", diags);
}
