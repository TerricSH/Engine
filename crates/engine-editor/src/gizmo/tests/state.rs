use super::*;

#[test]
fn gizmo_new_defaults() {
    let g = GizmoSystem::new();
    assert_eq!(g.mode, GizmoMode::Translate);
    assert_eq!(g.space, GizmoSpace::Global);
    assert!(!g.snapping);
    assert_eq!(g.snap_value, 0.5);
    assert!(!g.dragging);
    assert!(g.drag_axis.is_none());
}

#[test]
fn gizmo_default_impl() {
    let g = GizmoSystem::default();
    assert_eq!(g.mode, GizmoMode::Translate);
}

#[test]
fn gizmo_mode_switching() {
    let mut g = GizmoSystem::new();
    g.mode = GizmoMode::Rotate;
    assert_eq!(g.mode, GizmoMode::Rotate);
    g.mode = GizmoMode::Scale;
    assert_eq!(g.mode, GizmoMode::Scale);
}

#[test]
fn gizmo_space_switching() {
    let mut g = GizmoSystem::new();
    g.space = GizmoSpace::Local;
    assert_eq!(g.space, GizmoSpace::Local);
    g.space = GizmoSpace::Global;
    assert_eq!(g.space, GizmoSpace::Global);
}

#[test]
fn gizmo_axis_colors() {
    assert_eq!(GizmoAxis::X.color(), [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(GizmoAxis::Y.color(), [0.0, 1.0, 0.0, 1.0]);
    assert_eq!(GizmoAxis::Z.color(), [0.0, 0.0, 1.0, 1.0]);
}

#[test]
fn gizmo_axis_directions() {
    assert_eq!(GizmoAxis::X.direction(), Vec3::X);
    assert_eq!(GizmoAxis::Y.direction(), Vec3::Y);
    assert_eq!(GizmoAxis::Z.direction(), Vec3::Z);
}

// ── Drag state ──────────────────────────────────────────────────

#[test]
fn gizmo_drag_state() {
    let mut g = GizmoSystem::new();
    assert!(!g.dragging);
    assert!(g.drag_axis.is_none());
    g.dragging = true;
    g.drag_axis = Some(GizmoAxis::Y);
    assert!(g.dragging);
    assert_eq!(g.drag_axis, Some(GizmoAxis::Y));
}

#[test]
fn gizmo_snapping_toggle() {
    let mut g = GizmoSystem::new();
    g.snapping = true;
    assert!(g.snapping);
    g.snap_value = 1.0;
    assert!((g.snap_value - 1.0).abs() < f32::EPSILON);
}

// ── take_delta ──────────────────────────────────────────────────

#[test]
fn gizmo_take_delta() {
    let mut g = GizmoSystem::new();
    // Manually set internal delta
    g.delta = Vec3::new(1.0, 2.0, 3.0);
    let d = g.take_delta();
    assert_eq!(d, Vec3::new(1.0, 2.0, 3.0));
    // After take, delta is zero
    assert_eq!(g.take_delta(), Vec3::ZERO);
}

// ── update_gizmo (basic state machine) ──────────────────────────
