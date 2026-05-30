//! Tone-mapping render pass.
//!
//! Reads the HDR RGBA16F colour attachment and writes LDR (BGRA8) to the
//! swapchain image via a fullscreen triangle.

use engine_renderer::render_graph::{PassAttachment, PassNode, ResourceAccess, SizeSource};
use engine_renderer::{Diagnostic, FrameStats, PassKind, RenderFrameInput, RenderPass};
use render_core::{CommandEncoder, Device};

use crate::scene_renderer::SceneRenderer;

/// Tone-mapping render pass.
///
/// Stores a raw pointer to the [`SceneRenderer`] so it can access the
/// device, current framebuffer index, and encoder.
pub struct TonemapPass {
    renderer: *mut SceneRenderer,
}

// SAFETY: the raw pointer is only accessed on the render thread and the
// backing SceneRenderer outlives this pass.
unsafe impl Send for TonemapPass {}

impl TonemapPass {
    pub fn new(renderer: *mut SceneRenderer) -> Self {
        Self { renderer }
    }
}

impl RenderPass for TonemapPass {
    fn kind(&self) -> &'static str {
        "tone_map_pass"
    }

    fn declare(&self, view_id: u32) -> PassNode {
        PassNode {
            kind: PassKind::ToneMap,
            name: "tone_map_pass",
            view_id,
            inputs: vec![
                PassAttachment {
                    name: "hdr_color".into(),
                    format: Some("RGBA16F".into()),
                    clear: false,
                    load_op: "load".into(),
                    size_source: SizeSource::Swapchain,
                    access: ResourceAccess::Read,
                },
                PassAttachment {
                    name: "ssao_output".into(),
                    format: Some("R8".into()),
                    clear: false,
                    load_op: "load".into(),
                    size_source: SizeSource::Swapchain,
                    access: ResourceAccess::Read,
                },
            ],
            outputs: vec![PassAttachment {
                name: "ldr_color".into(),
                format: Some("RGBA8".into()),
                clear: true,
                load_op: "clear".into(),
                size_source: SizeSource::Swapchain,
                access: ResourceAccess::Write,
            }],
            depth_stencil: None,
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
        sr.execute_tonemap_pass(input, stats)
    }
}
