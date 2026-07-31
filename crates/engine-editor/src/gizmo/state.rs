use super::*;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// X-axis colour — red.
const COLOR_X: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
/// Y-axis colour — green.
const COLOR_Y: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
/// Z-axis colour — blue.
const COLOR_Z: [f32; 4] = [0.0, 0.0, 1.0, 1.0];
/// Length of translate arrow and scale axis lines in world units.
pub(crate) const GIZMO_LENGTH: f32 = 1.0;
/// Radius of rotate rings.
pub(crate) const GIZMO_RING_RADIUS: f32 = 0.8;
/// Desired screen-space length of a translate/scale axis.
pub(super) const GIZMO_TARGET_LENGTH_PX: f32 = 88.0;
/// Number of line segments used to approximate rotation rings.
pub(crate) const RING_SEGMENTS: u32 = 32;
/// Screen-space hit-test threshold in pixels.
pub(super) const HIT_THRESHOLD_PX: f32 = 12.0;

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
    pub(super) last_pointer: Vec2,
    /// Per-frame delta accumulated by `update_gizmo`, consumed by caller
    /// via `take_delta`.
    pub(super) delta: Vec3,
    /// Unsnapped axis amount accumulated over the complete pointer gesture.
    pub(super) raw_drag_total: f32,
    /// Total snapped axis amount already emitted for this gesture.
    pub(super) applied_drag_total: f32,
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
