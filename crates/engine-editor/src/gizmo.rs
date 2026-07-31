//! Editor gizmo system for 3D viewport manipulation.
//!
//! Provides translate, rotate, and scale gizmos with axis snapping,
//! local/global space support, and screen-space hit testing.
//!
//! Viewport integrations feed interaction deltas into
//! [`EditorScene::preview_transform_gizmo_drag`] and commit the complete
//! gesture through editor command history.

use glam::{Mat4, Quat, Vec2, Vec3};

use engine_scene::components::Transform;
use engine_scene::Scene;
use engine_serialize::{PersistentId, Value};

use crate::commands::Command;
use crate::{EditorError, EditorScene};

mod interaction;
mod math;
mod scene_gesture;
mod state;

pub(crate) use interaction::gizmo_axis_direction;
pub use interaction::update_gizmo;
pub(crate) use math::{gizmo_world_scale, project_world_to_screen};
pub(crate) use scene_gesture::SceneGizmoDrag;
pub use state::{GizmoAxis, GizmoMode, GizmoSpace, GizmoSystem};
pub(crate) use state::{GIZMO_LENGTH, GIZMO_RING_RADIUS, RING_SEGMENTS};

#[cfg(test)]
use math::{
    accumulate_gesture_amount, compute_rotate_delta, point_to_line_segment_distance,
    screen_distance_to_arrow, snap_amount,
};
#[cfg(test)]
use scene_gesture::{set_transform_field, TRANSFORM_COMPONENT_TYPE};
#[cfg(test)]
use state::GIZMO_TARGET_LENGTH_PX;

#[cfg(test)]
#[path = "gizmo/tests.rs"]
mod tests;

#[cfg(test)]
#[test]
fn production_facade_uses_real_modules() {
    let source = include_str!("gizmo.rs");
    assert!(!source.contains(concat!("include", "!(")));
    for module in ["interaction", "math", "scene_gesture", "state"] {
        assert!(source.contains(&format!("mod {module};")));
    }
}
