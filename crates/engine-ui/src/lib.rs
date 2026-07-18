//! UI system for engine-renderer.
//!
//! Produces [`engine_renderer::UiBatch`] data consumed by the rendering pipeline.
//! Elements are positioned via anchor-based [`Layout`]s relative to the canvas
//! or parent element, then resolved to pixel coordinates by [`Canvas::layout_all`].
//!
//! Coordinate system: +X right, +Y down, origin at top-left of the canvas.

#![forbid(unsafe_code)]

// ---------------------------------------------------------------------------
// Module declarations
// ---------------------------------------------------------------------------

mod batch;
mod canvas;
mod color;
mod font;
mod input;
mod layout;
mod render;
mod types;

// ---------------------------------------------------------------------------
// Re-exports
// ---------------------------------------------------------------------------

pub use canvas::{register_ui_extensions, Canvas};
pub use color::Color;
pub use font::{
    font_atlas_availability, font_atlas_texture_upload, take_font_atlas_diagnostics,
    FontAtlasAvailability, FONT_ATLAS_ASSET,
};
pub use input::{
    hit_test, hit_test_interactive, UiClickEvent, UiInputState, UiPointerEvent, UiValue,
};
pub use layout::{Layout, ScaleMode};
pub use render::{canvas_scale, extract_ui_quads, UiQuad, UiRenderBatch};
pub use types::{ElementId, UiElement, UiElementKind, UiError, UiRect};

/// Return the canonical scene field map for an authored UI canvas.
pub fn serialize_canvas_fields(
    canvas: &Canvas,
) -> std::collections::BTreeMap<String, engine_serialize::Value> {
    canvas::serialize_canvas(canvas)
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default material asset ID assigned to all UI batches.
pub const DEFAULT_UI_MATERIAL: &str = "engine/ui-default";

#[cfg(test)]
mod architecture_tests {
    #[test]
    fn callback_input_compatibility_surface_does_not_return() {
        let source = include_str!("input.rs");
        let old_function = ["pub fn update", "_input("].concat();
        let old_registry = ["struct Callback", "Registry"].concat();

        assert!(!source.contains(&old_function));
        assert!(!source.contains(&old_registry));
    }
}
