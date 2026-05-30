//! UI system for engine-renderer.
//!
//! Produces [`engine_renderer::UiBatch`] data consumed by the rendering pipeline.
//! Elements are positioned in pixel coordinates (+X right, +Y down, origin top-left).

#![forbid(unsafe_code)]

// ---------------------------------------------------------------------------
// Module declarations
// ---------------------------------------------------------------------------

mod batch;
mod canvas;
mod color;
mod layout;
mod types;

// ---------------------------------------------------------------------------
// Re-exports
// ---------------------------------------------------------------------------

pub use canvas::Canvas;
pub use color::Color;
pub use layout::*;
pub use types::{ElementId, UiElement, UiElementKind, UiError, UiRect};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default material asset ID assigned to all UI batches.
pub const DEFAULT_UI_MATERIAL: &str = "engine/ui-default";
