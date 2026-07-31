#[test]
fn editor_frame_time_uses_cpu_interval_until_gpu_timing_is_available() {
    assert_eq!(editor_frame_time_ms(4.25, 1.0 / 60.0), 4.25);
    assert!((editor_frame_time_ms(0.0, 1.0 / 60.0) - 16.666_668).abs() < 0.001);
    assert_eq!(editor_frame_time_ms(f32::NAN, f32::NAN), 0.0);
}

#[test]
fn project_browser_view_and_folder_round_trip_in_workspace_preferences() {
    let mut preferences = EditorWorkspacePreferences {
        project_asset_view: ProjectAssetView::List,
        project_asset_folder: "/materials/environment".to_string(),
        gizmos_visible: false,
        snapping_enabled: true,
        ..EditorWorkspacePreferences::default()
    };
    let json = serde_json::to_vec(&preferences).unwrap();
    let restored: EditorWorkspacePreferences = serde_json::from_slice(&json).unwrap();
    assert_eq!(restored.project_asset_view, ProjectAssetView::List);
    assert_eq!(restored.project_asset_folder, "/materials/environment");
    assert!(!restored.gizmos_visible);
    assert!(restored.snapping_enabled);

    preferences = serde_json::from_str("{}").unwrap();
    assert_eq!(preferences.project_asset_view, ProjectAssetView::Grid);
    assert_eq!(preferences.project_asset_folder, "/");
    assert!(preferences.gizmos_visible);
    assert!(!preferences.snapping_enabled);
}

#[test]
fn gizmo_rendering_requires_visible_editing_scene_viewport() {
    assert!(gizmo_viewport_enabled(true, true, ViewportTab::Scene));
    assert!(!gizmo_viewport_enabled(false, true, ViewportTab::Scene));
    assert!(!gizmo_viewport_enabled(true, false, ViewportTab::Scene));
    assert!(!gizmo_viewport_enabled(true, true, ViewportTab::Game));
}

#[test]
fn script_update_error_stops_play_and_restores_authoring_preview() {
    let authoring = engine_scene::sample_scene();
    let mut play_session = EditorPlaySession::default();
    let mut game_loop = GameLoop::new(EngineConfig::default());
    play_session
        .start(&authoring, |scene| game_loop.load_scene(scene))
        .unwrap();
    let mut runtime_mutation = authoring.clone();
    runtime_mutation.name = "Runtime mutation".into();
    game_loop.load_scene(runtime_mutation).unwrap();

    let diagnostics =
        recover_play_after_script_error(&mut play_session, &mut game_loop, "managed exception");

    assert!(play_session.is_editing());
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "EDPLAY_SCRIPT_UPDATE_FAILED"));
    let restored = game_loop.runtime.scene_ref().unwrap();
    assert_eq!(restored.name, authoring.name);
    assert!(restored
        .scene_settings
        .active_camera
        .as_deref()
        .is_some_and(|camera| camera.starts_with(EDITOR_CAMERA_ID_PREFIX)));
    assert!(restored
        .entities
        .iter()
        .all(|entity| !entity.components.contains_key("engine.script")));
}

struct SceneProjectFixture {
    _temp: tempfile::TempDir,
    project: GameProject,
}

fn scene_project_fixture() -> SceneProjectFixture {
    let temp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(temp.path()).unwrap();
    std::fs::create_dir_all(root.join("assets/source")).unwrap();
    std::fs::create_dir_all(root.join("assets/cooked")).unwrap();
    std::fs::create_dir_all(root.join("assets/scenes")).unwrap();

    let mut main = engine_scene::sample_scene();
    main.scene_id = "main".into();
    main.name = "Main Authoring Scene".into();
    save_scene_atomically(&main, &root.join("assets/scenes/main.scene.ron")).unwrap();

    let mut level_two = engine_scene::sample_scene();
    level_two.scene_id = "level_two".into();
    level_two.name = "Level Two".into();
    save_scene_atomically(&level_two, &root.join("assets/scenes/level_two.scene.ron")).unwrap();

    let mut invalid = engine_scene::sample_scene();
    invalid.scene_id = "invalid".into();
    invalid.name = "Invalid Runtime Scene".into();
    invalid.entities[0].components.insert(
        "game.unknown".into(),
        ComponentRecord {
            schema_version: SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: Default::default(),
        },
    );
    save_scene_atomically(&invalid, &root.join("assets/scenes/invalid.scene.ron")).unwrap();

    let mut manifest = ProjectManifest::new("Editor Scene Documents");
    manifest.startup_scene = PathBuf::from("main");
    manifest.scenes.insert(
        "level_two".into(),
        PathBuf::from("assets/scenes/level_two.scene.ron"),
    );
    manifest.scenes.insert(
        "invalid".into(),
        PathBuf::from("assets/scenes/invalid.scene.ron"),
    );
    manifest.input_actions = None;
    let manifest_path = manifest.write_to_root(&root).unwrap();
    let project = GameProject::load(manifest_path).unwrap();
    SceneProjectFixture {
        _temp: temp,
        project,
    }
}

