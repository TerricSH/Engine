//! Bloom post-process render pass.
//!
//! Dispatches the downsample/upsample bloom chain via compute shaders.
//! No-op when bloom resources haven't been initialised (SPIR-V not available).

use engine_renderer::render_graph::{PassAttachment, PassNode, ResourceAccess, SizeSource};
use engine_renderer::{Diagnostic, FrameStats, RenderFrameInput, RenderPass, PassKind};
use render_core::{CommandEncoder, Device};

use crate::device_impl::VulkanDevice;

/// Bloom post-process pass.
///
/// Stores a raw pointer to the [`VulkanDevice`] so it can dispatch the
/// compute-based bloom chain directly through the device's methods.
pub struct BloomPass {
    device: *mut VulkanDevice,
}

// SAFETY: VulkanDevice is Send; the pass only uses its device pointer
// on the thread where it was created (the render thread).
unsafe impl Send for BloomPass {}

impl BloomPass {
    pub fn new(device: *mut VulkanDevice) -> Self {
        Self { device }
    }

    /// Access the underlying device (caller must ensure the device outlives
    /// this pass and that no conflicting borrows exist).
    unsafe fn device(&self) -> &VulkanDevice {
        &*self.device
    }
}

impl RenderPass for BloomPass {
    fn kind(&self) -> &'static str {
        "bloom"
    }

    fn declare(&self, view_id: u32) -> PassNode {
        PassNode {
            kind: PassKind::Custom("bloom"),
            name: "bloom",
            view_id,
            inputs: vec![PassAttachment {
                name: "hdr_color".into(),
                format: Some("RGBA16F".into()),
                clear: false,
                load_op: "load".into(),
                size_source: SizeSource::Swapchain,
                access: ResourceAccess::Read,
            }],
            outputs: vec![],
            depth_stencil: None,
        }
    }

    fn prepare(&mut self, _device: &mut dyn Device) -> Result<(), Vec<Diagnostic>> {
        // Bloom resources are created lazily by the device; no extra
        // preparation needed here.
        Ok(())
    }

    fn execute(
        &mut self,
        _input: &RenderFrameInput,
        _encoder: &mut dyn CommandEncoder,
        _stats: &mut FrameStats,
    ) -> Result<(), Vec<Diagnostic>> {
        // SAFETY: the device outlives this pass; no conflicting borrows
        // exist because execute() takes &mut self (the pass) and the
        // device is in a separate allocation.
        let device = unsafe { self.device() };
        device.dispatch_bloom(device.current_frame);
        Ok(())
    }
}
