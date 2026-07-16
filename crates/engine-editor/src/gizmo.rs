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
use engine_scene::{Entity, Scene, World};
use engine_serialize::{PersistentId, Value};

use crate::commands::Command;
use crate::{EditorError, EditorScene};

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
pub(crate) const GIZMO_LENGTH: f32 = 1.0;
/// Radius of rotate rings.
pub(crate) const GIZMO_RING_RADIUS: f32 = 0.8;
/// Desired screen-space length of a translate/scale axis.
const GIZMO_TARGET_LENGTH_PX: f32 = 88.0;
/// Half-extent of scale cubes.
const GIZMO_CUBE_HALF: f32 = 0.05;
/// Number of line segments used to approximate rotation rings.
pub(crate) const RING_SEGMENTS: u32 = 32;
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
    /// Unsnapped axis amount accumulated over the complete pointer gesture.
    raw_drag_total: f32,
    /// Total snapped axis amount already emitted for this gesture.
    applied_drag_total: f32,
}

impl GizmoSystem {
    /// Create a new gizmo system with default settings.
    pub fn new() -> Self {
        Self {
            mode: GizmoMode::Translate,
            space: GizmoSpace::Global,
            snapping: false,
            snap_value: 0.5,
            dragging: false,
            drag_axis: None,
            last_pointer: Vec2::ZERO,
            delta: Vec3::ZERO,
            raw_drag_total: 0.0,
            applied_drag_total: 0.0,
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

    /// Cancel any pointer gesture and clear all transient drag accumulation.
    ///
    /// Hosts should call this on focus loss, viewport resize, Play-mode
    /// transitions, or when selection becomes unavailable. Mode, space,
    /// and snapping settings remain unchanged. Entity selection belongs to
    /// `EditorScene` and is intentionally not cached as an unstable ECS index.
    pub fn cancel_drag(&mut self) {
        self.dragging = false;
        self.drag_axis = None;
        self.last_pointer = Vec2::ZERO;
        self.delta = Vec3::ZERO;
        self.raw_drag_total = 0.0;
        self.applied_drag_total = 0.0;
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
    let rotation = if system.mode == GizmoMode::Scale || system.space == GizmoSpace::Local {
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
// Scene-level undoable Transform gestures
// ---------------------------------------------------------------------------

const TRANSFORM_COMPONENT_TYPE: &str = "engine.transform";

/// Active scene-level gizmo gesture. The persistent ID is captured when the
/// drag begins, so later selection changes cannot redirect the edit.
#[derive(Clone, Debug)]
pub(crate) struct SceneGizmoDrag {
    entity_id: PersistentId,
    original: TransformFieldSnapshot,
}

/// Exact serialized Transform fields. `None` is significant: cancelling or
/// undoing a gesture restores an originally omitted default field by removing
/// it again, instead of materialising a synthetic value.
#[derive(Clone, Debug, PartialEq)]
struct TransformFieldSnapshot {
    translation: Option<Value>,
    rotation: Option<Value>,
    scale: Option<Value>,
}

#[derive(Clone, Copy, Debug)]
struct EffectiveTransform {
    translation: Vec3,
    rotation: Quat,
    scale: Vec3,
}

#[derive(Clone, Copy, Debug)]
struct ResolvedWorldTransform {
    matrix: Mat4,
    rotation: Quat,
}

impl ResolvedWorldTransform {
    const IDENTITY: Self = Self {
        matrix: Mat4::IDENTITY,
        rotation: Quat::IDENTITY,
    };
}

impl TransformFieldSnapshot {
    fn from_transform(transform: &Transform) -> Self {
        Self {
            translation: Some(Value::Vec3(transform.translation.to_array())),
            rotation: Some(Value::Quat(transform.rotation.to_array())),
            scale: Some(Value::Vec3(transform.scale.to_array())),
        }
    }

    fn effective(&self) -> Option<EffectiveTransform> {
        let translation = match &self.translation {
            Some(Value::Vec3(value)) => Vec3::from_array(*value),
            None => Vec3::ZERO,
            Some(_) => return None,
        };
        let rotation = match &self.rotation {
            Some(Value::Quat(value)) => Quat::from_array(*value),
            None => Quat::IDENTITY,
            Some(_) => return None,
        };
        let scale = match &self.scale {
            Some(Value::Vec3(value)) => Vec3::from_array(*value),
            None => Vec3::ONE,
            Some(_) => return None,
        };
        if !translation.is_finite()
            || !rotation.is_finite()
            || rotation.length_squared() <= f32::EPSILON
            || !scale.is_finite()
        {
            return None;
        }
        Some(EffectiveTransform {
            translation,
            rotation,
            scale,
        })
    }
}

/// One command represents one complete pointer gesture, even when translate,
/// rotate, and scale fields were previewed repeatedly during the drag.
struct SetTransformGesture {
    entity_id: PersistentId,
    before: TransformFieldSnapshot,
    after: TransformFieldSnapshot,
}

impl Command for SetTransformGesture {
    fn name(&self) -> &str {
        "Transform Gizmo Drag"
    }

    fn execute(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        apply_transform_snapshot(scene, &self.entity_id, &self.after)
    }

    fn undo(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        apply_transform_snapshot(scene, &self.entity_id, &self.before)
    }
}

impl EditorScene {
    /// Read the selected entity's effective local Transform for gizmo drawing.
    ///
    /// Omitted serialized fields use the same defaults as the scene loader:
    /// zero translation, identity rotation, and unit scale. A stale selection,
    /// missing Transform, malformed field type, non-finite value, or zero
    /// quaternion returns `None`. The returned values are the serialized local
    /// TRS components; hierarchy resolution is performed internally when a
    /// world-space gizmo delta is previewed. `parent` is therefore `None`.
    pub fn selected_transform_for_gizmo(&self) -> Option<Transform> {
        let entity_id = self.selected_entity.as_ref()?;
        let transform = capture_transform_snapshot(&self.scene, entity_id)
            .ok()?
            .effective()?;
        Some(Transform {
            translation: transform.translation,
            rotation: transform.rotation,
            scale: transform.scale,
            parent: None,
        })
    }

    /// Begin a Transform gizmo gesture for the currently selected entity.
    ///
    /// Returns `false` without mutating state when there is no selection, the
    /// persistent ID is stale, the entity has no Transform, the serialized
    /// Transform is malformed, or another gesture is already active.
    pub fn begin_transform_gizmo_drag(&mut self) -> bool {
        if self.gizmo_drag.is_some() {
            return false;
        }
        let Some(entity_id) = self.selected_entity.clone() else {
            return false;
        };
        let Ok(original) = capture_transform_snapshot(&self.scene, &entity_id) else {
            return false;
        };
        if original.effective().is_none() {
            return false;
        }
        self.gizmo_drag = Some(SceneGizmoDrag {
            entity_id,
            original,
        });
        true
    }

    /// Apply one incremental gizmo delta as an uncommitted scene preview.
    ///
    /// Preview edits do not dirty command history. Call
    /// [`commit_transform_gizmo_drag`](Self::commit_transform_gizmo_drag) once
    /// on pointer release, or
    /// [`cancel_transform_gizmo_drag`](Self::cancel_transform_gizmo_drag) to
    /// restore the exact pre-drag values.
    pub fn preview_transform_gizmo_drag(&mut self, system: &GizmoSystem, delta: Vec3) -> bool {
        if !delta.is_finite() {
            return false;
        }
        let Some(entity_id) = self
            .gizmo_drag
            .as_ref()
            .map(|session| session.entity_id.clone())
        else {
            return false;
        };
        let Ok(snapshot) = capture_transform_snapshot(&self.scene, &entity_id) else {
            return false;
        };
        let Some(mut transform) = snapshot.effective() else {
            return false;
        };

        let field = match system.mode {
            GizmoMode::Translate => {
                let Some(parent_world) = resolve_parent_world_transform(&self.scene, &entity_id)
                else {
                    return false;
                };
                let determinant = parent_world.matrix.determinant();
                if !determinant.is_finite() || determinant.abs() <= f32::EPSILON {
                    return false;
                }
                let inverse_parent = parent_world.matrix.inverse();
                let local_delta = inverse_parent.transform_vector3(delta);
                if !inverse_parent.is_finite() || !local_delta.is_finite() {
                    return false;
                }
                transform.translation += local_delta;
                ("translation", Value::Vec3(transform.translation.to_array()))
            }
            GizmoMode::Rotate => {
                let Some(axis) = system.drag_axis else {
                    return false;
                };
                let Some(parent_world) = resolve_parent_world_transform(&self.scene, &entity_id)
                else {
                    return false;
                };
                let angle = match axis {
                    GizmoAxis::X => delta.x,
                    GizmoAxis::Y => delta.y,
                    GizmoAxis::Z => delta.z,
                };
                let local_rotation = transform.rotation.normalize();
                let world_rotation = (parent_world.rotation * local_rotation).normalize();
                let axis_rotation = Quat::from_axis_angle(axis.direction(), angle);
                let next_world_rotation = match system.space {
                    GizmoSpace::Local => world_rotation * axis_rotation,
                    GizmoSpace::Global => axis_rotation * world_rotation,
                }
                .normalize();
                transform.rotation =
                    (parent_world.rotation.inverse() * next_world_rotation).normalize();
                ("rotation", Value::Quat(transform.rotation.to_array()))
            }
            GizmoMode::Scale => {
                // Scale handles always represent the entity's local axes and
                // write its local scale components. This remains well-defined
                // under rotated/non-uniform parents, unlike a world-aligned
                // scale that would generally require storing shear.
                transform.scale *= Vec3::ONE + delta;
                transform.scale = transform.scale.max(Vec3::splat(0.001));
                ("scale", Value::Vec3(transform.scale.to_array()))
            }
        };

        set_transform_field(&mut self.scene, &entity_id, field.0, field.1).is_ok()
    }

    /// Commit the active preview as exactly one undoable command.
    ///
    /// Returns `Ok(false)` when no gesture is active or the Transform did not
    /// change. The command uses the real persistent entity ID captured at drag
    /// start, never a fabricated ECS index.
    pub fn commit_transform_gizmo_drag(&mut self) -> Result<bool, EditorError> {
        let Some(session) = self.gizmo_drag.take() else {
            return Ok(false);
        };
        let current = capture_transform_snapshot(&self.scene, &session.entity_id)?;
        if current == session.original {
            return Ok(false);
        }
        self.history.push(
            Box::new(SetTransformGesture {
                entity_id: session.entity_id,
                before: session.original,
                after: current,
            }),
            &mut self.scene,
        )?;
        Ok(true)
    }

    /// Cancel the active gesture and restore the exact pre-drag Transform.
    ///
    /// Cancellation never creates an undo entry or marks history dirty.
    pub fn cancel_transform_gizmo_drag(&mut self) -> bool {
        let Some(session) = self.gizmo_drag.take() else {
            return false;
        };
        apply_transform_snapshot(&mut self.scene, &session.entity_id, &session.original).is_ok()
    }

    /// Whether a scene-level Transform gesture is currently active.
    pub fn is_transform_gizmo_drag_active(&self) -> bool {
        self.gizmo_drag.is_some()
    }

    /// Persistent entity captured by the active Transform gesture.
    ///
    /// Hosts can compare this with their current selection before syncing a
    /// preview into a runtime world. A selection change never retargets an
    /// already-active gesture.
    pub fn active_transform_gizmo_entity(&self) -> Option<&PersistentId> {
        self.gizmo_drag.as_ref().map(|session| &session.entity_id)
    }
}

fn resolve_parent_world_transform(
    scene: &Scene,
    entity_id: &PersistentId,
) -> Option<ResolvedWorldTransform> {
    let entity = scene
        .entities
        .iter()
        .find(|entity| entity.persistent_id == *entity_id)?;
    let Some(parent_id) = entity.parent.as_ref() else {
        return Some(ResolvedWorldTransform::IDENTITY);
    };
    resolve_entity_world_transform(scene, parent_id, &mut Vec::new())
}

fn resolve_entity_world_transform(
    scene: &Scene,
    entity_id: &PersistentId,
    visiting: &mut Vec<PersistentId>,
) -> Option<ResolvedWorldTransform> {
    if visiting.iter().any(|visited| visited == entity_id) {
        return None;
    }
    let entity = scene
        .entities
        .iter()
        .find(|entity| entity.persistent_id == *entity_id)?;

    // Match runtime transform extraction: an entity without Transform is an
    // identity root, even if malformed input gave it a parent of its own.
    let Some(component) = entity.components.get(TRANSFORM_COMPONENT_TYPE) else {
        return Some(ResolvedWorldTransform::IDENTITY);
    };
    let local = TransformFieldSnapshot {
        translation: component.fields.get("translation").cloned(),
        rotation: component.fields.get("rotation").cloned(),
        scale: component.fields.get("scale").cloned(),
    }
    .effective()?;
    let local_rotation = local.rotation.normalize();
    let local_matrix =
        Mat4::from_scale_rotation_translation(local.scale, local_rotation, local.translation);
    if !local_matrix.is_finite() {
        return None;
    }

    visiting.push(entity_id.clone());
    let parent_world = match entity.parent.as_ref() {
        Some(parent_id) => resolve_entity_world_transform(scene, parent_id, visiting),
        None => Some(ResolvedWorldTransform::IDENTITY),
    };
    visiting.pop();
    let parent_world = parent_world?;
    let matrix = parent_world.matrix * local_matrix;
    let rotation = (parent_world.rotation * local_rotation).normalize();
    (matrix.is_finite() && rotation.is_finite())
        .then_some(ResolvedWorldTransform { matrix, rotation })
}

fn capture_transform_snapshot(
    scene: &Scene,
    entity_id: &PersistentId,
) -> Result<TransformFieldSnapshot, EditorError> {
    let entity = scene
        .entities
        .iter()
        .find(|entity| entity.persistent_id == *entity_id)
        .ok_or_else(|| EditorError::EntityNotFound(entity_id.clone()))?;
    let component = entity
        .components
        .get(TRANSFORM_COMPONENT_TYPE)
        .ok_or_else(|| EditorError::ComponentNotFound(TRANSFORM_COMPONENT_TYPE.into()))?;
    Ok(TransformFieldSnapshot {
        translation: component.fields.get("translation").cloned(),
        rotation: component.fields.get("rotation").cloned(),
        scale: component.fields.get("scale").cloned(),
    })
}

fn apply_transform_snapshot(
    scene: &mut Scene,
    entity_id: &PersistentId,
    snapshot: &TransformFieldSnapshot,
) -> Result<(), EditorError> {
    let entity = crate::commands::find_entity_mut(scene, entity_id)?;
    let component = entity
        .components
        .get_mut(TRANSFORM_COMPONENT_TYPE)
        .ok_or_else(|| EditorError::ComponentNotFound(TRANSFORM_COMPONENT_TYPE.into()))?;
    set_optional_field(component, "translation", snapshot.translation.clone());
    set_optional_field(component, "rotation", snapshot.rotation.clone());
    set_optional_field(component, "scale", snapshot.scale.clone());
    Ok(())
}

fn set_optional_field(
    component: &mut engine_scene::ComponentRecord,
    field: &str,
    value: Option<Value>,
) {
    if let Some(value) = value {
        component.fields.insert(field.to_string(), value);
    } else {
        component.fields.remove(field);
    }
}

fn set_transform_field(
    scene: &mut Scene,
    entity_id: &PersistentId,
    field: &str,
    value: Value,
) -> Result<(), EditorError> {
    let entity = crate::commands::find_entity_mut(scene, entity_id)?;
    let component = entity
        .components
        .get_mut(TRANSFORM_COMPONENT_TYPE)
        .ok_or_else(|| EditorError::ComponentNotFound(TRANSFORM_COMPONENT_TYPE.into()))?;
    component.fields.insert(field.to_string(), value);
    Ok(())
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
/// Note: this low-level World function bypasses scene command history. Editor
/// integrations should prefer [`EditorScene::begin_transform_gizmo_drag`],
/// [`EditorScene::preview_transform_gizmo_drag`], and
/// [`EditorScene::commit_transform_gizmo_drag`].
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

/// Finalise a low-level World gizmo drag as one scene-history command.
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
    let Some(entity_id) = world.persistent_id(entity).map(str::to_owned) else {
        return;
    };
    let before = TransformFieldSnapshot::from_transform(snapshot);
    let after = TransformFieldSnapshot::from_transform(current);
    if before == after {
        return;
    }
    let _ = history.push(
        Box::new(SetTransformGesture {
            entity_id,
            before,
            after,
        }),
        scene,
    );
}

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
fn screen_distance_to_cube(
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
fn compute_translate_delta(
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
/// axis should be used (the caller or [`apply_gizmo_drag`] selects the
/// correct component).
fn compute_rotate_delta(
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

/// Snap a logical axis amount based on the current mode's units.
fn snap_amount(amount: f32, snap: f32, mode: GizmoMode) -> f32 {
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
fn accumulate_gesture_amount(system: &mut GizmoSystem, raw_amount: f32) -> f32 {
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
    use engine_scene::ComponentRecord;
    use engine_serialize::SchemaVersion;

    fn install_transform(scene: &mut Scene, entity_id: &str, fields: &[(&str, Value)]) {
        let entity = scene
            .entities
            .iter_mut()
            .find(|entity| entity.persistent_id == entity_id)
            .unwrap();
        entity.components.insert(
            TRANSFORM_COMPONENT_TYPE.into(),
            ComponentRecord {
                schema_version: SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: fields
                    .iter()
                    .map(|(name, value)| ((*name).to_string(), value.clone()))
                    .collect(),
            },
        );
    }

    fn editor_scene_with_transform(fields: &[(&str, Value)]) -> EditorScene {
        let mut scene = engine_scene::sample_scene();
        install_transform(&mut scene, "cube-01", fields);
        let mut editor = EditorScene::new(scene);
        editor.selected_entity = Some("cube-01".into());
        editor
    }

    fn transform_field<'a>(editor: &'a EditorScene, field: &str) -> Option<&'a Value> {
        editor
            .scene
            .entities
            .iter()
            .find(|entity| entity.persistent_id == "cube-01")
            .and_then(|entity| entity.components.get(TRANSFORM_COMPONENT_TYPE))
            .and_then(|component| component.fields.get(field))
    }

    // ── GizmoSystem construction and field access ───────────────────

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
        let mut editor =
            editor_scene_with_transform(&[("translation", Value::Vec3([2.0, 3.0, 4.0]))]);
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
            editor.history.done.last().map(|command| command.name()),
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
        let mut editor =
            editor_scene_with_transform(&[("translation", Value::Vec3([1.0, 0.0, 0.0]))]);
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
        let mut editor =
            editor_scene_with_transform(&[("translation", Value::Vec3([1.0, 0.0, 0.0]))]);
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
        let mut editor =
            editor_scene_with_transform(&[("translation", Value::Vec3([3.0, 4.0, 5.0]))]);
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
        assert!(editor.preview_transform_gizmo_drag(
            &gizmo,
            Vec3::new(0.0, 0.0, std::f32::consts::FRAC_PI_2),
        ));
        assert!(editor.commit_transform_gizmo_drag().unwrap());
        let Some(Value::Quat(rotation)) = transform_field(&editor, "rotation") else {
            panic!("rotation preview was not stored");
        };
        let (_, angle) = Quat::from_array(*rotation).to_axis_angle();
        assert!((angle - std::f32::consts::FRAC_PI_2).abs() < 0.001);
    }

    #[test]
    fn editor_undo_during_preview_cancels_instead_of_touching_history() {
        let mut editor =
            editor_scene_with_transform(&[("translation", Value::Vec3([1.0, 0.0, 0.0]))]);
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

    #[test]
    fn project_world_to_screen_identity() {
        // With identity view/proj and viewport 2x2, origin should map to center
        let screen = project_world_to_screen(
            Vec3::ZERO,
            &Mat4::IDENTITY,
            &Mat4::IDENTITY,
            Vec2::new(2.0, 2.0),
        )
        .unwrap();
        // NDC = (0,0,0,1) → screen (1, 1)
        assert!((screen.x - 1.0).abs() < 0.001);
        assert!((screen.y - 1.0).abs() < 0.001);
    }

    #[test]
    fn projection_and_hit_testing_reject_points_behind_camera() {
        let projection = Mat4::perspective_lh(60.0_f32.to_radians(), 1.0, 0.1, 100.0);
        let viewport = Vec2::new(800.0, 800.0);
        assert!(project_world_to_screen(
            Vec3::new(0.0, 0.0, -5.0),
            &Mat4::IDENTITY,
            &projection,
            viewport,
        )
        .is_none());
        assert_eq!(
            screen_distance_to_arrow(
                Vec3::new(0.0, 0.0, -5.0),
                Vec3::X,
                1.0,
                Vec2::new(400.0, 400.0),
                &Mat4::IDENTITY,
                &projection,
                viewport,
            ),
            f32::MAX
        );
    }

    #[test]
    fn gizmo_screen_scale_and_hit_target_are_depth_independent() {
        let projection = Mat4::perspective_lh(60.0_f32.to_radians(), 1.0, 0.1, 100.0);
        let viewport = Vec2::splat(800.0);
        let near = Vec3::new(0.0, 0.0, 5.0);
        let far = Vec3::new(0.0, 0.0, 50.0);
        let near_scale = gizmo_world_scale(near, &Mat4::IDENTITY, &projection, viewport).unwrap();
        let far_scale = gizmo_world_scale(far, &Mat4::IDENTITY, &projection, viewport).unwrap();
        assert!((far_scale / near_scale - 10.0).abs() < 0.01);

        for position in [near, far] {
            let center =
                project_world_to_screen(position, &Mat4::IDENTITY, &projection, viewport).unwrap();
            let mut gizmo = GizmoSystem::new();
            assert!(update_gizmo(
                &mut gizmo,
                position,
                Quat::IDENTITY,
                &Mat4::IDENTITY,
                &projection,
                viewport,
                center + Vec2::new(GIZMO_TARGET_LENGTH_PX * 0.5, 0.0),
                true,
            ));
            assert_eq!(gizmo.drag_axis, Some(GizmoAxis::X));
        }
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
