#![forbid(unsafe_code)]

pub mod debug_draw;
pub mod material_resolver;
pub mod pipeline_library;
pub mod render_extension;
pub mod render_graph;
pub mod render_graph2;
pub mod screenshot;
mod traits;
mod types;
mod validation;

pub use debug_draw::{
    DebugDrawBuffer, DebugDrawProvider, DebugDrawRegistry, DebugLabel, DebugLine, DebugShape,
};
pub use material_resolver::{MaterialPipelineContext, MaterialResolver};
pub use pipeline_library::{hash_vertex_layout, PipelineCacheKey, PipelineLibrary};
pub use render_extension::{RenderExtensionProducer, RenderExtensionRegistry};
pub use render_graph2::{
    AliasSlot, AliasingPlan, PassConfigEntry, PassGraphConfig, PassKind as PassKind2,
    ResourceAccess, TransientResourcePool,
};
pub use traits::*;
pub use types::*;
pub use validation::validate_frame_input;

#[cfg(test)]
mod tests;
