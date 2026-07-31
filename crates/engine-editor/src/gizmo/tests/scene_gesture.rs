use super::*;

#[test]
fn scene_gizmo_begin_safely_rejects_no_selection_or_transform() {
    let mut editor = EditorScene::new(engine_scene::sample_scene());
    assert!(!editor.begin_transform_gizmo_drag());
    assert!(!editor.is_transform_gizmo_drag_active());

    editor.selected_entity = Some("cube-01".into());
    assert!(!editor.begin_transform_gizmo_drag());
    assert!(!editor.preview_transform_gizmo_drag(&GizmoSystem::new(), Vec3::X));
    assert!(!editor.commit_transform_gizmo_drag().unwrap());
    assert!(!editor.cancel_transform_gizmo_drag());

    editor.selected_entity = Some("stale-real-id".into());
    assert!(!editor.begin_transform_gizmo_drag());
    assert!(!editor.history.can_undo());
    assert!(!editor.is_dirty());
}

#[test]
fn selected_transform_for_gizmo_uses_defaults_and_rejects_bad_values() {
    let mut editor = editor_scene_with_transform(&[("translation", Value::Vec3([2.0, 3.0, 4.0]))]);
    let transform = editor.selected_transform_for_gizmo().unwrap();
    assert_eq!(transform.translation, Vec3::new(2.0, 3.0, 4.0));
    assert_eq!(transform.rotation, Quat::IDENTITY);
    assert_eq!(transform.scale, Vec3::ONE);
    assert!(transform.parent.is_none());

    editor.selected_entity = None;
    assert!(editor.selected_transform_for_gizmo().is_none());
    editor.selected_entity = Some("cube-01".into());
    set_transform_field(
        &mut editor.scene,
        &"cube-01".to_string(),
        "rotation",
        Value::Vec3([0.0; 3]),
    )
    .unwrap();
    assert!(editor.selected_transform_for_gizmo().is_none());
}

#[test]
fn scene_gizmo_previews_then_commits_one_real_id_undo() {
    let mut editor = editor_scene_with_transform(&[
        ("translation", Value::Vec3([1.0, 2.0, 3.0])),
        ("rotation", Value::Quat([0.0, 0.0, 0.0, 1.0])),
        ("scale", Value::Vec3([1.0, 1.0, 1.0])),
    ]);
    let gizmo = GizmoSystem {
        mode: GizmoMode::Translate,
        dragging: true,
        drag_axis: Some(GizmoAxis::X),
        ..GizmoSystem::new()
    };

    assert!(editor.begin_transform_gizmo_drag());
    assert!(!editor.begin_transform_gizmo_drag());
    assert!(editor.preview_transform_gizmo_drag(&gizmo, Vec3::new(2.0, 0.0, 0.0)));
    assert!(editor.preview_transform_gizmo_drag(&gizmo, Vec3::new(1.0, 0.0, 0.0)));
    assert_eq!(
        transform_field(&editor, "translation"),
        Some(&Value::Vec3([4.0, 2.0, 3.0]))
    );
    assert!(!editor.history.can_undo(), "preview must not enter history");
    assert!(!editor.is_dirty(), "preview must not dirty the scene");

    // Changing selection during the drag must not redirect the captured
    // real persistent target ID.
    editor.selected_entity = Some("camera-main".into());
    assert!(editor.commit_transform_gizmo_drag().unwrap());
    assert!(!editor.is_transform_gizmo_drag_active());
    assert_eq!(editor.history.done.len(), 1);
    assert_eq!(
        editor.history.done.last().map(|entry| entry.command.name()),
        Some("Transform Gizmo Drag")
    );
    assert!(editor.is_dirty());

    editor.undo().unwrap();
    assert_eq!(
        transform_field(&editor, "translation"),
        Some(&Value::Vec3([1.0, 2.0, 3.0]))
    );
    assert!(
        !editor.history.can_undo(),
        "one gesture must create exactly one undo step"
    );
    editor.redo().unwrap();
    assert_eq!(
        transform_field(&editor, "translation"),
        Some(&Value::Vec3([4.0, 2.0, 3.0]))
    );
}

