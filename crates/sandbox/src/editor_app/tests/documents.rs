#[test]
fn dirty_scene_switch_requires_an_explicit_resolution() {
    let fixture = scene_project_fixture();
    let mut app = editor_app_with_loaded_fixture(fixture.project.clone());
    app.editor_scene
        .as_mut()
        .unwrap()
        .execute(Box::new(engine_editor::SetEntityName::new(
            "cube-01".into(),
            Some("Unsaved Cube".into()),
        )))
        .unwrap();

    assert!(!app.request_scene_switch("level_two".into()).unwrap());
    assert_eq!(app.current_scene_id, "main");
    assert_eq!(
        app.pending_scene_switch.as_deref(),
        Some("opening scene 'level_two'")
    );
    assert_eq!(
        app.pending_document_action,
        Some(SceneDocumentAction::Open("level_two".into()))
    );

    app.apply_scene_document_action(SceneDocumentAction::CancelSwitch)
        .unwrap();
    assert_eq!(app.current_scene_id, "main");
    assert!(app.pending_scene_switch.is_none());
    assert!(app.editor_scene.as_ref().unwrap().is_dirty());
}

#[test]
fn dirty_document_mutations_do_not_touch_files_before_confirmation() {
    let fixture = scene_project_fixture();
    let mut app = editor_app_with_loaded_fixture(fixture.project);
    make_scene_dirty(&mut app, "Unsaved Cube");

    for (method, params, expected) in [
        (
            "document.create",
            serde_json::json!({ "sceneId": "created", "folder": "levels" }),
            SceneDocumentAction::Create {
                scene_id: "created".into(),
                folder: PathBuf::from("levels"),
            },
        ),
        (
            "document.duplicate",
            serde_json::json!({ "sourceId": "level_two", "newId": "duplicated" }),
            SceneDocumentAction::Duplicate {
                source_id: "level_two".into(),
                new_id: "duplicated".into(),
            },
        ),
        (
            "document.rename",
            serde_json::json!({ "oldId": "main", "newId": "renamed" }),
            SceneDocumentAction::Rename {
                old_id: "main".into(),
                new_id: "renamed".into(),
            },
        ),
        (
            "document.delete",
            serde_json::json!({
                "sceneId": "main",
                "replacementStartup": "level_two"
            }),
            SceneDocumentAction::Delete {
                scene_id: "main".into(),
                replacement_startup: Some("level_two".into()),
            },
        ),
    ] {
        let response = dispatch_test_request(&mut app, method, params);
        assert!(response.get("error").is_none(), "{response}");
        assert_eq!(app.current_scene_id, "main");
        assert_eq!(app.pending_document_action, Some(expected));
        assert_eq!(persisted_cube_name(&app).as_deref(), Some("Cube"));
        assert!(app.project.scene_path("main").is_some());
        assert!(app.project.scene_path("created").is_none());
        assert!(app.project.scene_path("duplicated").is_none());
        assert!(app.project.scene_path("renamed").is_none());

        let response = dispatch_test_request(
            &mut app,
            "document.resolvePendingSwitch",
            serde_json::json!({ "decision": "cancel" }),
        );
        assert!(response.get("error").is_none(), "{response}");
        assert!(app.pending_scene_switch.is_none());
        assert!(app.pending_document_action.is_none());
        assert!(app.editor_scene.as_ref().unwrap().is_dirty());
    }
}

#[test]
fn failed_discarded_switch_keeps_dirty_state_and_exact_pending_target() {
    let fixture = scene_project_fixture();
    let mut app = editor_app_with_loaded_fixture(fixture.project);
    make_scene_dirty(&mut app, "Unsaved Cube");

    let response = dispatch_test_request(
        &mut app,
        "document.open",
        serde_json::json!({ "sceneId": "invalid" }),
    );
    assert!(response.get("error").is_none(), "{response}");

    let response = dispatch_test_request(
        &mut app,
        "document.resolvePendingSwitch",
        serde_json::json!({ "decision": "discard" }),
    );
    assert_eq!(
        response["error"]["code"],
        serde_json::Value::String("validationFailed".into())
    );
    assert_eq!(app.current_scene_id, "main");
    assert!(app.editor_scene.as_ref().unwrap().is_dirty());
    assert_eq!(persisted_cube_name(&app).as_deref(), Some("Cube"));
    assert_eq!(
        app.pending_document_action,
        Some(SceneDocumentAction::Open("invalid".into()))
    );
    assert_eq!(
        app.pending_scene_switch.as_deref(),
        Some("opening scene 'invalid'")
    );
}

#[test]
fn a_second_document_request_cannot_overwrite_the_pending_target() {
    let fixture = scene_project_fixture();
    let mut app = editor_app_with_loaded_fixture(fixture.project);
    make_scene_dirty(&mut app, "Unsaved Cube");
    dispatch_test_request(
        &mut app,
        "document.open",
        serde_json::json!({ "sceneId": "level_two" }),
    );

    let response = dispatch_test_request(
        &mut app,
        "document.create",
        serde_json::json!({ "sceneId": "wrong-target", "folder": "" }),
    );

    assert_eq!(response["error"]["code"], "conflict");
    assert_eq!(
        app.pending_document_action,
        Some(SceneDocumentAction::Open("level_two".into()))
    );
    assert!(app.project.scene_path("wrong-target").is_none());
}

