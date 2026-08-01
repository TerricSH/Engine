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

        // Retire pipelines before their layouts and render passes.
        for pipeline in [
            self.tone_pipeline.take(),
            self.hdr_skybox_pipeline.take(),
            self.hdr_vfx_billboard_pipeline.take(),
            self.hdr_vfx_billboard_additive_pipeline.take(),
            self.hdr_vfx_billboard_oit_pipeline.take(),
            self.hdr_gpu_vfx_billboard_pipeline.take(),
            self.hdr_gpu_vfx_billboard_additive_pipeline.take(),
            self.hdr_gpu_vfx_billboard_oit_pipeline.take(),
            self.hdr_instanced_pipeline.take(),
            self.hdr_instanced_double_sided_pipeline.take(),
            self.hdr_forward_pipeline.take(),
            self.hdr_forward_double_sided_pipeline.take(),
            self.hdr_forward_blend_pipeline.take(),
            self.hdr_forward_blend_double_sided_pipeline.take(),
            self.hdr_forward_oit_pipeline.take(),
            self.hdr_forward_oit_double_sided_pipeline.take(),
            self.hdr_forward_additive_pipeline.take(),
            self.hdr_forward_additive_double_sided_pipeline.take(),
        ]
        .into_iter()
        .flatten()
        {
            // SAFETY: each handle was taken from exclusive engine ownership,
            // belongs to idle `d`, and dependent submissions completed first.
            unsafe { d.destroy_pipeline(pipeline, None) };
        }
        for layout in [
            self.tone_pipeline_layout.take(),
            self.hdr_forward_pipeline_layout.take(),
        ]
        .into_iter()
        .flatten()
        {
            // SAFETY: all pipelines using each device-created layout were
            // destroyed above and the renderer exclusively owns the handle.
            unsafe { d.destroy_pipeline_layout(layout, None) };
        }
        for render_pass in [self.tone_rp.take(), self.hdr_forward_rp.take()]
            .into_iter()
            .flatten()
        {
            // SAFETY: framebuffers and pipelines using each device-created pass
            // were destroyed above after the device became idle.
            unsafe { d.destroy_render_pass(render_pass, None) };
        }

        // Tone descriptor set infrastructure
        if let Some(pool) = self.tone_desc_pool.take() {
            // Pool frees its descriptor sets automatically.
            // SAFETY: the device is idle, `pool` belongs to it, and no command
            // retains descriptor sets allocated from the pool.
            unsafe {
                d.destroy_descriptor_pool(pool, None);
            }
        }
        if let Some(layout) = self.tone_desc_layout.take() {
            // SAFETY: pipeline layout/pool users were destroyed above and this
            // descriptor layout is exclusively owned by the renderer.
            unsafe {
                d.destroy_descriptor_set_layout(layout, None);
            }
        }

        // HDR sampler
        if let Some(s) = self.hdr_color_sampler.take() {
            // SAFETY: `s` belongs to idle `d`; all descriptors that referenced
            // it were invalidated with the descriptor pool above.
            unsafe {
                d.destroy_sampler(s, None);
            }
        }

        // HDR color image view + image + allocation
        if let Some(iv) = self.hdr_color_view.take() {
            // SAFETY: framebuffer/descriptor users are gone and this view is an
            // exclusively-owned handle created by idle `d`.
            unsafe {
                d.destroy_image_view(iv, None);
            }
        }
        if let Some(img) = self.hdr_color_image.take() {
            // SAFETY: all dependent views were destroyed before the image and
            // the image belongs exclusively to idle `d`.
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
            // SAFETY: all framebuffer/descriptor users are gone; each taken
            // image view belongs exclusively to idle `d`.
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
            // SAFETY: corresponding views were destroyed above; each image was
            // created by idle `d` and is exclusively owned by this teardown.
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
