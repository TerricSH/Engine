use super::state::{GIZMO_TARGET_LENGTH_PX, RING_SEGMENTS};
use super::*;

// ===========================================================================
// Internal helpers
// ===========================================================================

/// Project a world-space point to screen coordinates.
pub(crate) fn project_world_to_screen(
    world_pos: Vec3,
    view: &Mat4,
    proj: &Mat4,
    viewport: Vec2,
) -> Option<Vec2> {
    if !world_pos.is_finite()
        || !view.is_finite()
        || !proj.is_finite()
        || !viewport.is_finite()
        || viewport.x <= 0.0
        || viewport.y <= 0.0
    {
        return None;
    }
    let clip = *proj * *view * world_pos.extend(1.0);
    if !clip.is_finite() || clip.w <= 1.0e-5 {
        return None;
    }
    let ndc = clip.truncate() / clip.w;
    if !ndc.is_finite()
        || ndc.z < 0.0
        || ndc.z > 1.0
        || ndc.x < -1.1
        || ndc.x > 1.1
        || ndc.y < -1.1
        || ndc.y > 1.1
    {
        return None;
    }
    Some(Vec2::new(
        (ndc.x * 0.5 + 0.5) * viewport.x,
        (1.0 - (ndc.y * 0.5 + 0.5)) * viewport.y,
    ))
}

/// Return the world-space scale that keeps the visible gizmo approximately
/// the same size on screen for perspective and orthographic cameras.
///
/// The scale is derived from one camera-right/up world unit at the gizmo
/// depth. Drawing and hit testing both use this value, so the visible handle
/// and its interactive target cannot drift apart as the camera moves.
pub(crate) fn gizmo_world_scale(
    world_pos: Vec3,
    view: &Mat4,
    proj: &Mat4,
    viewport: Vec2,
) -> Option<f32> {
    let origin = project_world_to_screen_unbounded(world_pos, view, proj, viewport)?;
    let inverse_view = view.inverse();
    if !inverse_view.is_finite() {
        return None;
    }

    let pixels_per_world_unit = [Vec3::X, Vec3::Y]
        .into_iter()
        .filter_map(|camera_axis| {
            let world_axis = inverse_view
                .transform_vector3(camera_axis)
                .normalize_or_zero();
            if world_axis == Vec3::ZERO || !world_axis.is_finite() {
                return None;
            }
            project_world_to_screen_unbounded(world_pos + world_axis, view, proj, viewport)
                .map(|screen| (screen - origin).length())
                .filter(|length| length.is_finite() && *length > 1.0e-4)
        })
        .fold(0.0_f32, f32::max);

    if pixels_per_world_unit <= 1.0e-4 {
        return None;
    }
    let scale = GIZMO_TARGET_LENGTH_PX / pixels_per_world_unit;
    scale.is_finite().then(|| scale.clamp(1.0e-4, 1.0e4))
}

/// Projection used only for screen-size measurement. Unlike the public
/// viewport projection helper it intentionally accepts x/y positions outside
/// the visible rectangle, because a one-unit camera-right sample may cross an
/// edge while the gizmo origin itself is still visible.
fn project_world_to_screen_unbounded(
    world_pos: Vec3,
    view: &Mat4,
    proj: &Mat4,
    viewport: Vec2,
) -> Option<Vec2> {
    if !world_pos.is_finite()
        || !view.is_finite()
        || !proj.is_finite()
        || !viewport.is_finite()
        || viewport.x <= 0.0
        || viewport.y <= 0.0
    {
        return None;
    }
    let clip = *proj * *view * world_pos.extend(1.0);
    if !clip.is_finite() || clip.w <= 1.0e-5 {
        return None;
    }
    let ndc = clip.truncate() / clip.w;
    if !ndc.is_finite() || ndc.z < 0.0 || ndc.z > 1.0 {
        return None;
    }
    Some(Vec2::new(
        (ndc.x * 0.5 + 0.5) * viewport.x,
        (1.0 - (ndc.y * 0.5 + 0.5)) * viewport.y,
    ))
}

/// Closest distance from point `p` to the line segment `[a, b]` in 2D.
pub(super) fn point_to_line_segment_distance(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let ap = p - a;
    let ab_len2 = ab.length_squared();
    if ab_len2 < 1e-12 {
        return (p - a).length();
    }
    let t = (ap.dot(ab) / ab_len2).clamp(0.0, 1.0);
    (p - (a + ab * t)).length()
}

/// Screen-space distance from pointer to an axis arrow (line segment).
pub(super) fn screen_distance_to_arrow(
    origin: Vec3,
    dir: Vec3,
    length: f32,
    pointer: Vec2,
    view: &Mat4,
    proj: &Mat4,
    viewport: Vec2,
) -> f32 {
    let Some(p0) = project_world_to_screen(origin, view, proj, viewport) else {
        return f32::MAX;
    };
    let Some(p1) = project_world_to_screen(origin + dir * length, view, proj, viewport) else {
        return f32::MAX;
    };
    point_to_line_segment_distance(pointer, p0, p1)
}