#[test]
fn parented_translate_converts_world_delta_through_parent_trs() {
    let parent_rotation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
    let mut editor = editor_scene_with_transform(&[("translation", Value::Vec3([1.0, 0.0, 0.0]))]);
    install_transform(
        &mut editor.scene,
        "camera-main",
        &[
            ("translation", Value::Vec3([10.0, 0.0, 0.0])),
            ("rotation", Value::Quat(parent_rotation.to_array())),
            ("scale", Value::Vec3([2.0, 1.0, 1.0])),
        ],
    );
    editor
        .scene
        .entities
        .iter_mut()
        .find(|entity| entity.persistent_id == "cube-01")
        .unwrap()
        .parent = Some("camera-main".into());
    let gizmo = GizmoSystem {
        mode: GizmoMode::Translate,
        dragging: true,
        drag_axis: Some(GizmoAxis::Y),
        ..GizmoSystem::new()
    };

    assert!(editor.begin_transform_gizmo_drag());
    assert!(editor.preview_transform_gizmo_drag(&gizmo, Vec3::new(0.0, 2.0, 0.0)));
    let Some(Value::Vec3(local_translation)) = transform_field(&editor, "translation") else {
        panic!("translation preview was not stored");
    };
    assert!((local_translation[0] - 2.0).abs() < 1.0e-5);
    assert!(local_translation[1].abs() < 1.0e-5);
    assert!(local_translation[2].abs() < 1.0e-5);
}

#[test]
fn parented_rotate_converts_global_and_local_modes_through_parent_rotation() {
    let parent_rotation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
    let mut editor =
        editor_scene_with_transform(&[("rotation", Value::Quat(Quat::IDENTITY.to_array()))]);
    install_transform(
        &mut editor.scene,
        "camera-main",
        &[("rotation", Value::Quat(parent_rotation.to_array()))],
    );
    editor
        .scene
        .entities
        .iter_mut()
        .find(|entity| entity.persistent_id == "cube-01")
        .unwrap()
        .parent = Some("camera-main".into());
    let angle = std::f32::consts::FRAC_PI_2;

    let global = GizmoSystem {
        mode: GizmoMode::Rotate,
        space: GizmoSpace::Global,
        dragging: true,
        drag_axis: Some(GizmoAxis::X),
        ..GizmoSystem::new()
    };
    assert!(editor.begin_transform_gizmo_drag());
    assert!(editor.preview_transform_gizmo_drag(&global, Vec3::new(angle, 0.0, 0.0)));
    let Some(Value::Quat(global_local)) = transform_field(&editor, "rotation") else {
        panic!("global rotation preview was not stored");
    };
    let actual_world = (parent_rotation * Quat::from_array(*global_local)).normalize();
    let expected_world = (Quat::from_rotation_x(angle) * parent_rotation).normalize();
    assert!(actual_world.dot(expected_world).abs() > 1.0 - 1.0e-5);
    assert!(editor.cancel_transform_gizmo_drag());

    let local = GizmoSystem {
        space: GizmoSpace::Local,
        ..global
    };
    assert!(editor.begin_transform_gizmo_drag());
    assert!(editor.preview_transform_gizmo_drag(&local, Vec3::new(angle, 0.0, 0.0)));
    let Some(Value::Quat(local_rotation)) = transform_field(&editor, "rotation") else {
        panic!("local rotation preview was not stored");
    };
    let expected_local = Quat::from_rotation_x(angle);
    assert!(Quat::from_array(*local_rotation).dot(expected_local).abs() > 1.0 - 1.0e-5);
}

#[test]
fn parented_scale_always_writes_local_axis_components() {
    let parent_rotation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
    let mut editor = editor_scene_with_transform(&[("scale", Value::Vec3([1.0, 2.0, 3.0]))]);
    install_transform(
        &mut editor.scene,
        "camera-main",
        &[
            ("rotation", Value::Quat(parent_rotation.to_array())),
            ("scale", Value::Vec3([2.0, 3.0, 4.0])),
        ],
    );
    editor
        .scene
        .entities
        .iter_mut()
        .find(|entity| entity.persistent_id == "cube-01")
        .unwrap()
        .parent = Some("camera-main".into());
    let gizmo = GizmoSystem {
        mode: GizmoMode::Scale,
        // The space toggle intentionally does not turn this into an
        // unrepresentable world-aligned scale operation.
        space: GizmoSpace::Global,
        dragging: true,
        drag_axis: Some(GizmoAxis::X),
        ..GizmoSystem::new()
    };

    assert!(editor.begin_transform_gizmo_drag());
    assert!(editor.preview_transform_gizmo_drag(&gizmo, Vec3::new(0.5, 0.0, 0.0)));
    assert_eq!(
        transform_field(&editor, "scale"),
        Some(&Value::Vec3([1.5, 2.0, 3.0]))
    );
    assert_eq!(
        gizmo_axis_direction(&gizmo, parent_rotation, GizmoAxis::X),
        parent_rotation * Vec3::X
    );
}

