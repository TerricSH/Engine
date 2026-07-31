#[test]
fn asset_delete_rejects_a_reference_present_only_in_the_unsaved_scene() {
    let fixture = scene_project_fixture();
    let mut app = editor_app_with_loaded_fixture(fixture.project);
    let editor_scene = app.editor_scene.as_mut().unwrap();
    let renderable = editor_scene
        .scene
        .entities
        .iter_mut()
        .find_map(|entity| entity.components.get_mut("engine.renderable"))
        .expect("sample scene contains a renderable");
    renderable.fields.insert(
        "material".into(),
        Value::Asset(AssetId::new("dirty-only-material")),
    );
    editor_scene.history.mark_dirty();

    let response = dispatch_test_request(
        &mut app,
        "assets.delete",
        serde_json::json!({ "assetId": "dirty-only-material" }),
    );

    assert_eq!(response["error"]["code"], "conflict");
    assert!(response["error"]["message"]
        .as_str()
        .unwrap()
        .contains("open authoring scene"));
    assert!(app.background_job.is_none());
}

#[test]
fn active_asset_job_freezes_authoring_mutations_until_dependency_checks_finish() {
    let fixture = scene_project_fixture();
    let mut app = editor_app_with_loaded_fixture(fixture.project);
    let entity_count = app.editor_scene.as_ref().unwrap().scene.entities.len();
    let (_sender, receiver) = mpsc::channel::<Result<EditorJobOutput, String>>();
    app.background_job = Some(EditorBackgroundJob {
        id: 42,
        label: "Delete asset".into(),
        receiver,
        reload_assets: true,
    });

    let response = dispatch_test_request(
        &mut app,
        "scene.createEntity",
        serde_json::json!({ "templateId": "empty" }),
    );

    assert_eq!(response["error"]["code"], "conflict");
    assert_eq!(
        app.editor_scene.as_ref().unwrap().scene.entities.len(),
        entity_count
    );
}

#[test]
fn script_component_can_only_be_created_from_the_loaded_verified_class_list() {
    let fixture = scene_project_fixture();
    let mut app = editor_app_with_loaded_fixture(fixture.project);
    app.project.script_assembly = Some(PathBuf::from("build/scripts/GameScripts.dll"));
    let game_loop = app.game_loop.as_mut().unwrap();
    game_loop.runtime.register_script_host(Box::new(
        engine_script::MockHost::new()
            .with_verified_classes("GameScripts", ["GameScripts.PlayerController"]),
    ));
    game_loop
        .runtime
        .load_script_assembly("GameScripts", "mock", b"managed")
        .unwrap();
    app.editor_scene.as_mut().unwrap().selected_entity = Some("cube-01".into());

    assert!(app
        .verified_script_add_command("GameScripts", "GameScripts.Guessed")
        .err()
        .unwrap()
        .contains("not in the reflection-verified class list"));
    let command = app
        .verified_script_add_command("GameScripts", "GameScripts.PlayerController")
        .expect("verified class must produce an undoable command");
    assert!(app.execute_editor_command(command));

    let component = &app
        .editor_scene
        .as_ref()
        .unwrap()
        .scene
        .entities
        .iter()
        .find(|entity| entity.persistent_id == "cube-01")
        .unwrap()
        .components["engine.script"];
    assert_eq!(
        component.fields.get("assembly_id"),
        Some(&Value::Str("GameScripts".into()))
    );
    assert_eq!(
        component.fields.get("class_name"),
        Some(&Value::Str("GameScripts.PlayerController".into()))
    );
    assert!(app
        .verified_script_add_command("GameScripts", "GameScripts.PlayerController")
        .err()
        .unwrap()
        .contains("already has an engine.script component"));
}

#[test]
fn dirty_scene_recovery_snapshot_is_detected_and_restored_as_unsaved() {
    let fixture = scene_project_fixture();
    let mut app = editor_app_with_loaded_fixture(fixture.project.clone());
    app.editor_scene
        .as_mut()
        .unwrap()
        .execute(Box::new(engine_editor::SetEntityName::new(
            "cube-01".into(),
            Some("Recovered Cube".into()),
        )))
        .unwrap();
    app.last_recovery_snapshot = Instant::now() - std::time::Duration::from_secs(31);
    app.maybe_write_recovery_snapshot();
    let recovery = scene_recovery_path(&fixture.project, "main");
    assert!(recovery.is_file());

    let mut reopened = editor_app_with_loaded_fixture(fixture.project.clone());
    assert_eq!(
        reopened.pending_recovery.as_deref(),
        Some(recovery.as_path())
    );
    reopened.restore_recovery_snapshot().unwrap();
    let scene = reopened.editor_scene.as_ref().unwrap();
    assert!(scene.is_dirty());
    assert!(!scene.history.can_undo());
    assert_eq!(
        scene
            .scene
            .entities
            .iter()
            .find(|entity| entity.persistent_id == "cube-01")
            .and_then(|entity| entity.name.as_deref()),
        Some("Recovered Cube")
    );
}
