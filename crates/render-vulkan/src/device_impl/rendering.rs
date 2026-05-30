//! Render-entry points for VulkanDevice (triangle + model frames).
//!
//! These are high-level methods that orchestrate the full frame lifecycle:
//! acquire, record, submit, present.

use ash::vk;

use render_core::RendererStatistics;

use crate::error::{VkResult, VulkanError};

use super::VulkanDevice;

impl VulkanDevice {
    // --- Phase 1: render_triangle_frame ---

    /// Render one frame using the MVP triangle pipeline (no vertex input).
    pub fn render_triangle_frame(&mut self) -> VkResult<RendererStatistics> {
        if self.minimized {
            return Ok(RendererStatistics::default());
        }
        self.ensure_sc()?;
        if self.mvp_pipeline.is_none() {
            self.build_mvp()?;
        }
        if self.frame_sync.is_empty() {
            self.build_frames()?;
        }
        let fi = self.current_frame;
        let (ii, subopt) = self.acquire(fi)?;
        self.last_image_index = ii;
        self.record_triangle(fi, ii)?;
        self.submit_and_present(fi, ii)?;
        if subopt {
            self.destroy_mvp();
        }
        self.current_frame = (fi + 1) % 2;
        Ok(RendererStatistics {
            draw_calls: 1,
            triangles: 1,
            gpu_frame_ms: 0.0,
        })
    }

    // --- Model rendering (forward shaders + vertex buffers) ---

    /// Render one frame using the forward model pipeline.
    ///
    /// Requires that [`set_mvp_shaders`](Self::set_mvp_shaders) has been
    /// called beforehand with the FORWARD vertex/fragment SPIR-V.
    ///
    /// `vertex_buf` / `index_buf` must have been created via
    /// [`create_buffer`](render_core::Device::create_buffer).  The vertex
    /// layout expected by the pipeline is:
    ///
    /// | location | semantic  | format       | offset |
    /// |----------|-----------|--------------|--------|
    /// | 0        | position  | `float32x3`  | 0      |
    /// | 1        | normal    | `float32x3`  | 12     |
    /// | 2        | UV        | `float32x2`  | 24     |
    ///
    /// Stride: 32 bytes.
    pub fn render_model_frame(
        &mut self,
        vertex_buf: render_core::BufferHandle,
        index_buf: render_core::BufferHandle,
        index_count: u32,
    ) -> VkResult<RendererStatistics> {
        if self.minimized {
            return Ok(RendererStatistics::default());
        }
        self.ensure_sc()?;
        if self.model_pipeline.is_none() {
            self.build_model_pipeline()?;
        }
        if self.frame_sync.is_empty() {
            self.build_frames()?;
        }
        let fi = self.current_frame;
        let (ii, subopt) = self.acquire(fi)?;
        self.last_image_index = ii;

        // ── Shadow mapping ──────────────────────────────────────────────
        self.ensure_shadow()?;
        let light_mvp = self.compute_light_mvp();
        // SAFETY: `light_mvp` is a stack-local array whose address is valid for
        // the full size of `[[f32; 4]; 4]`; the resulting byte slice has the same
        // lifetime as the borrow of `self`.
        let mvp_bytes: &[u8] = unsafe {
            std::slice::from_raw_parts(
                &light_mvp as *const _ as *const u8,
                std::mem::size_of::<[[f32; 4]; 4]>(),
            )
        };
        self.write_ubo(fi, mvp_bytes, 176);

        // ── Record commands ────────────────────────────────────────────
        self.begin_cb(fi)?;
        self.record_shadow_pass(fi, 0, &light_mvp, vertex_buf, index_buf, index_count)?;
        self.record_model(fi, ii, vertex_buf, index_buf, index_count)?;
        self.submit_and_present(fi, ii)?;
        if subopt {
            self.destroy_mvp();
        }
        self.current_frame = (fi + 1) % 2;
        Ok(RendererStatistics {
            draw_calls: 1,
            triangles: index_count as u64 / 3,
            gpu_frame_ms: 0.0,
        })
    }

