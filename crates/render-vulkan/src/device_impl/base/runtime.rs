use super::super::*;

impl VulkanDevice {
    pub fn set_forward_shaders(&mut self, vert: &[u8], frag: &[u8]) {
        self.forward_vert_spv = Some(vert.to_vec());
        self.forward_frag_spv = Some(frag.to_vec());
    }

    pub fn set_skinned_vertex_shader(&mut self, vert: &[u8]) {
        self.skinned_vert_spv = Some(vert.to_vec());
    }

    pub fn set_vfx_billboard_shaders(&mut self, vert: &[u8], gpu_vert: &[u8], frag: &[u8]) {
        self.vfx_billboard_vert_spv = Some(vert.to_vec());
        self.gpu_vfx_billboard_vert_spv = Some(gpu_vert.to_vec());
        self.vfx_billboard_frag_spv = Some(frag.to_vec());
    }

    pub fn set_instanced_vertex_shader(&mut self, vert: &[u8]) {
        self.instanced_vert_spv = Some(vert.to_vec());
    }

    pub fn set_skybox_shaders(&mut self, vert: &[u8], frag: &[u8]) {
        self.skybox_vert_spv = Some(vert.to_vec());
        self.skybox_frag_spv = Some(frag.to_vec());
    }

    /// Returns the index of the current in-flight frame (0 or 1 for double
    /// buffering). Used by sandbox code that writes per-frame UBO data via
    /// [`write_ubo`](Self::write_ubo).
    pub fn current_frame_index(&self) -> usize {
        self.current_frame
    }

    /// Create one framebuffer per swapchain image view, each with colour +
    /// depth attachments.  Inserts into the framebuffer slab and returns
    /// handles that the `VkCmdEncoder` can resolve.
    pub fn create_scene_framebuffers(
        &mut self,
        render_pass: render_core::RenderPassHandle,
    ) -> VkResult<Vec<FramebufferHandle>> {
        let render_pass = self
            .render_passes
            .get(render_pass.index, render_pass.generation)
            .copied()
            .ok_or_else(|| VulkanError::Loader("invalid scene render-pass handle".into()))?;
        let sc = self
            .swapchain
            .as_ref()
            .ok_or(VulkanError::Loader("no swapchain".into()))?;
        let dv = self.depth_image_view.unwrap_or(vk::ImageView::null());
        let ext = self.swapchain_extent;
        let mut handles = Vec::with_capacity(sc.image_views.len());
        for &iv in &sc.image_views {
            let att = [iv, dv];
            // SAFETY: device is valid; framebuffer info references valid image
            // views that outlive this device; render pass is alive.
            let fb = unsafe {
                self.logical_device.device.create_framebuffer(
                    &vk::FramebufferCreateInfo::default()
                        .render_pass(render_pass)
                        .attachments(&att)
                        .width(ext.width)
                        .height(ext.height)
                        .layers(1),
                    None,
                )
            }
            .map_err(|e| VulkanError::vk("create_scene_fb", e))?;
            let (idx, gen) = self.framebuffers.insert(FbEntry {
                framebuffer: fb,
                color_attachment_count: 1,
                has_depth: true,
            });
            handles.push(FramebufferHandle::new(idx, gen));
        }
        Ok(handles)
    }

    /// Remove scene framebuffers from the slab and destroy their Vulkan handles.
    pub fn destroy_scene_framebuffers(&mut self, handles: &[FramebufferHandle]) {
        let d = &self.logical_device.device;
        for h in handles {
            if let Some(fb) = self.framebuffers.remove(h.index, h.generation) {
                // SAFETY: `fb` was created by this device and is no longer
                // referenced by any in-flight frame.
                unsafe {
                    d.destroy_framebuffer(fb.framebuffer, None);
                }
            }
        }
    }

    /// Convenience wrapper: write UBO data for the current in-flight frame.
    /// Delegates to [`write_ubo`](Self::write_ubo).
    ///
    /// # Panics
    ///
    /// Panics if `data` exceeds `ubo_size - offset`.
    pub fn write_ubo_current(&mut self, data: &[u8], offset: u64) {
        self.write_ubo(self.current_frame, data, offset);
    }

    pub(crate) fn retire_pipeline(&mut self, pipeline: vk::Pipeline) {
        if pipeline == vk::Pipeline::null() {
            return;
        }

        if self.frame_sync.is_empty() || self.retired_pipelines.is_empty() {
            // SAFETY: `pipeline` was created by this device and there are no
            // in-flight frames that could still reference it.
            unsafe {
                self.logical_device.device.destroy_pipeline(pipeline, None);
            }
            return;
        }

        let retire_index = self.current_frame % self.retired_pipelines.len();
        self.retired_pipelines[retire_index].push(pipeline);
    }

    pub(crate) fn drain_retired_pipelines(&mut self, frame_index: usize) {
        let Some(slot) = self.retired_pipelines.get_mut(frame_index) else {
            return;
        };

        let retired = std::mem::take(slot);
        for pipeline in retired {
            // SAFETY: the fence for `frame_index` has already completed when
            // this queue is drained, so no in-flight submission can still
            // reference the retired pipeline.
            unsafe {
                self.logical_device.device.destroy_pipeline(pipeline, None);
            }
        }
    }

    pub(crate) fn drain_all_retired_pipelines(&mut self) {
        for frame_index in 0..self.retired_pipelines.len() {
            self.drain_retired_pipelines(frame_index);
        }
    }

    /// Tear down resources that reference the current swapchain images.
    pub(crate) fn destroy_swapchain_resources(&mut self) {
        self.destroy_ui_overlay_resources();
        self.destroy_descriptor_infra();
        self.destroy_depth_texture();
        self.destroy_hdr_resources();
        self.swapchain = None;
        self.swapchain_recreate_pending = false;
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        self.window_width = w.max(1);
        self.window_height = h.max(1);
        self.minimized = w == 0 || h == 0;
        // SAFETY: `self.logical_device` is alive by type invariant (ManuallyDrop
        // ensures VkLogicalDevice is not dropped before VulkanDevice).
        unsafe {
            let _ = self.logical_device.device.device_wait_idle();
        };
        self.destroy_swapchain_resources();
    }
    pub fn wait_idle(&self) {
        // SAFETY: `self.logical_device` is alive by type invariant (ManuallyDrop
        // ensures VkLogicalDevice is not dropped before VulkanDevice).
        unsafe {
            let _ = self.logical_device.device.device_wait_idle();
        };
    }
}
