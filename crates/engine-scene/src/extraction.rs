use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

use engine_renderer::{
    AxisAlignedBox, BlendMode, ClearFlags, DebugPrimitive, DebugPrimitiveKind, ExtractionStats,
    LightItem, Rect, RenderFrameInput, RenderView, RenderableItem, ShadowMode, ViewCompose,
};
use engine_serialize::{AssetId, Diagnostic, DiagnosticSeverity, PersistentId};

use crate::components;
use crate::scene::ECS_SCENE_CONTRACT;
use crate::World;

mod frame;
mod spatial;
mod viewport;

pub use frame::{
    extract_renderer_input_from_world, extract_renderer_input_from_world_with_viewport,
};
pub use spatial::{
    aabb_in_frustum, active_camera_view, active_camera_world_position,
    camera_relative_render_origin, entity_world_position, entity_world_transform,
    extract_frustum_planes, render_layer_bit,
};
pub use viewport::{ActiveCameraView, RenderViewportContext};

#[cfg(test)]
use spatial::{map_clear_flags, translate_debug_primitives};

// Tests

#[cfg(test)]
mod tests {
    include!("extraction/tests/common.rs");
    include!("extraction/tests/frustum_camera.rs");
    include!("extraction/tests/drawables_hierarchy.rs");
    include!("extraction/tests/transform_precision.rs");
    include!("extraction/tests/camera_relative.rs");
    include!("extraction/tests/debug_origin.rs");
}

#[cfg(test)]
#[test]
fn production_facade_uses_real_modules() {
    let source = include_str!("extraction.rs");
    let production = source.split("// Tests").next().expect("production facade");
    assert!(!production.contains(concat!("include", "!(")));
    for module in ["frame", "spatial", "viewport"] {
        assert!(production.contains(&format!("mod {module};")));
    }
}
