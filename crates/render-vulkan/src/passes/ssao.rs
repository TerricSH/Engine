//! SSAO (screen-space ambient occlusion) render pass.
//!
//! Samples the depth buffer and writes an occlusion factor to an R8 texture.
//! No-op when SSAO resources are not available.

use engine_renderer::render_graph::{PassAttachment, PassNode, ResourceAccess, SizeSource};
use engine_renderer::{Diagnostic, FrameStats, RenderFrameInput, RenderPass, PassKind};
use render_core::{CommandEncoder, Device};

use crate::device_impl::VulkanDevice;

/// SSAO post-process pass.
///
/// Stores a raw pointer to the [`VulkanDevice`] so it can dispatch the
/// compute-based SSAO shader directly through the device's methods.
pub struct SSAOPass {
    device: *mut VulkanDevice,
}

// SAFETY: VulkanDevice is Send; the pass only uses its device pointer
// on the render thread.
unsafe impl Send for SSAOPass {}

impl SSAOPass {
    pub fn new(device: *mut VulkanDevice) -> Self {
        Self { device }
    }

    unsafe fn device_mut(&mut self) -> &mut VulkanDevice {
        &mut *self.device
    }
}

impl RenderPass for SSAOPass {
    fn kind(&self) -> &'static str {
        "ssao"
    }

    fn declare(&self, view_id: u32) -> PassNode {
        PassNode {
            kind: PassKind::Custom("ssao"),
            name: "ssao",
            view_id,
            inputs: vec![PassAttachment {
                name: "depth_stencil".into(),
                format: Some("D32".into()),
                clear: false,
                load_op: "load".into(),
                size_source: SizeSource::Swapchain,
                access: ResourceAccess::Read,
            }],
            outputs: vec![PassAttachment {
                name: "ssao_output".into(),
                format: Some("R8".into()),
                clear: true,
                load_op: "clear".into(),
                size_source: SizeSource::Swapchain,
                access: ResourceAccess::Write,
            }],
            depth_stencil: None,
        }
    }

    fn prepare(&mut self, _device: &mut dyn Device) -> Result<(), Vec<Diagnostic>> {
        // SSAO resources are created lazily by the device.
        Ok(())
    }

    fn execute(
        &mut self,
        _input: &RenderFrameInput,
        _encoder: &mut dyn CommandEncoder,
        _stats: &mut FrameStats,
    ) -> Result<(), Vec<Diagnostic>> {
        // SAFETY: the device outlives this pass; no conflicting borrows.
        let device = unsafe { self.device_mut() };
        device.update_ssao_depth_descriptor();
        let fi = device.current_frame;
        device.dispatch_ssao(fi);
        Ok(())
    }
}
