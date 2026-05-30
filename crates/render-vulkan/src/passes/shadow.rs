//! Directional shadow (CSM) render pass.
//!
//! Renders shadow-casting drawables into 3 cascade layers of a 2D array
//! depth texture using the device's shadow pipeline.

use engine_renderer::render_graph::{PassAttachment, PassNode, ResourceAccess, SizeSource};
use engine_renderer::{Diagnostic, FrameStats, PassKind, RenderFrameInput, RenderPass};
use render_core::{CommandEncoder, Device};

use crate::scene_renderer::SceneRenderer;

/// Directional shadow render pass.
///
/// Stores a raw pointer to the [`SceneRenderer`] so it can access the
/// device, mesh cache, and compute cascade data.
pub struct ShadowPass {
    renderer: *mut SceneRenderer,
}

// SAFETY: the raw pointer is only accessed on the render thread and the
// backing SceneRenderer outlives this pass.
unsafe impl Send for ShadowPass {}

impl ShadowPass {
    pub fn new(renderer: *mut SceneRenderer) -> Self {
        Self { renderer }
    }
}

impl RenderPass for ShadowPass {
    fn kind(&self) -> &'static str {
        "directional_shadow_pass"
    }

    fn declare(&self, view_id: u32) -> PassNode {
        PassNode {
            kind: PassKind::DirectionalShadow,
            name: "directional_shadow_pass",
            view_id,
            inputs: vec![PassAttachment {
                name: "depth".into(),
                format: Some("D32".into()),
                clear: true,
                load_op: "clear".into(),
                size_source: SizeSource::Swapchain,
                access: ResourceAccess::Read,
            }],
            outputs: vec![PassAttachment {
                name: "shadow_map".into(),
                format: Some("D32".into()),
                clear: false,
                load_op: "load".into(),
                size_source: SizeSource::Custom(1024, 1024),
                access: ResourceAccess::Write,
            }],
            depth_stencil: Some(PassAttachment {
                name: "shadow_depth".into(),
                format: Some("D32".into()),
                clear: true,
                load_op: "clear".into(),
                size_source: SizeSource::Custom(1024, 1024),
                access: ResourceAccess::ReadWrite,
            }),
        }
    }

    fn prepare(&mut self, _device: &mut dyn Device) -> Result<(), Vec<Diagnostic>> {
        Ok(())
    }

    fn execute(
        &mut self,
        input: &RenderFrameInput,
        _encoder: &mut dyn CommandEncoder,
        stats: &mut FrameStats,
    ) -> Result<(), Vec<Diagnostic>> {
        // SAFETY: the SceneRenderer outlives this pass; separate allocation.
        let sr = unsafe { &mut *self.renderer };
        sr.execute_shadow_pass(input, stats)
    }
}
