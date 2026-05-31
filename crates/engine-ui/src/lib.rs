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
pub use input::{hit_test, update_input, CallbackRegistry, UiInputState};
pub use layout::{Layout, ScaleMode};
pub use render::{canvas_scale, extract_ui_quads, UiQuad, UiRenderBatch};
pub use types::{ElementId, UiElement, UiElementKind, UiError, UiRect};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default material asset ID assigned to all UI batches.
pub const DEFAULT_UI_MATERIAL: &str = "engine/ui-default";
