fn gizmo_test_transform(translation: [f32; 3]) -> engine_scene::ComponentRecord {
    engine_scene::ComponentRecord {
        schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
        enabled: true,
        fields: std::collections::BTreeMap::from([
            ("translation".into(), Value::Vec3(translation)),
            ("rotation".into(), Value::Quat([0.0, 0.0, 0.0, 1.0])),
            ("scale".into(), Value::Vec3([1.0, 1.0, 1.0])),
        ]),
    }
}

fn gizmo_test_scene_and_runtime() -> (EditorScene, EngineRuntime) {
    let mut scene = engine_scene::sample_scene();
    scene
        .entities
        .iter_mut()
        .find(|entity| entity.persistent_id == "camera-main")
        .unwrap()
        .components
        .insert(
            "engine.transform".into(),
            gizmo_test_transform([0.0, 0.0, 0.0]),
        );
    scene
        .entities
        .iter_mut()
        .find(|entity| entity.persistent_id == "cube-01")
        .unwrap()
        .components
        .insert(
            "engine.transform".into(),
            gizmo_test_transform([0.0, 0.0, -5.0]),
        );

    let mut editor_scene = EditorScene::new(scene.clone());
    editor_scene.selected_entity = Some("cube-01".into());
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.load_scene(scene).unwrap();
    (editor_scene, runtime)
}

fn project_gizmo_test_point(view: RuntimeGizmoView, world: Vec3) -> Vec2 {
    let clip = view.projection * view.view * world.extend(1.0);
    let ndc = clip.truncate() / clip.w;
    view.viewport_origin
        + Vec2::new(
            (ndc.x * 0.5 + 0.5) * view.viewport_size.x,
            (1.0 - (ndc.y * 0.5 + 0.5)) * view.viewport_size.y,
        )
}

fn project_gizmo_test_axis_interior(view: RuntimeGizmoView, axis: Vec3) -> Vec2 {
    let center = project_gizmo_test_point(view, view.world_position);
    let unit_tip = project_gizmo_test_point(view, view.world_position + axis);
    // Production gizmos keep an 88 px screen-space length independent of
    // camera depth. Pick well inside that visible segment instead of
    // assuming one world unit is the rendered handle length.
    center + (unit_tip - center).normalize() * 44.0
}

fn full_test_viewport(width: u32, height: u32) -> RenderViewportContext {
    RenderViewportContext::new(width, height, RendererRect::FULL).unwrap()
}

#[test]
fn scene_view_picking_selects_visible_runtime_entity() {
    let (_, runtime) = gizmo_test_scene_and_runtime();
    let window_size = Vec2::splat(800.0);
    let viewport = full_test_viewport(800, 800);
    let pointer = runtime
        .with_world(|world| {
            let input =
                extract_renderer_input_from_world_with_viewport(world, 0, viewport).unwrap();
            let view = &input.views[0];
            let drawable = input
                .drawables
                .iter()
                .find(|drawable| drawable.entity.as_deref() == Some("cube-01"))
                .unwrap();
            let center = (Vec3::from_array(drawable.bounds.min)
                + Vec3::from_array(drawable.bounds.max))
                * 0.5;
            project_world_point(
                center,
                Mat4::from_cols_array(&view.view_matrix),
                Mat4::from_cols_array(&view.projection_matrix),
                window_size,
            )
            .unwrap()
            .0
        })
        .unwrap();

    assert_eq!(
        pick_runtime_entity(&runtime, 0, viewport, Vec2::ZERO, window_size, pointer,).as_deref(),
        Some("cube-01")
    );
    assert_eq!(
        pick_runtime_entity(
            &runtime,
            0,
            viewport,
            Vec2::new(200.0, 200.0),
            Vec2::new(600.0, 600.0),
            Vec2::new(100.0, 100.0),
        ),
        None
    );
}

