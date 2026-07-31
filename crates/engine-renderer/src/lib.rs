#![forbid(unsafe_code)]

pub mod backend_shared;
mod clustered_lighting;
pub mod debug_draw;
pub mod frame_timing;
mod gpu_particles;
pub mod pipeline_library;
pub mod render_extension;
pub mod render_graph2;
pub mod render_pass;
pub mod screenshot;
mod traits;
mod types;
mod validation;

pub use clustered_lighting::*;
pub use debug_draw::{
    DebugDrawBuffer, DebugDrawProvider, DebugDrawRegistry, DebugLabel, DebugLine, DebugShape,
};
pub use frame_timing::{
    FrameTimingSummary, FrameTimingTracker, FrameTimings, GpuPassTime, GpuTimingStatus, PassTiming,
    PassTimingStats, TimingAggregate, DEFAULT_TIMING_WINDOW,
};
pub use gpu_particles::*;
pub use pipeline_library::{hash_vertex_layout, PipelineCacheKey, PipelineLibrary};
pub use render_extension::{RenderExtensionProducer, RenderExtensionRegistry};
pub use render_graph2::{
    AliasSlot, AliasingPlan, PassConfigEntry, PassGraphConfig, PassGraphOutputMode, PassKind,
    ResourceAccess, TransientResourcePool,
};
pub use render_pass::{PassRegistry, RenderPass};
pub use traits::*;
pub use types::*;
pub use validation::{
    validate_environment_settings, validate_frame_input, validate_pass_graph_settings,
    validate_post_process_settings,
};

#[cfg(test)]
mod tests;
