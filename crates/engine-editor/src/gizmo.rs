//! Editor gizmo system for 3D viewport manipulation.
//!
//! Provides translate, rotate, and scale gizmos with axis snapping,
//! local/global space support, and screen-space hit testing.
//!
//! # Usage
//!
//! ```ignore
//! let mut gizmo = GizmoSystem::new();
//!
//! // Each frame:
//! let consumed = update_gizmo(
//!     &mut gizmo, entity_pos, entity_rot,
//!     &view, &proj, viewport_size, pointer_pos, pointer_down,
//! );
//!
//! if consumed {
//!     let delta = gizmo.take_delta();
//!     apply_gizmo_drag(&gizmo, entity, &mut world, delta);
//! }
//!
//! draw_gizmo(&mut debug_buffer, &gizmo, &entity_transform);
//! ```

use glam::{Mat4, Quat, Vec2, Vec3};

use engine_renderer::DebugDrawBuffer;
use engine_scene::components::Transform;
use engine_scene::{Entity, World};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// X-axis colour — red.
const COLOR_X: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
/// Y-axis colour — green.
const COLOR_Y: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
/// Z-axis colour — blue.
const COLOR_Z: [f32; 4] = [0.0, 0.0, 1.0, 1.0];
/// Highlight colour (dragged axis).
const COLOR_HIGHLIGHT: [f32; 4] = [1.0, 1.0, 0.0, 1.0];

/// Length of translate arrow and scale axis lines in world units.
const GIZMO_LENGTH: f32 = 1.0;
/// Radius of rotate rings.
const GIZMO_RING_RADIUS: f32 = 0.8;
/// Half-extent of scale cubes.
const GIZMO_CUBE_HALF: f32 = 0.05;
/// Number of line segments used to approximate rotation rings.
const RING_SEGMENTS: u32 = 32;
/// Screen-space hit-test threshold in pixels.
const HIT_THRESHOLD_PX: f32 = 12.0;

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

/// Active gizmo manipulation mode.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GizmoMode {
    /// Translation arrows along each axis.
    Translate,
    /// Rotation rings around each axis.
    Rotate,
    /// Scale handles along each axis.
    Scale,
}

/// Reference space for gizmo axes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GizmoSpace {
    /// Align axes to the entity's local rotation.
    Local,
    /// Align axes to the world coordinate system.
    Global,
}

/// One of the three primary axes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GizmoAxis {
    X,
    Y,
    Z,
}

impl GizmoAxis {
    /// Return the canonical colour for this axis (X=red, Y=green, Z=blue).
    pub fn color(&self) -> [f32; 4] {
        match self {
            GizmoAxis::X => COLOR_X,
            GizmoAxis::Y => COLOR_Y,
            GizmoAxis::Z => COLOR_Z,
        }
    }

    /// Return the unit direction vector for this axis.
    pub fn direction(&self) -> Vec3 {
        match self {
            GizmoAxis::X => Vec3::X,
            GizmoAxis::Y => Vec3::Y,
            GizmoAxis::Z => Vec3::Z,
        }
    }
}

// ---------------------------------------------------------------------------
// GizmoSystem
// ---------------------------------------------------------------------------

/// Central state for the editor gizmo system.
///
/// Tracks the current mode, space, snap settings, entity selection, and
/// active drag state.  Per-frame drag deltas are accumulated and can be
/// consumed via [`take_delta`](GizmoSystem::take_delta).
pub struct GizmoSystem {
    /// Current manipulation mode.
    pub mode: GizmoMode,
    /// Reference space for axes.
    pub space: GizmoSpace,
    /// Whether snapping is enabled.
    pub snapping: bool,
    /// Snap increment (world-units for translate/scale, degrees for rotate).
    pub snap_value: f32,
    /// Currently selected entity identifier (caller-defined, e.g. packed
    /// entity index or persistent ID).
    pub selected_entity: Option<u64>,
    /// Whether the user is currently dragging a gizmo handle.
    pub dragging: bool,
    /// Which axis is being dragged (if any).
    pub drag_axis: Option<GizmoAxis>,

    // ── internal state ──────────────────────────────────────────────
    /// Pointer position from the previous frame (used for delta computation).
    last_pointer: Vec2,
    /// Per-frame delta accumulated by `update_gizmo`, consumed by caller
    /// via `take_delta`.
    delta: Vec3,
}

