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

    // Undo - should become visible again.
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
// Scene save -> load round-trip through Editor
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
    assert!(
        !es.is_dirty(),
        "a successful editor save must mark history clean"
    );

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
// Full editor pipeline: load -> edit -> extract -> validate -> renderer
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
    //    (Renderer is constructed as a mock - no GPU needed.)
    assert_eq!(input.drawables.len(), 1, "expected 1 draw item");

    // 6. Undo the add, verify extraction changes
    es.undo().unwrap();
    let input_after_undo = extract(&es.scene).expect("extraction after undo should succeed");
    let cams_after = input_after_undo.views.len();
    assert_eq!(cams_after, 1, "after undo, should have 1 camera (original)");
}

// ============================================================================
// Error handling
// ============================================================================