/// Screen-space distance from pointer to a rotation ring (approximated as
/// line segments).
pub(super) fn screen_distance_to_ring(
    center: Vec3,
    normal: Vec3,
    radius: f32,
    pointer: Vec2,
    view: &Mat4,
    proj: &Mat4,
    viewport: Vec2,
) -> f32 {
    let tangent = if normal.x.abs() > 0.9 {
        Vec3::Y.cross(normal).normalize()
    } else {
        Vec3::X.cross(normal).normalize()
    };
    let bitangent = normal.cross(tangent).normalize();
    let seg_angle = std::f32::consts::PI * 2.0 / RING_SEGMENTS as f32;

    let mut min_dist = f32::MAX;
    let Some(mut prev_screen) =
        project_world_to_screen(center + tangent * radius, view, proj, viewport)
    else {
        return f32::MAX;
    };
    let first_screen = prev_screen;

    for i in 1..RING_SEGMENTS {
        let a = i as f32 * seg_angle;
        let pos = center + tangent * a.cos() * radius + bitangent * a.sin() * radius;
        let Some(screen) = project_world_to_screen(pos, view, proj, viewport) else {
            return f32::MAX;
        };
        let d = point_to_line_segment_distance(pointer, prev_screen, screen);
        if d < min_dist {
            min_dist = d;
        }
        prev_screen = screen;
    }
    // Close the ring
    let d = point_to_line_segment_distance(pointer, prev_screen, first_screen);
    if d < min_dist {
        min_dist = d;
    }

    min_dist
}

/// Screen-space distance from pointer to a scale cube.
pub(super) fn screen_distance_to_cube(
    origin: Vec3,
    dir: Vec3,
    length: f32,
    pointer: Vec2,
    view: &Mat4,
    proj: &Mat4,
    viewport: Vec2,
) -> f32 {
    let tip = origin + dir * length;
    let Some(center_screen) = project_world_to_screen(tip, view, proj, viewport) else {
        return f32::MAX;
    };
    (pointer - center_screen).length()
}

/// Compute the world-space translation delta along `axis_dir` from a
/// pointer movement.
pub(super) fn compute_translate_delta(
    pointer: Vec2,
    last_pointer: Vec2,
    origin: Vec3,
    axis_dir: Vec3,
    view: &Mat4,
    proj: &Mat4,
    viewport: Vec2,
) -> Vec3 {
    let Some(origin_screen) = project_world_to_screen(origin, view, proj, viewport) else {
        return Vec3::ZERO;
    };
    let Some(tip_screen) = project_world_to_screen(origin + axis_dir, view, proj, viewport) else {
        return Vec3::ZERO;
    };
    let axis_screen = (tip_screen - origin_screen).normalize_or_zero();

    let mouse_delta = pointer - last_pointer;
    let screen_proj = mouse_delta.dot(axis_screen);

    let pixel_len = (tip_screen - origin_screen).length();
    if pixel_len < 0.001 {
        return Vec3::ZERO;
    }

    let world_amount = screen_proj / pixel_len;
    axis_dir * world_amount
}

/// Compute a rotation angle delta from a pointer movement.
///
/// Returns a [`Vec3`] where only the component corresponding to the drag
/// axis should be used by the scene-level gizmo preview.
pub(super) fn compute_rotate_delta(
    pointer: Vec2,
    last_pointer: Vec2,
    center: Vec3,
    axis: GizmoAxis,
    view: &Mat4,
    proj: &Mat4,
    viewport: Vec2,
) -> Vec3 {
    let Some(center_screen) = project_world_to_screen(center, view, proj, viewport) else {
        return Vec3::ZERO;
    };
    let angle_curr = (pointer - center_screen)
        .y
        .atan2((pointer - center_screen).x);
    let angle_last = (last_pointer - center_screen)
        .y
        .atan2((last_pointer - center_screen).x);
    axis.direction() * (angle_curr - angle_last)
}

/// Compute the scale delta from a pointer movement (same as translate,
/// but the caller interprets the result as a scale factor).
pub(super) fn compute_scale_delta(
    pointer: Vec2,
    last_pointer: Vec2,
    origin: Vec3,
    axis_dir: Vec3,
    view: &Mat4,
    proj: &Mat4,
    viewport: Vec2,
) -> Vec3 {
    compute_translate_delta(
        pointer,
        last_pointer,
        origin,
        axis_dir,
        view,
        proj,
        viewport,
    )
}

/// Snap a logical axis amount based on the current mode's units.
pub(super) fn snap_amount(amount: f32, snap: f32, mode: GizmoMode) -> f32 {
    if snap <= 0.0 {
        return amount;
    }
    let snap_val = match mode {
        GizmoMode::Rotate => snap.to_radians(),
        _ => snap,
    };
    (amount / snap_val).round() * snap_val
}

/// Convert an incremental logical-axis amount into the incremental amount that
/// should be applied after gesture-wide snapping. Snapping the accumulated
/// total avoids losing a series of sub-grid pointer movements.
pub(super) fn accumulate_gesture_amount(system: &mut GizmoSystem, raw_amount: f32) -> f32 {
    system.raw_drag_total += raw_amount;
    let target_total = if system.snapping {
        snap_amount(system.raw_drag_total, system.snap_value, system.mode)
    } else {
        system.raw_drag_total
    };
    let incremental = target_total - system.applied_drag_total;
    system.applied_drag_total = target_total;
    incremental
}
