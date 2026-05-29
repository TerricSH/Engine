#![forbid(unsafe_code)]

pub mod debug_draw;
pub mod render_extension;
pub mod render_graph;
pub mod screenshot;
mod types;
mod traits;
mod validation;

pub use debug_draw::{DebugDrawBuffer, DebugDrawProvider, DebugDrawRegistry, DebugLabel, DebugLine, DebugShape};
pub use render_extension::{RenderExtensionProducer, RenderExtensionRegistry};
pub use types::*;
pub use traits::*;
pub use validation::validate_frame_input;

#[cfg(test)]
mod tests;