#[test]
fn discard_resolves_each_deferred_document_action_without_losing_its_target() {
    for (method, params, expected_scene) in [
        (
            "document.open",
            serde_json::json!({ "sceneId": "level_two" }),
            "level_two",
        ),
        (
            "document.create",
            serde_json::json!({ "sceneId": "created", "folder": "levels" }),
            "created",
        ),
        (
            "document.duplicate",
            serde_json::json!({ "sourceId": "level_two", "newId": "duplicated" }),
            "duplicated",
        ),
        (
            "document.rename",
            serde_json::json!({ "oldId": "main", "newId": "renamed" }),
            "renamed",
        ),
        (
            "document.delete",
            serde_json::json!({
                "sceneId": "main",
                "replacementStartup": "level_two"
            }),
            "level_two",
        ),
    ] {
        let fixture = scene_project_fixture();
        let mut app = editor_app_with_loaded_fixture(fixture.project);
        make_scene_dirty(&mut app, "Must Be Discarded");

        let response = dispatch_test_request(&mut app, method, params);
        assert!(response.get("error").is_none(), "{method}: {response}");
        assert_eq!(app.current_scene_id, "main");
        assert!(app.pending_document_action.is_some());

        let response = dispatch_test_request(
            &mut app,
            "document.resolvePendingSwitch",
            serde_json::json!({ "decision": "discard" }),
        );
        assert!(response.get("error").is_none(), "{method}: {response}");
        assert_eq!(app.current_scene_id, expected_scene, "{method}");
        assert!(app.pending_scene_switch.is_none(), "{method}");
        assert!(app.pending_document_action.is_none(), "{method}");
        assert!(!app.editor_scene.as_ref().unwrap().is_dirty(), "{method}");
        assert!(app
            .editor_scene
            .as_ref()
            .unwrap()
            .scene
            .entities
            .iter()
            .all(|entity| entity.name.as_deref() != Some("Must Be Discarded")));
    }
}

#[test]
fn resolving_dirty_switch_with_save_persists_then_opens_exact_target() {
    let fixture = scene_project_fixture();
    let main_path = fixture.project.scene_path("main").unwrap();
    let mut app = editor_app_with_loaded_fixture(fixture.project);
    make_scene_dirty(&mut app, "Saved Cube");

    dispatch_test_request(
        &mut app,
        "document.open",
        serde_json::json!({ "sceneId": "level_two" }),
    );
    let response = dispatch_test_request(
        &mut app,
        "document.resolvePendingSwitch",
        serde_json::json!({ "decision": "save" }),
    );

    assert!(response.get("error").is_none(), "{response}");
    assert_eq!(app.current_scene_id, "level_two");
    assert!(app.pending_document_action.is_none());
    assert!(!app.editor_scene.as_ref().unwrap().is_dirty());
    let saved = Scene::load_from_file(&main_path).unwrap();
    assert_eq!(
        saved
            .entities
            .iter()
            .find(|entity| entity.persistent_id == "cube-01")
            .and_then(|entity| entity.name.as_deref()),
        Some("Saved Cube")
    );
}

#[test]
fn play_mode_cannot_bypass_or_resolve_a_pending_document_prompt() {
    let fixture = scene_project_fixture();
    let mut app = editor_app_with_loaded_fixture(fixture.project);
    make_scene_dirty(&mut app, "Unsaved Cube");
    dispatch_test_request(
        &mut app,
        "document.open",
        serde_json::json!({ "sceneId": "level_two" }),
    );

    let response = dispatch_test_request(
        &mut app,
        "runtime.setMode",
        serde_json::json!({ "mode": "play" }),
    );
    assert_eq!(response["error"]["code"], "conflict");
    assert!(app.play_session.is_editing());
    assert!(app.pending_document_action.is_some());

    // Simulate an already queued native Play transition racing the web
    // prompt. Resolution must still refuse authoring writes in Play mode.
    app.start_play();
    assert!(!app.play_session.is_editing());
    let response = dispatch_test_request(
        &mut app,
        "document.resolvePendingSwitch",
        serde_json::json!({ "decision": "save" }),
    );
    assert_eq!(response["error"]["code"], "editingRequired");
    assert_eq!(persisted_cube_name(&app).as_deref(), Some("Cube"));
    assert!(app.pending_document_action.is_some());
}