fn editor_app_with_loaded_fixture(project: GameProject) -> EditorApp {
    let mut app = EditorApp::new(project);
    let scene = Scene::load_from_file(&app.current_scene_path).unwrap();
    let mut game_loop = GameLoop::new(EngineConfig::default());
    let (preview, diagnostics) = editor_preview_scene(&game_loop.runtime, &scene);
    assert!(diagnostics.is_empty());
    game_loop.load_scene(preview).unwrap();
    app.game_loop = Some(game_loop);
    app.editor_scene = Some(EditorScene::new(scene));
    app
}

fn dispatch_test_request(
    app: &mut EditorApp,
    method: &str,
    params: serde_json::Value,
) -> serde_json::Value {
    let request = serde_json::json!({
        "id": format!("test-{method}"),
        "protocol": protocol::EDITOR_PROTOCOL,
        "sessionId": app.session_id.as_str(),
        "baseRevision": app.editor_revision,
        "method": method,
        "params": params,
    });
    let messages = app.dispatch_ipc_json(&request.to_string());
    serde_json::from_str(
        messages
            .json_messages
            .first()
            .expect("dispatch must return a response"),
    )
    .unwrap()
}

fn make_scene_dirty(app: &mut EditorApp, name: &str) {
    app.editor_scene
        .as_mut()
        .unwrap()
        .execute(Box::new(engine_editor::SetEntityName::new(
            "cube-01".into(),
            Some(name.into()),
        )))
        .unwrap();
}

fn persisted_cube_name(app: &EditorApp) -> Option<String> {
    Scene::load_from_file(&app.current_scene_path)
        .unwrap()
        .entities
        .into_iter()
        .find(|entity| entity.persistent_id == "cube-01")
        .and_then(|entity| entity.name)
}

#[derive(Default)]
struct ResizeBackend;

impl BackendRenderer for ResizeBackend {
    fn resize(
        &mut self,
        _width: u32,
        _height: u32,
    ) -> Result<(), Vec<engine_renderer::Diagnostic>> {
        Ok(())
    }
}

fn send_viewport_bounds(app: &mut EditorApp, viewport: &str, visible: bool, rect: ScreenRect) {
    let request = serde_json::json!({
        "id": "viewport-bounds-test",
        "protocol": protocol::EDITOR_PROTOCOL,
        "sessionId": app.session_id.as_str(),
        "method": "viewport.bounds",
        "params": {
            "viewport": viewport,
            "visible": visible,
            "rect": rect,
        },
    });
    let _ = app.dispatch_ipc_json(&request.to_string());
}

fn seed_active_web_input(app: &mut EditorApp) {
    app.web_viewport_input.pointer_id = Some(7);
    app.web_viewport_input.pointer = Some(Vec2::new(20.0, 30.0));
    app.web_viewport_input.buttons = 1;
    app.web_viewport_input.keys.insert("KeyW".to_string());
    app.web_viewport_input.focused = true;
}

#[test]
fn hidden_or_switched_web_viewports_cancel_captured_input() {
    let fixture = scene_project_fixture();
    let mut app = editor_app_with_loaded_fixture(fixture.project);
    app.web_viewport_rect = ScreenRect {
        x: 10.0,
        y: 20.0,
        width: 640.0,
        height: 480.0,
    };
    seed_active_web_input(&mut app);

    send_viewport_bounds(&mut app, "scene", false, ScreenRect::default());
    assert!(app.web_viewport_input.pointer_id.is_none());
    assert_eq!(app.web_viewport_input.buttons, 0);
    assert!(app.web_viewport_input.keys.is_empty());
    assert!(!app.web_viewport_input.focused);
    assert_eq!(app.web_viewport_rect.width, 0.0);

    seed_active_web_input(&mut app);
    send_viewport_bounds(
        &mut app,
        "game",
        true,
        ScreenRect {
            x: 30.0,
            y: 40.0,
            width: 800.0,
            height: 450.0,
        },
    );
    assert_eq!(app.viewport_tab, ViewportTab::Game);
    assert!(app.web_viewport_input.pointer_id.is_none());
    assert_eq!(app.web_viewport_input.buttons, 0);
    assert!(app.web_viewport_input.keys.is_empty());
    assert_eq!(app.web_viewport_rect.x, 30.0);
    assert_eq!(app.web_viewport_rect.width, 800.0);
}