    /// Record the MVP triangle draw commands.
    fn record_triangle(&self, fi: usize, ii: u32) -> VkResult<()> {
        self.begin_cb(fi)?;
        let d = &self.logical_device.device;
        let f = &self.frame_sync[fi];
        let sc = self
            .swapchain
            .as_ref()
            .ok_or(VulkanError::Loader("swapchain not initialized".into()))?;
        let rp = self.mvp_rp.ok_or(VulkanError::Loader(
            "MVP render pass not initialized".into(),
        ))?;
        let pl = self
            .mvp_pipeline
            .ok_or(VulkanError::Loader("MVP pipeline not initialized".into()))?;
        let cc = [vk::ClearValue {
            color: vk::ClearColorValue {
                float32: [0.02, 0.02, 0.06, 1.0],
            },
        }];
        let rpbi = vk::RenderPassBeginInfo::default()
            .render_pass(rp)
            .framebuffer(self.mvp_framebuffers[ii as usize])
            .render_area(vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: sc.extent,
            })
            .clear_values(&cc);
        // SAFETY: command buffer is in recording state; render pass and
        // framebuffer are valid; `SubpassContents::INLINE` is correct.
        unsafe {
            d.cmd_begin_render_pass(f.command_buffer, &rpbi, vk::SubpassContents::INLINE);
        }
        let vp = vk::Viewport {
            x: 0.0,
            y: 0.0,
            width: sc.extent.width as f32,
            height: sc.extent.height as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        };
        // SAFETY: command buffer is inside a render pass instance; handles are
        // valid Vulkan objects created by the same device.
        unsafe {
            d.cmd_set_viewport(f.command_buffer, 0, &[vp]);
            d.cmd_set_scissor(
                f.command_buffer,
                0,
                &[vk::Rect2D {
                    offset: vk::Offset2D { x: 0, y: 0 },
                    extent: sc.extent,
                }],
            );
            d.cmd_bind_pipeline(f.command_buffer, vk::PipelineBindPoint::GRAPHICS, pl);
            d.cmd_draw(f.command_buffer, 3, 1, 0, 0);
            d.cmd_end_render_pass(f.command_buffer);
        }
        Ok(())
    }

    /// Record the model render pass (color + depth with forward shaders).
    fn record_model(
        &self,
        fi: usize,
        ii: u32,
        vertex_buf: render_core::BufferHandle,
        index_buf: render_core::BufferHandle,
        index_count: u32,
    ) -> VkResult<()> {
        // NOTE: command buffer is already started by render_model_frame
        let d = &self.logical_device.device;
        let f = &self.frame_sync[fi];
        let sc = self
            .swapchain
            .as_ref()
            .ok_or(VulkanError::Loader("swapchain not initialized".into()))?;
        let rp = self.model_rp.ok_or(VulkanError::Loader(
            "model render pass not initialized".into(),
        ))?;
        let pl = self
            .model_pipeline
            .ok_or(VulkanError::Loader("model pipeline not initialized".into()))?;
        let pll = self.model_pipeline_layout.ok_or(VulkanError::Loader(
            "model pipeline layout not initialized".into(),
        ))?;

        // Look up Vulkan buffer handles from the slab
        let vk_vb = self
            .buffers
            .get(vertex_buf.index, vertex_buf.generation)
            .map(|e| e.buffer)
            .ok_or(VulkanError::Loader("vertex buffer not found".into()))?;
        let vk_ib = self
            .buffers
            .get(index_buf.index, index_buf.generation)
            .map(|e| e.buffer)
            .ok_or(VulkanError::Loader("index buffer not found".into()))?;

        // Descriptor set for the current frame (set=0)
        let desc_set = self
            .frame_desc_sets
            .get(fi)
            .copied()
            .unwrap_or(vk::DescriptorSet::null());

        let cc_color = vk::ClearValue {
            color: vk::ClearColorValue {
                float32: [0.02, 0.02, 0.06, 1.0],
            },
        };
        let cc_depth = vk::ClearValue {
            depth_stencil: vk::ClearDepthStencilValue {
                depth: 1.0,
                stencil: 0,
            },
        };
        let cc_both = [cc_color, cc_depth];

        let rpbi = vk::RenderPassBeginInfo::default()
            .render_pass(rp)
            .framebuffer(self.model_framebuffers[ii as usize])
            .render_area(vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: sc.extent,
            })
            .clear_values(&cc_both);
        // SAFETY: `f.command_buffer` is a valid command buffer in the recording
        // state; `rpbi` references a valid render pass and framebuffer.
        unsafe {
            d.cmd_begin_render_pass(f.command_buffer, &rpbi, vk::SubpassContents::INLINE);
        }
        let vp = vk::Viewport {
            x: 0.0,
            y: 0.0,
            width: sc.extent.width as f32,
            height: sc.extent.height as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        };
        // SAFETY: command buffer is in the recording state; all handles
        // (pipeline, descriptor sets, buffers) are valid Vulkan objects created
        // by the same device; the render pass is active.
        unsafe {
            d.cmd_set_viewport(f.command_buffer, 0, &[vp]);
            d.cmd_set_scissor(
                f.command_buffer,
                0,
                &[vk::Rect2D {
                    offset: vk::Offset2D { x: 0, y: 0 },
                    extent: sc.extent,
                }],
            );
            d.cmd_bind_pipeline(f.command_buffer, vk::PipelineBindPoint::GRAPHICS, pl);
            // Bind descriptor sets (set=0 = UBO, set=1 = shadow map)
            let mut desc_sets: Vec<vk::DescriptorSet> = Vec::new();
            if desc_set != vk::DescriptorSet::null() {
                desc_sets.push(desc_set);
            }
            if let Some(sds) = self.shadow_desc_set {
                desc_sets.push(sds);
            }
            if !desc_sets.is_empty() {
                d.cmd_bind_descriptor_sets(
                    f.command_buffer,
                    vk::PipelineBindPoint::GRAPHICS,
                    pll,
                    0,
                    &desc_sets,
                    &[],
                );
            }
            // Bind vertex + index buffers
            let vbs = [vk_vb];
            let offsets = [0u64];
            d.cmd_bind_vertex_buffers(f.command_buffer, 0, &vbs, &offsets);
            d.cmd_bind_index_buffer(f.command_buffer, vk_ib, 0, vk::IndexType::UINT32);
            d.cmd_draw_indexed(f.command_buffer, index_count, 1, 0, 0, 0);
            d.cmd_end_render_pass(f.command_buffer);
        }
        Ok(())
    }
}
