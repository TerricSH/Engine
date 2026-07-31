use super::*;

#[test]
fn update_gizmo_pointer_up_ends_drag() {
    let mut g = GizmoSystem::new();
    g.dragging = true;
    g.drag_axis = Some(GizmoAxis::X);
    g.delta = Vec3::new(1.0, 0.0, 0.0);
    g.raw_drag_total = 2.0;
    g.applied_drag_total = 2.0;

    let consumed = update_gizmo(
        &mut g,
        Vec3::ZERO,
        Quat::IDENTITY,
        &Mat4::IDENTITY,
        &Mat4::IDENTITY,
        Vec2::new(1920.0, 1080.0),
        Vec2::new(100.0, 100.0),
        false, // pointer up
    );
    assert!(!consumed);
    assert!(!g.dragging);
    assert!(g.drag_axis.is_none());
    assert_eq!(g.take_delta(), Vec3::ZERO);
    assert_eq!(g.raw_drag_total, 0.0);
    assert_eq!(g.applied_drag_total, 0.0);
}

#[test]
fn cancel_drag_clears_transient_state_but_preserves_configuration() {
    let mut gizmo = GizmoSystem {
        mode: GizmoMode::Rotate,
        space: GizmoSpace::Local,
        snapping: true,
        snap_value: 15.0,
        dragging: true,
        drag_axis: Some(GizmoAxis::Y),
        last_pointer: Vec2::new(12.0, 34.0),
        delta: Vec3::Y,
        raw_drag_total: 0.7,
        applied_drag_total: 0.5,
    };

    gizmo.cancel_drag();

    assert!(!gizmo.dragging);
    assert!(gizmo.drag_axis.is_none());
    assert_eq!(gizmo.last_pointer, Vec2::ZERO);
    assert_eq!(gizmo.delta, Vec3::ZERO);
    assert_eq!(gizmo.raw_drag_total, 0.0);
    assert_eq!(gizmo.applied_drag_total, 0.0);
    assert_eq!(gizmo.mode, GizmoMode::Rotate);
    assert_eq!(gizmo.space, GizmoSpace::Local);
    assert!(gizmo.snapping);
    assert_eq!(gizmo.snap_value, 15.0);
}

// ── snap_delta ──────────────────────────────────────────────────

#[test]
fn snap_translate_delta() {
    // 0.63 snaps to 0.5 at snap=0.5
    let amount = snap_amount(0.63, 0.5, GizmoMode::Translate);
    assert!((amount - 0.5).abs() < 0.001);
}

#[test]
fn gesture_snapping_accumulates_sub_grid_movements_and_emits_difference() {
    let mut gizmo = GizmoSystem {
        snapping: true,
        snap_value: 0.5,
        mode: GizmoMode::Translate,
        drag_axis: Some(GizmoAxis::X),
        dragging: true,
        ..GizmoSystem::new()
    };

    assert_eq!(accumulate_gesture_amount(&mut gizmo, 0.1), 0.0);
    assert_eq!(accumulate_gesture_amount(&mut gizmo, 0.1), 0.0);
    assert_eq!(
        accumulate_gesture_amount(&mut gizmo, 0.1),
        0.5,
        "three small movements must cross the 0.5 snap threshold"
    );
    assert_eq!(
        accumulate_gesture_amount(&mut gizmo, -0.2),
        -0.5,
        "moving back across the threshold must emit only the correction"
    );
}

#[test]
fn local_x_translate_snaps_along_rotated_world_axis() {
    let mut gizmo = GizmoSystem {
        mode: GizmoMode::Translate,
        space: GizmoSpace::Local,
        snapping: true,
        snap_value: 0.5,
        dragging: true,
        drag_axis: Some(GizmoAxis::X),
        last_pointer: Vec2::new(50.0, 50.0),
        ..GizmoSystem::new()
    };
    let rotation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
    let viewport = Vec2::splat(100.0);

    for pointer_y in [45.0, 40.0] {
        assert!(update_gizmo(
            &mut gizmo,
            Vec3::ZERO,
            rotation,
            &Mat4::IDENTITY,
            &Mat4::IDENTITY,
            viewport,
            Vec2::new(50.0, pointer_y),
            true,
        ));
        assert_eq!(gizmo.take_delta(), Vec3::ZERO);
    }
    assert!(update_gizmo(
        &mut gizmo,
        Vec3::ZERO,
        rotation,
        &Mat4::IDENTITY,
        &Mat4::IDENTITY,
        viewport,
        Vec2::new(50.0, 35.0),
        true,
    ));
    let delta = gizmo.take_delta();
    assert!(delta.x.abs() < 0.001);
    assert!((delta.y - 0.5).abs() < 0.001);
    assert!(delta.z.abs() < 0.001);
}

