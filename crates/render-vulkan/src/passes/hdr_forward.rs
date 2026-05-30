//! Opaque PBR forward render pass — the main forward-shading pass.
//!
//! Renders all opaque drawables and skinned items into the RGBA16F HDR
//! colour attachment via the device-side HDR forward pipeline.

use engine_renderer::render_graph::{PassAttachment, PassNode, ResourceAccess, SizeSource};
use engine_renderer::{Diagnostic, FrameStats, PassKind, RenderFrameInput, RenderPass};
use render_core::{CommandEncoder, Device};

use crate::scene_renderer::SceneRenderer;

/// HDR forward render pass.
///
/// Stores a raw pointer to the [`SceneRenderer`] so it can access the
/// device, mesh cache, and material/descriptor-set caches that are
/// needed to issue per-drawable draw calls.
pub struct HdrForwardPass {
    renderer: *mut SceneRenderer,
}

// SAFETY: the raw pointer is only accessed on the render thread and the
// backing SceneRenderer outlives this pass.
unsafe impl Send for HdrForwardPass {}

impl HdrForwardPass {
    pub fn new(renderer: *mut SceneRenderer) -> Self {
        Self { renderer }
    }
}

impl RenderPass for HdrForwardPass {
    fn kind(&self) -> &'static str {
        "opaque_pbr_forward_pass"
    }

    fn declare(&self, view_id: u32) -> PassNode {
        PassNode {
            kind: PassKind::OpaquePbrForward,
            name: "opaque_pbr_forward_pass",
            view_id,
            inputs: vec![],
            outputs: vec![PassAttachment {
                name: "hdr_color".into(),
                format: Some("RGBA16F".into()),
                clear: true,
                load_op: "clear".into(),
                size_source: SizeSource::Swapchain,
                access: ResourceAccess::Write,
            }],
            depth_stencil: Some(PassAttachment {
                name: "depth_stencil".into(),
                format: Some("D24S8".into()),
                clear: true,
                load_op: "clear".into(),
                size_source: SizeSource::Swapchain,
                access: ResourceAccess::ReadWrite,
            }),
        }
    }

    fn prepare(&mut self, _device: &mut dyn Device) -> Result<(), Vec<Diagnostic>> {
        // HDR forward resources are created lazily by VulkanDevice during
        // init_once().  No extra preparation is required here.
        Ok(())
    }

    fn execute(
        &mut self,
        input: &RenderFrameInput,
        _encoder: &mut dyn CommandEncoder,
        stats: &mut FrameStats,
    ) -> Result<(), Vec<Diagnostic>> {
        // SAFETY: the SceneRenderer outlives this pass; the raw pointer
        // points to a separate heap allocation so there is no aliasing
        // with &mut self (the pass).
        let sr = unsafe { &mut *self.renderer };
        // Delegate to the extracted implementation on SceneRenderer.
        sr.execute_hdr_forward_pass(input, stats)
    }
}