#[test]
fn selection_change_during_drag_never_retargets_preview() {
    let mut editor = editor_scene_with_transform(&[("translation", Value::Vec3([1.0, 0.0, 0.0]))]);
    install_transform(
        &mut editor.scene,
        "camera-main",
        &[("translation", Value::Vec3([20.0, 0.0, 0.0]))],
    );
    let gizmo = GizmoSystem {
        mode: GizmoMode::Translate,
        dragging: true,
        drag_axis: Some(GizmoAxis::X),
        ..GizmoSystem::new()
    };

    assert!(editor.begin_transform_gizmo_drag());
    assert_eq!(
        editor.active_transform_gizmo_entity().map(String::as_str),
        Some("cube-01")
    );
    editor.selected_entity = Some("camera-main".into());
    assert!(editor.preview_transform_gizmo_drag(&gizmo, Vec3::X));
    assert_eq!(
        transform_field(&editor, "translation"),
        Some(&Value::Vec3([2.0, 0.0, 0.0]))
    );
    let camera_translation = editor
        .scene
        .entities
        .iter()
        .find(|entity| entity.persistent_id == "camera-main")
        .unwrap()
        .components[TRANSFORM_COMPONENT_TYPE]
        .fields
        .get("translation");
    assert_eq!(camera_translation, Some(&Value::Vec3([20.0, 0.0, 0.0])));
    assert_eq!(
        editor.active_transform_gizmo_entity().map(String::as_str),
        Some("cube-01")
    );
    assert!(editor.cancel_transform_gizmo_drag());
    assert!(editor.active_transform_gizmo_entity().is_none());
}

#[test]
fn scene_gizmo_cancel_restores_exact_missing_fields_without_history() {
    let mut editor = editor_scene_with_transform(&[("translation", Value::Vec3([3.0, 4.0, 5.0]))]);
    let gizmo = GizmoSystem {
        mode: GizmoMode::Scale,
        dragging: true,
        drag_axis: Some(GizmoAxis::Y),
        ..GizmoSystem::new()
    };

    assert!(editor.begin_transform_gizmo_drag());
    assert!(editor.preview_transform_gizmo_drag(&gizmo, Vec3::new(0.0, 0.5, 0.0)));
    assert_eq!(
        transform_field(&editor, "scale"),
        Some(&Value::Vec3([1.0, 1.5, 1.0]))
    );
    assert!(editor.cancel_transform_gizmo_drag());
    assert_eq!(transform_field(&editor, "scale"), None);
    assert_eq!(transform_field(&editor, "rotation"), None);
    assert_eq!(
        transform_field(&editor, "translation"),
        Some(&Value::Vec3([3.0, 4.0, 5.0]))
    );
    assert!(!editor.history.can_undo());
    assert!(!editor.is_dirty());
}

#[test]
fn scene_gizmo_rotate_preview_commits_and_noop_commit_stays_clean() {
    let mut editor = editor_scene_with_transform(&[]);
    let gizmo = GizmoSystem {
        mode: GizmoMode::Rotate,
        dragging: true,
        drag_axis: Some(GizmoAxis::Z),
        ..GizmoSystem::new()
    };

    assert!(editor.begin_transform_gizmo_drag());
    assert!(!editor.commit_transform_gizmo_drag().unwrap());
    assert!(!editor.history.can_undo());

    assert!(editor.begin_transform_gizmo_drag());
    assert!(!editor.preview_transform_gizmo_drag(&gizmo, Vec3::splat(f32::NAN)));
    assert!(editor
        .preview_transform_gizmo_drag(&gizmo, Vec3::new(0.0, 0.0, std::f32::consts::FRAC_PI_2),));
    assert!(editor.commit_transform_gizmo_drag().unwrap());
    let Some(Value::Quat(rotation)) = transform_field(&editor, "rotation") else {
        panic!("rotation preview was not stored");
    };
    let (_, angle) = Quat::from_array(*rotation).to_axis_angle();
    assert!((angle - std::f32::consts::FRAC_PI_2).abs() < 0.001);
}

#[test]
fn editor_undo_during_preview_cancels_instead_of_touching_history() {
    let mut editor = editor_scene_with_transform(&[("translation", Value::Vec3([1.0, 0.0, 0.0]))]);
    let gizmo = GizmoSystem::new();
    assert!(editor.begin_transform_gizmo_drag());
    assert!(editor.preview_transform_gizmo_drag(&gizmo, Vec3::X));

    editor.undo().unwrap();
    assert_eq!(
        transform_field(&editor, "translation"),
        Some(&Value::Vec3([1.0, 0.0, 0.0]))
    );
    assert!(!editor.is_transform_gizmo_drag_active());
    assert!(!editor.history.can_undo());
}