#[test]
fn local_x_scale_uses_canonical_scale_axis_after_rotated_drag() {
    let mut gizmo = GizmoSystem {
        mode: GizmoMode::Scale,
        space: GizmoSpace::Local,
        snapping: true,
        snap_value: 0.5,
        dragging: true,
        drag_axis: Some(GizmoAxis::X),
        last_pointer: Vec2::new(50.0, 50.0),
        ..GizmoSystem::new()
    };
    let rotation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
    let viewport = Vec2::splat(100.0);

    for pointer_y in [45.0, 40.0, 35.0] {
        assert!(update_gizmo(
            &mut gizmo,
            Vec3::ZERO,
            rotation,
            &Mat4::IDENTITY,
            &Mat4::IDENTITY,
            viewport,
            Vec2::new(50.0, pointer_y),
            true,
        ));
    }
    assert_eq!(gizmo.take_delta(), Vec3::new(0.5, 0.0, 0.0));
}

#[test]
fn global_space_toggle_keeps_scale_drag_on_rotated_local_handle() {
    let mut gizmo = GizmoSystem {
        mode: GizmoMode::Scale,
        space: GizmoSpace::Global,
        snapping: true,
        snap_value: 0.5,
        dragging: true,
        drag_axis: Some(GizmoAxis::X),
        last_pointer: Vec2::new(50.0, 50.0),
        ..GizmoSystem::new()
    };
    let rotation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
    let viewport = Vec2::splat(100.0);

    for pointer_y in [45.0, 40.0, 35.0] {
        assert!(update_gizmo(
            &mut gizmo,
            Vec3::ZERO,
            rotation,
            &Mat4::IDENTITY,
            &Mat4::IDENTITY,
            viewport,
            Vec2::new(50.0, pointer_y),
            true,
        ));
    }
    assert_eq!(gizmo.take_delta(), Vec3::new(0.5, 0.0, 0.0));
}

#[test]
fn snap_rotate_delta() {
    // 30 degrees = 0.5236 rad; 0.53 should snap to that
    let amount = snap_amount(0.53, 30.0, GizmoMode::Rotate);
    let expected = 30.0_f32.to_radians();
    assert!((amount - expected).abs() < 0.01);
}

#[test]
fn snap_zero_snap_value_passthrough() {
    let amount = snap_amount(0.37, 0.0, GizmoMode::Translate);
    assert!((amount - 0.37).abs() < 0.001);
}

#[test]
fn rotate_delta_is_written_to_the_dragged_axis() {
    let viewport = Vec2::new(2.0, 2.0);
    let pointer_last = Vec2::new(2.0, 1.0);
    let pointer = Vec2::new(1.0, 2.0);
    let y = compute_rotate_delta(
        pointer,
        pointer_last,
        Vec3::ZERO,
        GizmoAxis::Y,
        &Mat4::IDENTITY,
        &Mat4::IDENTITY,
        viewport,
    );
    let z = compute_rotate_delta(
        pointer,
        pointer_last,
        Vec3::ZERO,
        GizmoAxis::Z,
        &Mat4::IDENTITY,
        &Mat4::IDENTITY,
        viewport,
    );
    assert!(y.x.abs() < f32::EPSILON && y.z.abs() < f32::EPSILON);
    assert!((y.y - std::f32::consts::FRAC_PI_2).abs() < 0.001);
    assert!(z.x.abs() < f32::EPSILON && z.y.abs() < f32::EPSILON);
    assert!((z.z - std::f32::consts::FRAC_PI_2).abs() < 0.001);
}

// ── Internal helpers ────────────────────────────────────────────