#[test]
fn embedded_editor_viewport_drives_render_projection_picking_and_gizmo_geometry() {
    let (_, runtime) = gizmo_test_scene_and_runtime();
    let surface = Vec2::new(1000.0, 800.0);
    let viewport_rect = ScreenRect {
        x: 200.0,
        y: 100.0,
        width: 500.0,
        height: 500.0,
    };
    let (_, _, viewport) = editor_render_viewport(viewport_rect, 1.0, surface).unwrap();
    assert_eq!(
        viewport.output_rect(),
        RendererRect {
            min: [0.2, 0.125],
            max: [0.7, 0.75],
        }
    );

    let gizmo_view = runtime_gizmo_view(&runtime, "cube-01", 0, viewport).unwrap();
    assert_eq!(gizmo_view.viewport_origin, Vec2::new(200.0, 100.0));
    assert_eq!(gizmo_view.viewport_size, Vec2::splat(500.0));
    let pointer = project_gizmo_test_point(gizmo_view, gizmo_view.world_position);
    assert_eq!(
        pick_runtime_entity(
            &runtime,
            0,
            viewport,
            Vec2::new(200.0, 100.0),
            Vec2::new(700.0, 600.0),
            pointer,
        )
        .as_deref(),
        Some("cube-01")
    );
    assert_eq!(
        pick_runtime_entity(
            &runtime,
            0,
            viewport,
            Vec2::new(200.0, 100.0),
            Vec2::new(700.0, 600.0),
            Vec2::new(100.0, 400.0),
        ),
        None
    );
}

#[test]
fn queued_gizmo_drag_undo_save_reload_roundtrip() {
    let (mut editor_scene, runtime) = gizmo_test_scene_and_runtime();
    let viewport = full_test_viewport(800, 800);
    let view = runtime_gizmo_view(&runtime, "cube-01", 0, viewport).unwrap();
    let center = project_gizmo_test_point(view, view.world_position);
    let x_tip = project_gizmo_test_point(view, view.world_position + Vec3::X);
    let press = project_gizmo_test_axis_interior(view, Vec3::X);
    let moved = press + (x_tip - center).normalize() * 60.0;
    let mut gizmo = GizmoSystem::new();
    let processed = process_gizmo_pointer_events(
        vec![
            GizmoPointerEvent::Press(press),
            GizmoPointerEvent::Move(moved),
            GizmoPointerEvent::Release(moved),
        ],
        &mut editor_scene,
        &mut gizmo,
        &runtime,
        "cube-01",
        view,
    );
    assert!(
        processed,
        "dragging={} axis={:?} active={} transform={:?}",
        gizmo.dragging,
        gizmo.drag_axis,
        editor_scene.is_transform_gizmo_drag_active(),
        editor_scene.selected_transform_for_gizmo()
    );
    let changed = editor_scene.selected_transform_for_gizmo().unwrap();
    assert!(changed.translation.x > 0.1);
    let runtime_x = runtime
        .with_world(|world| {
            let entity = world.entity_by_persistent_id("cube-01").unwrap();
            world.get::<Transform>(entity).unwrap().translation.x
        })
        .unwrap();
    assert!((runtime_x - changed.translation.x).abs() < 1.0e-5);

    let overlay_view = runtime_gizmo_view(&runtime, "cube-01", 1, viewport).unwrap();
    let overlay = offset_gizmo_batch(
        build_gizmo_ui_batch(
            &gizmo,
            overlay_view.world_position,
            overlay_view.world_rotation,
            overlay_view.view,
            overlay_view.projection,
            overlay_view.viewport_size,
        )
        .unwrap(),
        overlay_view,
    );
    assert_eq!(overlay.clip_rect.min, [0.0, 0.0]);
    assert_eq!(overlay.clip_rect.max, [800.0, 800.0]);

    let temp = tempfile::tempdir().unwrap();
    let saved = temp.path().join("gizmo.scene.ron");
    save_scene_atomically(&editor_scene.scene, &saved).unwrap();
    let reloaded = Scene::load_from_file(&saved).unwrap();
    let reloaded_x = match reloaded
        .entities
        .iter()
        .find(|entity| entity.persistent_id == "cube-01")
        .unwrap()
        .components["engine.transform"]
        .fields["translation"]
    {
        Value::Vec3(value) => value[0],
        ref other => panic!("unexpected translation: {other:?}"),
    };
    assert!((reloaded_x - changed.translation.x).abs() < 1.0e-5);

    editor_scene.undo().unwrap();
    assert_eq!(
        editor_scene
            .selected_transform_for_gizmo()
            .unwrap()
            .translation,
        Vec3::new(0.0, 0.0, -5.0)
    );
    assert!(!editor_scene.history.can_undo());
    assert!(editor_scene.history.can_redo());
}