#[test]
fn save_and_close_persists_the_open_document_before_exit() {
    let fixture = scene_project_fixture();
    let mut app = editor_app_with_loaded_fixture(fixture.project);
    app.editor_scene
        .as_mut()
        .unwrap()
        .execute(Box::new(engine_editor::SetEntityName::new(
            "cube-01".into(),
            Some("Saved Before Close".into()),
        )))
        .unwrap();
    app.close_confirmation_pending = true;

    app.apply_close_document_action(CloseDocumentAction::SaveAndClose)
        .unwrap();

    assert!(app.exit_after_frame);
    assert!(!app.close_confirmation_pending);
    assert!(!app.editor_scene.as_ref().unwrap().is_dirty());
    let saved = Scene::load_from_file(&app.current_scene_path).unwrap();
    assert_eq!(
        saved
            .entities
            .iter()
            .find(|entity| entity.persistent_id == "cube-01")
            .and_then(|entity| entity.name.as_deref()),
        Some("Saved Before Close")
    );
}

#[test]
fn cancel_or_discard_close_never_implicitly_saves() {
    let fixture = scene_project_fixture();
    let mut app = editor_app_with_loaded_fixture(fixture.project);
    app.editor_scene
        .as_mut()
        .unwrap()
        .execute(Box::new(engine_editor::SetEntityName::new(
            "cube-01".into(),
            Some("Unsaved Before Close".into()),
        )))
        .unwrap();
    app.close_confirmation_pending = true;

    app.apply_close_document_action(CloseDocumentAction::Cancel)
        .unwrap();
    assert!(!app.exit_after_frame);
    assert!(!app.close_confirmation_pending);
    assert!(app.editor_scene.as_ref().unwrap().is_dirty());

    app.close_confirmation_pending = true;
    app.apply_close_document_action(CloseDocumentAction::DiscardAndClose)
        .unwrap();
    assert!(app.exit_after_frame);
    let saved = Scene::load_from_file(&app.current_scene_path).unwrap();
    assert_eq!(
        saved
            .entities
            .iter()
            .find(|entity| entity.persistent_id == "cube-01")
            .and_then(|entity| entity.name.as_deref()),
        Some("Cube")
    );
}

#[test]
fn scene_document_switch_replaces_document_and_resets_editor_state() {
    let fixture = scene_project_fixture();
    let mut app = editor_app_with_loaded_fixture(fixture.project.clone());
    app.editor_scene.as_mut().unwrap().selected_entity = Some("cube-01".into());
    app.material_editor_selection = Some("mat-default".into());
    app.gizmo.dragging = true;

    assert!(app.switch_scene_document("level_two").unwrap());

    assert_eq!(app.current_scene_id, "level_two");
    assert_eq!(
        app.current_scene_path,
        fixture.project.scene_path("level_two").unwrap()
    );
    let editor_scene = app.editor_scene.as_ref().unwrap();
    assert_eq!(editor_scene.scene.scene_id, "level_two");
    assert!(!editor_scene.is_dirty());
    assert!(editor_scene.selected_entity.is_none());
    assert!(!app.gizmo.dragging);
    assert!(app.material_editor_selection.is_none());
    let preview = app.game_loop.as_ref().unwrap().runtime.scene_ref().unwrap();
    assert!(preview
        .entities
        .iter()
        .any(|entity| entity.persistent_id.starts_with(EDITOR_CAMERA_ID_PREFIX)));
    assert!(preview
        .entities
        .iter()
        .all(|entity| !entity.components.contains_key("engine.script")));
}

#[test]
fn failed_scene_document_switch_preserves_document_and_runtime_preview() {
    let fixture = scene_project_fixture();
    let mut app = editor_app_with_loaded_fixture(fixture.project);
    let before_document = app.editor_scene.as_ref().unwrap().scene.clone();
    let before_preview = app
        .game_loop
        .as_ref()
        .unwrap()
        .runtime
        .scene_ref()
        .unwrap()
        .clone();

    let error = app.switch_scene_document("invalid").unwrap_err();

    assert!(error.contains("game.unknown"), "{error}");
    assert_eq!(app.current_scene_id, "main");
    assert_eq!(app.editor_scene.as_ref().unwrap().scene, before_document);
    assert_eq!(
        app.game_loop.as_ref().unwrap().runtime.scene_ref().unwrap(),
        &before_preview
    );
}

#[test]
fn editor_play_tracks_the_open_catalog_scene_and_stop_restores_its_preview() {
    let fixture = scene_project_fixture();
    let mut app = editor_app_with_loaded_fixture(fixture.project);
    app.switch_scene_document("level_two").unwrap();

    app.start_play();
    assert_eq!(app.play_session.mode(), EditorPlayMode::Playing);
    assert_eq!(app.play_runtime_scene_id.as_deref(), Some("level_two"));
    assert_eq!(
        app.game_loop
            .as_ref()
            .unwrap()
            .runtime
            .scene_ref()
            .map(|scene| scene.scene_id.as_str()),
        Some("level_two")
    );

    app.stop_play();
    assert!(app.play_session.is_editing());
    assert!(app.play_runtime_scene_id.is_none());
    let preview = app.game_loop.as_ref().unwrap().runtime.scene_ref().unwrap();
    assert_eq!(preview.scene_id, "level_two");
    assert!(preview
        .entities
        .iter()
        .any(|entity| entity.persistent_id.starts_with(EDITOR_CAMERA_ID_PREFIX)));
}
