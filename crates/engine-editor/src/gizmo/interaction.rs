use super::math::{
    accumulate_gesture_amount, compute_rotate_delta, compute_scale_delta, compute_translate_delta,
    screen_distance_to_arrow, screen_distance_to_cube, screen_distance_to_ring,
};
use super::state::{GIZMO_LENGTH, GIZMO_RING_RADIUS, HIT_THRESHOLD_PX};
use super::*;

// ---------------------------------------------------------------------------
// update_gizmo
// ---------------------------------------------------------------------------

/// Run gizmo hit-testing and drag tracking.
///
/// **Parameters**
/// - `system`         – gizmo state (mutated in-place).
/// - `gizmo_position` – world-space position of the gizmo (typically the
///   selected entity's translation).
/// - `gizmo_rotation` – world-space rotation of the gizmo (used when
///   [`GizmoSpace::Local`] is active).
/// - `view_matrix`    – camera view matrix.
/// - `proj_matrix`    – camera projection matrix.
/// - `viewport_size`  – viewport dimensions in pixels.
/// - `pointer_pos`    – current pointer (mouse) position in pixels.
/// - `pointer_down`   – whether the primary pointer button is held.
///
/// **Returns** `true` if the gizmo consumed the input (hit or ongoing drag).
///
/// When `true` is returned, call [`take_delta`](GizmoSystem::take_delta)
/// to retrieve the per-frame drag delta, then pass it to
/// [`EditorScene::preview_transform_gizmo_drag`].
#[allow(clippy::too_many_arguments)]
pub fn update_gizmo(
    system: &mut GizmoSystem,
    gizmo_position: Vec3,
    gizmo_rotation: Quat,
    view_matrix: &Mat4,
    proj_matrix: &Mat4,
    viewport_size: Vec2,
    pointer_pos: Vec2,
    pointer_down: bool,
) -> bool {
    // ── End drag on pointer release ─────────────────────────────────
    if !pointer_down && system.dragging {
        system.cancel_drag();
        return false;
    }

    // ── Continue active drag ────────────────────────────────────────
    if pointer_down && system.dragging {
        let axis = match system.drag_axis {
            Some(a) => a,
            None => return false,
        };

        let axis_dir = gizmo_axis_direction(system, gizmo_rotation, axis);

        let raw = match system.mode {
            GizmoMode::Translate => compute_translate_delta(
                pointer_pos,
                system.last_pointer,
                gizmo_position,
                axis_dir,
                view_matrix,
                proj_matrix,
                viewport_size,
            ),
            GizmoMode::Rotate => compute_rotate_delta(
                pointer_pos,
                system.last_pointer,
                gizmo_position,
                axis,
                view_matrix,
                proj_matrix,
                viewport_size,
            ),
            GizmoMode::Scale => compute_scale_delta(
                pointer_pos,
                system.last_pointer,
                gizmo_position,
                axis_dir,
                view_matrix,
                proj_matrix,
                viewport_size,
            ),
        };

        // Projection helpers return world-space vectors for translation and
        // scale. Accumulating their logical axis amount keeps snapping valid
        // when a Local axis is rotated away from its canonical world axis.
        let (raw_amount, output_axis) = match system.mode {
            GizmoMode::Translate => (raw.dot(axis_dir), axis_dir),
            GizmoMode::Rotate => {
                let logical_axis = axis.direction();
                (raw.dot(logical_axis), logical_axis)
            }
            GizmoMode::Scale => (raw.dot(axis_dir), axis.direction()),
        };
        system.delta = output_axis * accumulate_gesture_amount(system, raw_amount);
        system.last_pointer = pointer_pos;
        return true;
    }

    // ── Start drag on fresh press ───────────────────────────────────
    if pointer_down && !system.dragging {
        let axes = [GizmoAxis::X, GizmoAxis::Y, GizmoAxis::Z];
        let mut best_dist = HIT_THRESHOLD_PX;
        let mut best_axis: Option<GizmoAxis> = None;
        let world_scale =
            gizmo_world_scale(gizmo_position, view_matrix, proj_matrix, viewport_size)
                .unwrap_or(1.0);

        for &axis in &axes {
            let axis_dir = gizmo_axis_direction(system, gizmo_rotation, axis);

            let dist = match system.mode {
                GizmoMode::Translate => screen_distance_to_arrow(
                    gizmo_position,
                    axis_dir,
                    GIZMO_LENGTH * world_scale,
                    pointer_pos,
                    view_matrix,
                    proj_matrix,
                    viewport_size,
                ),
                GizmoMode::Rotate => screen_distance_to_ring(
                    gizmo_position,
                    axis_dir,
                    GIZMO_RING_RADIUS * world_scale,
                    pointer_pos,
                    view_matrix,
                    proj_matrix,
                    viewport_size,
                ),
                GizmoMode::Scale => screen_distance_to_cube(
                    gizmo_position,
                    axis_dir,
                    GIZMO_LENGTH * world_scale,
                    pointer_pos,
                    view_matrix,
                    proj_matrix,
                    viewport_size,
                ),
            };

            if dist < best_dist {
                best_dist = dist;
                best_axis = Some(axis);
            }
        }

        if let Some(axis) = best_axis {
            system.dragging = true;
            system.drag_axis = Some(axis);
            system.last_pointer = pointer_pos;
            system.delta = Vec3::ZERO;
            system.raw_drag_total = 0.0;
            system.applied_drag_total = 0.0;
            return true;
        }
    }

    false
}

/// Resolve the world-space direction used to draw and hit-test an axis.
///
/// Scale is deliberately local-axis only, even when the translate/rotate
/// space toggle is set to [`GizmoSpace::Global`]. A world-aligned scale of an
/// arbitrarily rotated or non-uniformly scaled hierarchy cannot in general be
/// represented by changing only the child's local TRS scale components.
pub(crate) fn gizmo_axis_direction(
    system: &GizmoSystem,
    gizmo_rotation: Quat,
    axis: GizmoAxis,
) -> Vec3 {
    if system.mode == GizmoMode::Scale || system.space == GizmoSpace::Local {
        gizmo_rotation * axis.direction()
    } else {
        axis.direction()
    }
}