#[test]
fn gizmo_release_position_applies_final_drag_segment() {
    let (mut editor_scene, runtime) = gizmo_test_scene_and_runtime();
    let view = runtime_gizmo_view(&runtime, "cube-01", 0, full_test_viewport(800, 800)).unwrap();
    let center = project_gizmo_test_point(view, view.world_position);
    let x_tip = project_gizmo_test_point(view, view.world_position + Vec3::X);
    let press = project_gizmo_test_axis_interior(view, Vec3::X);
    let released = press + (x_tip - center).normalize() * 60.0;
    let mut gizmo = GizmoSystem::new();
    assert!(process_gizmo_pointer_events(
        vec![
            GizmoPointerEvent::Press(press),
            GizmoPointerEvent::Release(released),
        ],
        &mut editor_scene,
        &mut gizmo,
        &runtime,
        "cube-01",
        view,
    ));
    assert!(
        editor_scene
            .selected_transform_for_gizmo()
            .unwrap()
            .translation
            .x
            > 0.1
    );
    assert!(editor_scene.history.can_undo());
    assert!(!gizmo.dragging);
}

#[test]
fn gizmo_overlay_and_hit_testing_share_the_scene_interaction_rect() {
    let (mut editor_scene, runtime) = gizmo_test_scene_and_runtime();
    let full_view =
        runtime_gizmo_view(&runtime, "cube-01", 0, full_test_viewport(800, 800)).unwrap();
    let visible_view =
        restrict_gizmo_view_to_rect(full_view, Vec2::new(250.0, 200.0), Vec2::new(650.0, 600.0))
            .unwrap();
    let gizmo = GizmoSystem::new();
    let overlay = offset_gizmo_batch(
        build_gizmo_ui_batch(
            &gizmo,
            visible_view.world_position,
            visible_view.world_rotation,
            visible_view.view,
            visible_view.projection,
            visible_view.viewport_size,
        )
        .unwrap(),
        visible_view,
    );
    assert_eq!(overlay.clip_rect.min, [250.0, 200.0]);
    assert_eq!(overlay.clip_rect.max, [650.0, 600.0]);

    let excluded_view =
        restrict_gizmo_view_to_rect(full_view, Vec2::new(0.0, 0.0), Vec2::new(100.0, 100.0))
            .unwrap();
    let center = project_gizmo_test_point(full_view, full_view.world_position);
    let x_tip = project_gizmo_test_point(full_view, full_view.world_position + Vec3::X);
    let press = center.lerp(x_tip, 0.55);
    let mut gizmo = GizmoSystem::new();
    assert!(!process_gizmo_pointer_events(
        vec![GizmoPointerEvent::Press(press)],
        &mut editor_scene,
        &mut gizmo,
        &runtime,
        "cube-01",
        excluded_view,
    ));
    assert!(!gizmo.dragging);
    assert!(!editor_scene.is_transform_gizmo_drag_active());
}

#[test]
fn queued_gizmo_cancel_restores_preview_without_history() {
    let (mut editor_scene, mut runtime) = gizmo_test_scene_and_runtime();
    let view = runtime_gizmo_view(&runtime, "cube-01", 0, full_test_viewport(800, 800)).unwrap();
    let center = project_gizmo_test_point(view, view.world_position);
    let x_tip = project_gizmo_test_point(view, view.world_position + Vec3::X);
    let press = project_gizmo_test_axis_interior(view, Vec3::X);
    let moved = press + (x_tip - center).normalize() * 50.0;
    let mut gizmo = GizmoSystem::new();
    assert!(process_gizmo_pointer_events(
        vec![
            GizmoPointerEvent::Press(press),
            GizmoPointerEvent::Move(moved),
            GizmoPointerEvent::Cancel,
        ],
        &mut editor_scene,
        &mut gizmo,
        &runtime,
        "cube-01",
        view,
    ));
    assert_eq!(
        editor_scene
            .selected_transform_for_gizmo()
            .unwrap()
            .translation,
        Vec3::new(0.0, 0.0, -5.0)
    );
    assert!(!editor_scene.history.can_undo());
    assert!(!gizmo.dragging);
    runtime.load_scene(editor_scene.scene.clone()).unwrap();
    let runtime_translation = runtime
        .with_world(|world| {
            let entity = world.entity_by_persistent_id("cube-01").unwrap();
            world.get::<Transform>(entity).unwrap().translation
        })
        .unwrap();
    assert_eq!(runtime_translation, Vec3::new(0.0, 0.0, -5.0));
}