#[test]
fn minimized_or_occluded_surfaces_stop_redraw_and_resume_once_visible() {
    let fixture = scene_project_fixture();
    let mut app = editor_app_with_loaded_fixture(fixture.project);
    app.game_loop
        .as_mut()
        .unwrap()
        .runtime
        .set_renderer_backend(Box::<ResizeBackend>::default());
    seed_active_web_input(&mut app);

    assert_eq!(
        app.handle_native_surface_resize(0, 0, None),
        HostDirective::Continue
    );
    assert!(app.surface_zero_sized);
    assert!(app.web_viewport_input.pointer_id.is_none());
    let frame = app.frame;
    assert_eq!(
        app.on_host_event(HostEvent::Redraw),
        HostDirective::Continue
    );
    assert_eq!(app.frame, frame);

    assert_eq!(
        app.on_host_event(HostEvent::Occluded(true)),
        HostDirective::Continue
    );
    assert_eq!(
        app.handle_native_surface_resize(1280, 720, None),
        HostDirective::Continue
    );
    assert!(!app.surface_zero_sized);
    assert!(app.surface_occluded);
    assert_eq!(
        app.on_host_event(HostEvent::Occluded(false)),
        HostDirective::RequestRedraw
    );
    assert!(!app.surface_render_suspended());
}

#[test]
fn renderer_failure_stops_self_redraw_and_external_retry_can_recover() {
    let fixture = scene_project_fixture();
    let mut app = editor_app_with_loaded_fixture(fixture.project);
    app.web_viewport_rect = ScreenRect {
        x: 0.0,
        y: 0.0,
        width: 640.0,
        height: 480.0,
    };
    app.window_w = 640.0;
    app.window_h = 480.0;

    assert_eq!(
        app.on_host_event(HostEvent::Redraw),
        HostDirective::Continue
    );
    assert!(app.render_faulted);
    assert_eq!(
        app.on_host_event(HostEvent::Redraw),
        HostDirective::Continue
    );

    app.game_loop
        .as_mut()
        .unwrap()
        .runtime
        .set_renderer_backend(Box::<crate::qa::QaBackend>::default());
    assert_eq!(
        app.on_host_event(HostEvent::Redraw),
        HostDirective::RequestRedraw
    );
    assert!(!app.render_faulted);
}

#[test]
fn periodic_frame_event_serializes_only_complete_telemetry_domains() {
    let fixture = scene_project_fixture();
    let mut app = editor_app_with_loaded_fixture(fixture.project);
    let messages = app.take_frame_bridge_messages(true);

    assert_eq!(messages.len(), 1);
    let event: serde_json::Value = serde_json::from_str(&messages[0]).unwrap();
    assert_eq!(event["event"], protocol::TELEMETRY_EVENT);
    assert_eq!(event["revision"], app.editor_revision);
    let params = event["params"].as_object().unwrap();
    assert!(params.contains_key("performance"));
    assert!(params.contains_key("animation"));
    assert!(params.contains_key("build"));
    assert!(params.contains_key("terrain"));
    assert_eq!(params.len(), 4);
    assert!(!params.contains_key("hierarchy"));
    assert!(!params.contains_key("projectName"));
}

#[test]
fn completed_background_job_bumps_revision_and_sends_one_full_snapshot() {
    let fixture = scene_project_fixture();
    let mut app = editor_app_with_loaded_fixture(fixture.project);
    let revision = app.editor_revision;
    let (sender, receiver) = mpsc::channel();
    sender
        .send(Err("expected worker failure".to_string()))
        .unwrap();
    app.background_job = Some(EditorBackgroundJob {
        id: 77,
        label: "Async asset operation".to_string(),
        receiver,
        reload_assets: false,
    });

    assert_eq!(app.render_react_frame(), EditorFrameOutcome::Completed);
    assert_eq!(app.editor_revision, revision.wrapping_add(1));
    assert!(app.pending_full_snapshot);

    let messages = app.take_frame_bridge_messages(true);
    let project_events = messages
        .iter()
        .filter_map(|message| serde_json::from_str::<serde_json::Value>(message).ok())
        .filter(|event| event["event"] == protocol::PROJECT_CHANGED_EVENT)
        .collect::<Vec<_>>();
    assert_eq!(project_events.len(), 1);
    assert_eq!(project_events[0]["revision"], app.editor_revision);
    assert!(project_events[0]["params"].get("hierarchy").is_some());
    assert_eq!(
        project_events[0]["params"]["backgroundOperations"][0]["id"],
        77
    );
    assert_eq!(
        project_events[0]["params"]["backgroundOperations"][0]["state"],
        "failed"
    );
    assert!(!app.pending_full_snapshot);
}

#[test]
fn recent_background_operation_statuses_survive_a_newer_job() {
    let fixture = scene_project_fixture();
    let mut app = editor_app_with_loaded_fixture(fixture.project);
    app.set_editor_operation_status(EditorOperationStatus {
        id: 10,
        label: "First".into(),
        state: EditorOperationState::Succeeded,
    });
    app.set_editor_operation_status(EditorOperationStatus {
        id: 11,
        label: "Second".into(),
        state: EditorOperationState::Running,
    });

    let snapshot = serde_json::to_value(app.editor_snapshot()).unwrap();
    let operations = snapshot["backgroundOperations"].as_array().unwrap();
    assert_eq!(operations.len(), 2);
    assert_eq!(operations[0]["id"], 10);
    assert_eq!(operations[0]["state"], "succeeded");
    assert_eq!(operations[1]["id"], 11);
    assert_eq!(operations[1]["state"], "running");
}
