use super::*;

impl VulkanDevice {
    // ======================================================================
    // Destruction
    // ======================================================================

    /// Destroy tone-mapping framebuffers only (called on resize).
    pub(super) fn destroy_tone_framebuffers(&mut self) {
        let d = &self.logical_device.device;
        for fb in self.tone_framebuffers.drain(..) {
            // SAFETY: `fb` was created by this device and is still alive.
            unsafe {
                d.destroy_framebuffer(fb, None);
            }
        }
    }

    /// Destroy all HDR + tone-mapping resources (reverse order of creation).
    pub(crate) fn destroy_hdr_resources(&mut self) {
        // Destroy tone-mapping framebuffers first (no device borrow conflict).
        for fb in self.tone_framebuffers.drain(..) {
            let d = &self.logical_device.device;
            // SAFETY: `fb` was created by this device and is still alive.
            unsafe {
                d.destroy_framebuffer(fb, None);
            }
        }

        let d = &self.logical_device.device;

        // Forward HDR framebuffer
        if let Some(fb) = self.hdr_forward_fb.take() {
            // SAFETY: `fb` was created by this device.
            unsafe {
                d.destroy_framebuffer(fb, None);
            }
        }

        // Tone pipeline + layout
        if let Some(p) = self.tone_pipeline.take() {
            unsafe {
                d.destroy_pipeline(p, None);
            }
        }
        if let Some(l) = self.tone_pipeline_layout.take() {
            unsafe {
                d.destroy_pipeline_layout(l, None);
            }
        }

        // Forward HDR pipeline + layout
        if let Some(p) = self.hdr_skybox_pipeline.take() {
            unsafe {
                d.destroy_pipeline(p, None);
            }
        }
        if let Some(p) = self.hdr_vfx_billboard_pipeline.take() {
            unsafe {
                d.destroy_pipeline(p, None);
            }
        }
        if let Some(p) = self.hdr_vfx_billboard_additive_pipeline.take() {
            unsafe {
                d.destroy_pipeline(p, None);
            }
        }
        if let Some(p) = self.hdr_vfx_billboard_oit_pipeline.take() {
            unsafe {
                d.destroy_pipeline(p, None);
            }
        }
        if let Some(p) = self.hdr_gpu_vfx_billboard_pipeline.take() {
            unsafe {
                d.destroy_pipeline(p, None);
            }
        }
        if let Some(p) = self.hdr_gpu_vfx_billboard_additive_pipeline.take() {
            unsafe {
                d.destroy_pipeline(p, None);
            }
        }
        if let Some(p) = self.hdr_gpu_vfx_billboard_oit_pipeline.take() {
            unsafe {
                d.destroy_pipeline(p, None);
            }
        }
        if let Some(p) = self.hdr_instanced_pipeline.take() {
            unsafe {
                d.destroy_pipeline(p, None);
            }
        }
        if let Some(p) = self.hdr_instanced_double_sided_pipeline.take() {
            unsafe {
                d.destroy_pipeline(p, None);
            }
        }
        if let Some(p) = self.hdr_forward_pipeline.take() {
            unsafe {
                d.destroy_pipeline(p, None);
            }
        }
        if let Some(p) = self.hdr_forward_double_sided_pipeline.take() {
            unsafe {
                d.destroy_pipeline(p, None);
            }
        }
        if let Some(p) = self.hdr_forward_blend_pipeline.take() {
            unsafe {
                d.destroy_pipeline(p, None);
            }
        }
        if let Some(p) = self.hdr_forward_blend_double_sided_pipeline.take() {
            unsafe {
                d.destroy_pipeline(p, None);
            }
        }
        if let Some(p) = self.hdr_forward_oit_pipeline.take() {
            unsafe {
                d.destroy_pipeline(p, None);
            }
        }
        if let Some(p) = self.hdr_forward_oit_double_sided_pipeline.take() {
            unsafe {
                d.destroy_pipeline(p, None);
            }
        }
        if let Some(p) = self.hdr_forward_additive_pipeline.take() {
            unsafe {
                d.destroy_pipeline(p, None);
            }
        }
        if let Some(p) = self.hdr_forward_additive_double_sided_pipeline.take() {
            unsafe {
                d.destroy_pipeline(p, None);
            }
        }
        if let Some(l) = self.hdr_forward_pipeline_layout.take() {
            unsafe {
                d.destroy_pipeline_layout(l, None);
            }
        }

        // Render passes
        if let Some(rp) = self.tone_rp.take() {
            unsafe {
                d.destroy_render_pass(rp, None);
            }
        }
        if let Some(rp) = self.hdr_forward_rp.take() {
            unsafe {
                d.destroy_render_pass(rp, None);
            }
        }

        // Tone descriptor set infrastructure
        if let Some(pool) = self.tone_desc_pool.take() {
            // Pool frees its descriptor sets automatically.
            unsafe {
                d.destroy_descriptor_pool(pool, None);
            }
        }
        if let Some(layout) = self.tone_desc_layout.take() {
            unsafe {
                d.destroy_descriptor_set_layout(layout, None);
            }
        }

        // HDR sampler
        if let Some(s) = self.hdr_color_sampler.take() {
            unsafe {
                d.destroy_sampler(s, None);
            }
        }

        // HDR color image view + image + allocation
        if let Some(iv) = self.hdr_color_view.take() {
            unsafe {
                d.destroy_image_view(iv, None);
            }
        }
        if let Some(img) = self.hdr_color_image.take() {
            unsafe {
                d.destroy_image(img, None);
            }
        }
        if let Some(mut a) = self.hdr_color_allocation.take() {
            let allocator = self.logical_device.allocator();
            free_hdr_target_allocation(&allocator, &mut a);
        }
        for image_view in [
            self.hdr_msaa_color_view.take(),
            self.oit_msaa_accum_view.take(),
            self.oit_msaa_optical_depth_view.take(),
            self.hdr_msaa_depth_view.take(),
            self.oit_accum_view.take(),
            self.oit_optical_depth_view.take(),
        ]
        .into_iter()
        .flatten()
        {
            unsafe {
                d.destroy_image_view(image_view, None);
            }
        }
        for image in [
            self.hdr_msaa_color_image.take(),
            self.oit_msaa_accum_image.take(),
            self.oit_msaa_optical_depth_image.take(),
            self.hdr_msaa_depth_image.take(),
            self.oit_accum_image.take(),
            self.oit_optical_depth_image.take(),
        ]
        .into_iter()
        .flatten()
        {
            unsafe {
                d.destroy_image(image, None);
            }
        }
        for mut allocation in [
            self.hdr_msaa_color_allocation.take(),
            self.oit_msaa_accum_allocation.take(),
            self.oit_msaa_optical_depth_allocation.take(),
            self.hdr_msaa_depth_allocation.take(),
            self.oit_accum_allocation.take(),
            self.oit_optical_depth_allocation.take(),
        ]
        .into_iter()
        .flatten()
        {
            let allocator = self.logical_device.allocator();
            free_hdr_target_allocation(&allocator, &mut allocation);
        }
    }
}