#[test]
fn scene_view_controls_drive_runtime_editor_camera_only() {
    let (editor_scene, _) = gizmo_test_scene_and_runtime();
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    let (preview, diagnostics) = editor_preview_scene(&runtime, &editor_scene.scene);
    assert!(diagnostics.is_empty());
    runtime.load_scene(preview).unwrap();
    let authored_camera = editor_scene
        .scene
        .entities
        .iter()
        .find(|entity| entity.persistent_id == "camera-main")
        .unwrap()
        .components["engine.transform"]
        .fields["translation"]
        .clone();
    let mut panel = SceneViewPanel::new("Scene View");
    panel.set_target([0.0, 0.0, 0.0]);
    panel.set_camera_orbit(0.0, 0.0, 7.0);

    assert!(apply_editor_camera(&runtime, &panel));
    let (editor_camera, runtime_authored_camera) = runtime
        .with_world(|world| {
            let editor_id = world.scene_settings().active_camera.as_deref().unwrap();
            let editor_entity = world.entity_by_persistent_id(editor_id).unwrap();
            let authored_entity = world.entity_by_persistent_id("camera-main").unwrap();
            (
                world.get::<Transform>(editor_entity).unwrap().clone(),
                world.get::<Transform>(authored_entity).unwrap().clone(),
            )
        })
        .unwrap();
    assert!((editor_camera.translation - Vec3::new(7.0, 0.0, 0.0)).length() < 1.0e-5);
    assert!((editor_camera.rotation * -Vec3::Z - Vec3::NEG_X).length() < 1.0e-5);
    assert_eq!(runtime_authored_camera.translation, Vec3::ZERO);
    assert_eq!(
        editor_scene
            .scene
            .entities
            .iter()
            .find(|entity| entity.persistent_id == "camera-main")
            .unwrap()
            .components["engine.transform"]
            .fields["translation"],
        authored_camera
    );

    let view =
        runtime_gizmo_view(&runtime, "camera-main", 0, full_test_viewport(800, 800)).unwrap();
    assert!(build_gizmo_ui_batch(
        &GizmoSystem::new(),
        view.world_position,
        view.world_rotation,
        view.view,
        view.projection,
        view.viewport_size,
    )
    .is_some());
}

#[test]
fn scene_view_camera_state_survives_ordered_preview_synchronization() {
    let (mut editor_scene, _) = gizmo_test_scene_and_runtime();
    let mut game_loop = GameLoop::new(EngineConfig::default());
    let mut panel = SceneViewPanel::new("Scene View");
    panel.apply_action(engine_editor::SceneViewAction::SetDistance(25.0));

    synchronize_editor_preview_and_camera(&mut game_loop, &mut editor_scene, &panel);

    let editor_camera_translation = game_loop
        .runtime
        .with_world(|world| {
            let camera_id = world.scene_settings().active_camera.as_deref().unwrap();
            let camera = world.entity_by_persistent_id(camera_id).unwrap();
            world.get::<Transform>(camera).unwrap().translation
        })
        .unwrap();
    let pitch = 20.0_f32.to_radians();
    let yaw = 45.0_f32.to_radians();
    let expected = Vec3::new(
        25.0 * yaw.cos() * pitch.cos(),
        25.0 * pitch.sin(),
        25.0 * yaw.sin() * pitch.cos(),
    );
    assert!((editor_camera_translation - expected).length() < 1.0e-5);
}