impl GizmoSystem {
    /// Create a new gizmo system with default settings.
    pub fn new() -> Self {
        Self {
            mode: GizmoMode::Translate,
            space: GizmoSpace::Global,
            snapping: false,
            snap_value: 0.5,
            selected_entity: None,
            dragging: false,
            drag_axis: None,
            last_pointer: Vec2::ZERO,
            delta: Vec3::ZERO,
        }
    }

    /// Consume the per-frame drag delta (resets to zero).
    ///
    /// Call this after `update_gizmo` returns `true` to obtain the
    /// computed delta for the current frame.
    pub fn take_delta(&mut self) -> Vec3 {
        let d = self.delta;
        self.delta = Vec3::ZERO;
        d
    }
}

impl Default for GizmoSystem {
    fn default() -> Self {
        Self::new()
    }
}

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
/// [`apply_gizmo_drag`].
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
        system.dragging = false;
        system.drag_axis = None;
        system.delta = Vec3::ZERO;
        return false;
    }

    // ── Continue active drag ────────────────────────────────────────
    if pointer_down && system.dragging {
        let axis = match system.drag_axis {
            Some(a) => a,
            None => return false,
        };

        let axis_dir = match system.space {
            GizmoSpace::Local => gizmo_rotation * axis.direction(),
            GizmoSpace::Global => axis.direction(),
        };

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

        let snapped = if system.snapping {
            snap_delta(raw, system.snap_value, system.mode, axis)
        } else {
            raw
        };

        system.delta = snapped;
        system.last_pointer = pointer_pos;
        return true;
    }

    // ── Start drag on fresh press ───────────────────────────────────
    if pointer_down && !system.dragging {
        let axes = [GizmoAxis::X, GizmoAxis::Y, GizmoAxis::Z];
        let mut best_dist = HIT_THRESHOLD_PX;
        let mut best_axis: Option<GizmoAxis> = None;

        for &axis in &axes {
            let axis_dir = match system.space {
                GizmoSpace::Local => gizmo_rotation * axis.direction(),
                GizmoSpace::Global => axis.direction(),
            };

            let dist = match system.mode {
                GizmoMode::Translate => screen_distance_to_arrow(
                    gizmo_position,
                    axis_dir,
                    GIZMO_LENGTH,
                    pointer_pos,
                    view_matrix,
                    proj_matrix,
                    viewport_size,
                ),
                GizmoMode::Rotate => screen_distance_to_ring(
                    gizmo_position,
                    axis_dir,
                    GIZMO_RING_RADIUS,
                    pointer_pos,
                    view_matrix,
                    proj_matrix,
                    viewport_size,
                ),
                GizmoMode::Scale => screen_distance_to_cube(
                    gizmo_position,
                    axis_dir,
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
            return true;
        }
    }

    false
}

// ---------------------------------------------------------------------------
// draw_gizmo
// ---------------------------------------------------------------------------

/// Draw the gizmo at the given transform's position.
///
/// Renders axis arrows (translate), rings (rotate), or cubes (scale)
/// depending on the current mode.  The axis currently being dragged is
/// drawn in the highlight colour.
pub fn draw_gizmo(buffer: &mut DebugDrawBuffer, system: &GizmoSystem, transform: &Transform) {
    let position = transform.translation;
    let rotation = if system.space == GizmoSpace::Local {
        transform.rotation
    } else {
        Quat::IDENTITY
    };

    match system.mode {
        GizmoMode::Translate => draw_translate_gizmo(buffer, position, rotation, system),
        GizmoMode::Rotate => draw_rotate_gizmo(buffer, position, rotation, system),
        GizmoMode::Scale => draw_scale_gizmo(buffer, position, rotation, system),
    }
}

/// Draw the translate gizmo (three axis arrows with spheres at tips).
fn draw_translate_gizmo(
    buffer: &mut DebugDrawBuffer,
    position: Vec3,
    rotation: Quat,
    system: &GizmoSystem,
) {
    for axis in &[GizmoAxis::X, GizmoAxis::Y, GizmoAxis::Z] {
        let dir = rotation * axis.direction();
        let color = if system.drag_axis == Some(*axis) {
            COLOR_HIGHLIGHT
        } else {
            axis.color()
        };
        let tip = position + dir * GIZMO_LENGTH;
        buffer.arrow(position, tip, color);
        buffer.sphere_wireframe(tip, 0.06, color);
    }
}

/// Draw the rotate gizmo (three orthogonal rings).
fn draw_rotate_gizmo(
    buffer: &mut DebugDrawBuffer,
    position: Vec3,
    rotation: Quat,
    system: &GizmoSystem,
) {
    for axis in &[GizmoAxis::X, GizmoAxis::Y, GizmoAxis::Z] {
        let normal = rotation * axis.direction();
        let color = if system.drag_axis == Some(*axis) {
            COLOR_HIGHLIGHT
        } else {
            axis.color()
        };
        draw_circle(
            buffer,
            position,
            normal,
            GIZMO_RING_RADIUS,
            color,
            RING_SEGMENTS,
        );
    }
}

/// Draw the scale gizmo (three axis lines with cubes at tips).
fn draw_scale_gizmo(
    buffer: &mut DebugDrawBuffer,
    position: Vec3,
    rotation: Quat,
    system: &GizmoSystem,
) {
    for axis in &[GizmoAxis::X, GizmoAxis::Y, GizmoAxis::Z] {
        let dir = rotation * axis.direction();
        let color = if system.drag_axis == Some(*axis) {
            COLOR_HIGHLIGHT
        } else {
            axis.color()
        };
        let tip = position + dir * GIZMO_LENGTH;
        buffer.line(position, tip, color);
        buffer.box_wireframe(tip, Vec3::splat(GIZMO_CUBE_HALF), color);
    }
}

// ---------------------------------------------------------------------------
// apply_gizmo_drag
// ---------------------------------------------------------------------------

/// Apply a drag `delta` to the entity's [`Transform`] component.
///
/// The interpretation of `delta` depends on the current mode:
/// - `Translate` – world-space translation offset.
/// - `Rotate`    – rotation angle (radians) around the drag axis.
/// - `Scale`     – multiplicative scale factor offset.
///
/// Note: this function bypasses the undo/command system.  For undo support
/// call [`begin_gizmo_session`] before the first drag and
/// [`end_gizmo_session`] after the final drag of a gesture.
pub fn apply_gizmo_drag(system: &GizmoSystem, entity: Entity, world: &mut World, delta: Vec3) {
    let transform = match world.get_mut::<Transform>(entity) {
        Some(t) => t,
        None => return,
    };

    match system.mode {
        GizmoMode::Translate => {
            transform.translation += delta;
        }
        GizmoMode::Rotate => {
            if let Some(axis) = system.drag_axis {
                let dir = if system.space == GizmoSpace::Local {
                    transform.rotation * axis.direction()
                } else {
                    axis.direction()
                };
                let angle = match axis {
                    GizmoAxis::X => delta.x,
                    GizmoAxis::Y => delta.y,
                    GizmoAxis::Z => delta.z,
                };
                let q = Quat::from_axis_angle(dir, angle);
                transform.rotation = (q * transform.rotation).normalize();
            }
        }
        GizmoMode::Scale => {
            transform.scale *= Vec3::ONE + delta;
            transform.scale = transform.scale.max(Vec3::splat(0.001));
        }
    }
}

/// Snapshot an entity's transform so that [`end_gizmo_session`] can produce
/// an undoable command recording the full gesture delta.
///
/// Call this once when the gizmo drag *starts* (e.g. on pointer-down).
pub fn begin_gizmo_session(entity: Entity, world: &World) -> Option<Transform> {
    world.get::<Transform>(entity).cloned()
}

/// Finalise a gizmo drag session by pushing a `SetComponentField` command
/// for each transform field that changed.
///
/// Call this once when the gizmo drag *ends* (e.g. on pointer-up).
///
/// Requires the `snapshot` from [`begin_gizmo_session`] and the current
/// entity transform, plus a mutable reference to the editor's command
/// history and scene so the transform delta is recorded for undo/redo.
pub fn end_gizmo_session(
    entity: Entity,
    world: &World,
    snapshot: &Transform,
    history: &mut crate::commands::CommandHistory,
    scene: &mut engine_scene::Scene,
) {
    let current = match world.get::<Transform>(entity) {
        Some(t) => t,
        None => return,
    };

    let p_id: String = format!("gizmo_ent_{}", entity.index());

    // Only push commands for fields that actually changed.
    if snapshot.translation != current.translation {
        let cmd = crate::commands::SetComponentField::new(
            p_id.clone(),
            "engine.transform".into(),
            "translation".to_string(),
            engine_serialize::Value::Vec3(current.translation.into()),
        );
        let _ = history.push(Box::new(cmd), scene);
    }
    const QUAT_EPS: f32 = 1e-6;
    if (snapshot.rotation - current.rotation).length_squared() > QUAT_EPS {
        let cmd = crate::commands::SetComponentField::new(
            p_id.clone(),
            "engine.transform".into(),
            "rotation".to_string(),
            engine_serialize::Value::Quat(current.rotation.into()),
        );
        let _ = history.push(Box::new(cmd), scene);
    }
    if snapshot.scale != current.scale {
        let cmd = crate::commands::SetComponentField::new(
            p_id.clone(),
            "engine.transform".into(),
            "scale".to_string(),
            engine_serialize::Value::Vec3(current.scale.into()),
        );
        let _ = history.push(Box::new(cmd), scene);
    }
}

// ===========================================================================
// Internal helpers
// ===========================================================================

/// Project a world-space point to screen coordinates.
fn world_to_screen(world_pos: Vec3, view: &Mat4, proj: &Mat4, viewport: Vec2) -> Vec2 {
    let clip = *proj * *view * world_pos.extend(1.0);
    let ndc = clip.truncate() / clip.w;
    Vec2::new(
        (ndc.x * 0.5 + 0.5) * viewport.x,
        (1.0 - (ndc.y * 0.5 + 0.5)) * viewport.y,
    )
}

/// Closest distance from point `p` to the line segment `[a, b]` in 2D.
fn point_to_line_segment_distance(p: Vec2, a: Vec2, b: Vec2) -> f32 {
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
fn screen_distance_to_arrow(
    origin: Vec3,
    dir: Vec3,
    length: f32,
    pointer: Vec2,
    view: &Mat4,
    proj: &Mat4,
    viewport: Vec2,
) -> f32 {
    let p0 = world_to_screen(origin, view, proj, viewport);
    let p1 = world_to_screen(origin + dir * length, view, proj, viewport);
    point_to_line_segment_distance(pointer, p0, p1)
}

/// Screen-space distance from pointer to a rotation ring (approximated as
/// line segments).
fn screen_distance_to_ring(
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
    let mut prev_screen = world_to_screen(center + tangent * radius, view, proj, viewport);
    let first_screen = prev_screen;

    for i in 1..RING_SEGMENTS {
        let a = i as f32 * seg_angle;
        let pos = center + tangent * a.cos() * radius + bitangent * a.sin() * radius;
        let screen = world_to_screen(pos, view, proj, viewport);
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
fn screen_distance_to_cube(
    origin: Vec3,
    dir: Vec3,
    pointer: Vec2,
    view: &Mat4,
    proj: &Mat4,
    viewport: Vec2,
) -> f32 {
    let tip = origin + dir * GIZMO_LENGTH;
    let center_screen = world_to_screen(tip, view, proj, viewport);
    (pointer - center_screen).length()
}

/// Compute the world-space translation delta along `axis_dir` from a
/// pointer movement.
fn compute_translate_delta(
    pointer: Vec2,
    last_pointer: Vec2,
    origin: Vec3,
    axis_dir: Vec3,
    view: &Mat4,
    proj: &Mat4,
    viewport: Vec2,
) -> Vec3 {
    let origin_screen = world_to_screen(origin, view, proj, viewport);
    let tip_screen = world_to_screen(origin + axis_dir, view, proj, viewport);
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
/// axis should be used (the caller or [`apply_gizmo_drag`] selects the
/// correct component).
fn compute_rotate_delta(
    pointer: Vec2,
    last_pointer: Vec2,
    center: Vec3,
    view: &Mat4,
    proj: &Mat4,
    viewport: Vec2,
) -> Vec3 {
    let center_screen = world_to_screen(center, view, proj, viewport);
    let angle_curr = (pointer - center_screen)
        .y
        .atan2((pointer - center_screen).x);
    let angle_last = (last_pointer - center_screen)
        .y
        .atan2((last_pointer - center_screen).x);
    Vec3::new(angle_curr - angle_last, 0.0, 0.0)
}

/// Compute the scale delta from a pointer movement (same as translate,
/// but the caller interprets the result as a scale factor).
fn compute_scale_delta(
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

/// Snap a delta vector based on the current snap settings.
fn snap_delta(delta: Vec3, snap: f32, mode: GizmoMode, axis: GizmoAxis) -> Vec3 {
    if snap <= 0.0 {
        return delta;
    }
    let snap_val = match mode {
        GizmoMode::Rotate => snap.to_radians(),
        _ => snap,
    };
    let snap_fn = |v: f32| (v / snap_val).round() * snap_val;
    match axis {
        GizmoAxis::X => Vec3::new(snap_fn(delta.x), 0.0, 0.0),
        GizmoAxis::Y => Vec3::new(0.0, snap_fn(delta.y), 0.0),
        GizmoAxis::Z => Vec3::new(0.0, 0.0, snap_fn(delta.z)),
    }
}

/// Draw a wireframe circle (ring) using line segments.
fn draw_circle(
    buffer: &mut DebugDrawBuffer,
    center: Vec3,
    normal: Vec3,
    radius: f32,
    color: [f32; 4],
    segments: u32,
) {
    let tangent = if normal.x.abs() > 0.9 {
        Vec3::Y.cross(normal).normalize()
    } else {
        Vec3::X.cross(normal).normalize()
    };
    let bitangent = normal.cross(tangent).normalize();
    let seg = std::f32::consts::PI * 2.0 / segments as f32;

    let mut prev = center + tangent * radius;
    for i in 1..=segments {
        let a = i as f32 * seg;
        let curr = center + tangent * a.cos() * radius + bitangent * a.sin() * radius;
        buffer.line(prev, curr, color);
        prev = curr;
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use engine_scene::components::Transform;

    // ── GizmoSystem construction and field access ───────────────────

    #[test]
    fn gizmo_new_defaults() {
        let g = GizmoSystem::new();
        assert_eq!(g.mode, GizmoMode::Translate);
        assert_eq!(g.space, GizmoSpace::Global);
        assert!(!g.snapping);
        assert_eq!(g.snap_value, 0.5);
        assert!(g.selected_entity.is_none());
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

    // ── Draw functions (must not panic with empty inputs) ──────────

    #[test]
    fn draw_gizmo_no_crash_empty() {
        let mut buf = DebugDrawBuffer::new();
        let g = GizmoSystem::new();
        let t = Transform::default();
        draw_gizmo(&mut buf, &g, &t);
    }

    #[test]
    fn draw_gizmo_all_modes_no_crash() {
        for mode in &[GizmoMode::Translate, GizmoMode::Rotate, GizmoMode::Scale] {
            let mut buf = DebugDrawBuffer::new();
            let mut g = GizmoSystem::new();
            g.mode = *mode;
            let t = Transform {
                translation: Vec3::new(1.0, 2.0, 3.0),
                rotation: Quat::IDENTITY,
                scale: Vec3::ONE,
                parent: None,
            };
            draw_gizmo(&mut buf, &g, &t);
        }
    }

    #[test]
    fn draw_translate_gizmo_produces_items() {
        let mut buf = DebugDrawBuffer::new();
        let g = GizmoSystem::new(); // default = Translate
        let t = Transform::default();
        draw_gizmo(&mut buf, &g, &t);
        // Should have arrows (shapes) and tip spheres (shapes)
        assert!(buf.shapes.len() >= 3);
    }

    #[test]
    fn draw_rotate_gizmo_produces_lines() {
        let mut buf = DebugDrawBuffer::new();
        let mut g = GizmoSystem::new();
        g.mode = GizmoMode::Rotate;
        let t = Transform::default();
        draw_gizmo(&mut buf, &g, &t);
        // Rings produce many line segments
        assert!(!buf.lines.is_empty());
    }

    #[test]
    fn draw_gizmo_with_drag_axis_highlights() {
        let mut buf = DebugDrawBuffer::new();
        let mut g = GizmoSystem::new();
        g.drag_axis = Some(GizmoAxis::Z);
        let t = Transform::default();
        draw_gizmo(&mut buf, &g, &t);
        assert!(!buf.shapes.is_empty());
    }

    // ── apply_gizmo_drag ────────────────────────────────────────────

    #[test]
    fn apply_translate_drag() {
        let mut world = engine_scene::World::new();
        let entity = world.create_entity();
        world.add_component(entity, Transform::default());

        let g = GizmoSystem {
            mode: GizmoMode::Translate,
            dragging: true,
            drag_axis: Some(GizmoAxis::X),
            ..GizmoSystem::new()
        };

        apply_gizmo_drag(&g, entity, &mut world, Vec3::new(5.0, 0.0, 0.0));
        let t = world.get::<Transform>(entity).unwrap();
        assert!((t.translation.x - 5.0).abs() < 0.001);
    }

    #[test]
    fn apply_rotate_drag() {
        let mut world = engine_scene::World::new();
        let entity = world.create_entity();
        world.add_component(entity, Transform::default());

        let g = GizmoSystem {
            mode: GizmoMode::Rotate,
            dragging: true,
            drag_axis: Some(GizmoAxis::Y),
            ..GizmoSystem::new()
        };

        apply_gizmo_drag(
            &g,
            entity,
            &mut world,
            Vec3::new(0.0, std::f32::consts::FRAC_PI_2, 0.0),
        );
        let t = world.get::<Transform>(entity).unwrap();
        let (_axis, angle) = t.rotation.to_axis_angle();
        assert!((angle - std::f32::consts::FRAC_PI_2).abs() < 0.001);
    }

    #[test]
    fn apply_scale_drag() {
        let mut world = engine_scene::World::new();
        let entity = world.create_entity();
        world.add_component(entity, Transform::default());

        let g = GizmoSystem {
            mode: GizmoMode::Scale,
            dragging: true,
            drag_axis: Some(GizmoAxis::X),
            ..GizmoSystem::new()
        };

        apply_gizmo_drag(&g, entity, &mut world, Vec3::new(0.5, 0.0, 0.0));
        let t = world.get::<Transform>(entity).unwrap();
        assert!((t.scale.x - 1.5).abs() < 0.001);
    }

    #[test]
    fn apply_drag_no_entity_transform_no_crash() {
        let mut world = engine_scene::World::new();
        let entity = world.create_entity();
        // Entity has no Transform component
        let g = GizmoSystem::new();
        apply_gizmo_drag(&g, entity, &mut world, Vec3::ZERO);
        // Should not panic
    }

    #[test]
    fn apply_drag_stale_entity_no_crash() {
        let mut world = engine_scene::World::new();
        let entity = world.create_entity();
        world.destroy_entity(entity);
        let g = GizmoSystem::new();
        apply_gizmo_drag(&g, entity, &mut world, Vec3::ZERO);
        // Should not panic
    }

    // ── update_gizmo (basic state machine) ──────────────────────────

    #[test]
    fn update_gizmo_pointer_up_ends_drag() {
        let mut g = GizmoSystem::new();
        g.dragging = true;
        g.drag_axis = Some(GizmoAxis::X);
        g.delta = Vec3::new(1.0, 0.0, 0.0);

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
    }

    // ── snap_delta ──────────────────────────────────────────────────

    #[test]
    fn snap_translate_delta() {
        // 0.63 snaps to 0.5 at snap=0.5
        let d = snap_delta(
            Vec3::new(0.63, 0.0, 0.0),
            0.5,
            GizmoMode::Translate,
            GizmoAxis::X,
        );
        assert!((d.x - 0.5).abs() < 0.001);
    }

    #[test]
    fn snap_rotate_delta() {
        // 30 degrees = 0.5236 rad; 0.53 should snap to that
        let d = snap_delta(
            Vec3::new(0.0, 0.53, 0.0),
            30.0,
            GizmoMode::Rotate,
            GizmoAxis::Y,
        );
        let expected = 30.0_f32.to_radians();
        assert!((d.y - expected).abs() < 0.01);
    }

    #[test]
    fn snap_zero_snap_value_passthrough() {
        let d = snap_delta(
            Vec3::new(0.37, 0.0, 0.0),
            0.0,
            GizmoMode::Translate,
            GizmoAxis::X,
        );
        assert!((d.x - 0.37).abs() < 0.001);
    }

    // ── Internal helpers ────────────────────────────────────────────

    #[test]
    fn world_to_screen_identity() {
        // With identity view/proj and viewport 2x2, origin should map to center
        let screen = world_to_screen(
            Vec3::ZERO,
            &Mat4::IDENTITY,
            &Mat4::IDENTITY,
            Vec2::new(2.0, 2.0),
        );
        // NDC = (0,0,0,1) → screen (1, 1)
        assert!((screen.x - 1.0).abs() < 0.001);
        assert!((screen.y - 1.0).abs() < 0.001);
    }

    #[test]
    fn point_to_line_segment_distance_on_endpoint() {
        let d = point_to_line_segment_distance(
            Vec2::new(0.0, 0.0),
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
        );
        assert!(d < 0.001);
    }

    #[test]
    fn point_to_line_segment_distance_perpendicular() {
        let d = point_to_line_segment_distance(
            Vec2::new(0.5, 1.0),
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
        );
        assert!((d - 1.0).abs() < 0.001);
    }

    #[test]
    fn draw_circle_no_crash() {
        let mut buf = DebugDrawBuffer::new();
        draw_circle(&mut buf, Vec3::ZERO, Vec3::Y, 1.0, [1.0, 0.0, 0.0, 1.0], 8);
        assert!(!buf.lines.is_empty());
    }
}
