//! VkCmdEncoder — implements render_core::CommandEncoder.

use ash::vk;
use ash::Device as AshDevice;

use render_core::{
    BufferHandle, CommandEncoder as CmdEncoderTrait, FramebufferHandle, IndexFormat,
    PipelineHandle, PipelineLayoutHandle, RenderPassHandle,
};

// ============================================================================
// VkCmdEncoder
// ============================================================================

pub(crate) struct VkCmdEncoder {
    pub(crate) device: AshDevice,
    pub(crate) cmd: vk::CommandBuffer,
    /// Snapshot of slab entries taken at encoder creation.
    /// Each slot: `Some((generation, pipeline))` if occupied.
    pub(crate) pipeline_cache: Vec<Option<(u32, vk::Pipeline)>>,
    /// Snapshot of slab entries taken at encoder creation.
    /// Each slot: `Some((generation, buffer))` if occupied.
    pub(crate) buffer_cache: Vec<Option<(u32, vk::Buffer)>>,
    /// Snapshot of slab entries taken at encoder creation.
    /// Each slot: `Some((generation, render_pass))` if occupied.
    pub(crate) render_pass_cache: Vec<Option<(u32, vk::RenderPass)>>,
    /// Snapshot of slab entries taken at encoder creation.
    /// Each slot: `Some((generation, framebuffer))` if occupied.
    pub(crate) framebuffer_cache: Vec<Option<(u32, vk::Framebuffer, u32, bool)>>,
    /// Snapshot of slab entries taken at encoder creation.
    /// Each slot: `Some((generation, layout))` if occupied.
    pub(crate) pipeline_layout_cache: Vec<Option<(u32, vk::PipelineLayout)>>,
    // Per-frame descriptor set (set=0 per FD-041), set by begin_frame
    pub(crate) current_desc_set: vk::DescriptorSet,
    pub(crate) render_pass_active: bool,
}
// VkCmdEncoder: all fields are Send (AshDevice and Vulkan handles), no raw pointers.
// The unsafe impl Send is removed — Send is derived automatically.

impl CmdEncoderTrait for VkCmdEncoder {
    fn begin_render_pass(
        &mut self,
        rp: RenderPassHandle,
        fb: FramebufferHandle,
        area: (u32, u32, u32, u32),
        clear: [f32; 4],
        depth: Option<f32>,
    ) {
        let rp_ = self.render_pass_cache.get(rp.index as usize).and_then(|s| {
            s.as_ref()
                .filter(|(g, _)| *g == rp.generation)
                .map(|(_, v)| *v)
        });
        let fb_ = self.framebuffer_cache.get(fb.index as usize).and_then(|s| {
            s.as_ref()
                .filter(|(g, ..)| *g == fb.generation)
                .map(|(_, v, color_count, has_depth)| (*v, *color_count, *has_depth))
        });
        if let (Some(rp_), Some((fb_, color_count, has_depth))) = (rp_, fb_) {
            if self.render_pass_active {
                return;
            }
            let mut clear_values =
                Vec::with_capacity(color_count as usize + usize::from(has_depth));
            for _ in 0..color_count {
                clear_values.push(vk::ClearValue {
                    color: vk::ClearColorValue { float32: clear },
                });
            }
            if has_depth {
                clear_values.push(vk::ClearValue {
                    depth_stencil: vk::ClearDepthStencilValue {
                        depth: depth.unwrap_or(1.0),
                        stencil: 0,
                    },
                });
            }
            let rpbi = vk::RenderPassBeginInfo::default()
                .render_pass(rp_)
                .framebuffer(fb_)
                .render_area(vk::Rect2D {
                    offset: vk::Offset2D {
                        x: area.0 as i32,
                        y: area.1 as i32,
                    },
                    extent: vk::Extent2D {
                        width: area.2,
                        height: area.3,
                    },
                })
                .clear_values(&clear_values);
            unsafe {
                self.device
                    .cmd_begin_render_pass(self.cmd, &rpbi, vk::SubpassContents::INLINE);
            }
            self.render_pass_active = true;
        }
    }
    fn bind_pipeline(&mut self, p: PipelineHandle) {
        if let Some(&pipeline) = self.pipeline_cache.get(p.index as usize).and_then(|s| {
            s.as_ref()
                .filter(|(g, _)| *g == p.generation)
                .map(|(_, v)| v)
        }) {
            unsafe {
                self.device
                    .cmd_bind_pipeline(self.cmd, vk::PipelineBindPoint::GRAPHICS, pipeline);
            }
        }
    }
    fn bind_vertex_buffers(&mut self, bufs: &[BufferHandle], offs: &[u64]) {
        let v: Vec<vk::Buffer> = bufs
            .iter()
            .filter_map(|h| {
                self.buffer_cache.get(h.index as usize).and_then(|s| {
                    s.as_ref()
                        .filter(|(g, _)| *g == h.generation)
                        .map(|(_, b)| *b)
                })
            })
            .collect();
        if !v.is_empty() {
            unsafe {
                self.device.cmd_bind_vertex_buffers(self.cmd, 0, &v, offs);
            }
        }
    }
    fn bind_index_buffer(&mut self, buf: BufferHandle, o: u64, f: IndexFormat) {
        if let Some(&buffer) = self.buffer_cache.get(buf.index as usize).and_then(|s| {
            s.as_ref()
                .filter(|(g, _)| *g == buf.generation)
                .map(|(_, b)| b)
        }) {
            unsafe {
                self.device.cmd_bind_index_buffer(
                    self.cmd,
                    buffer,
                    o,
                    match f {
                        IndexFormat::U16 => vk::IndexType::UINT16,
                        IndexFormat::U32 => vk::IndexType::UINT32,
                    },
                );
            }
        }
    }
    fn bind_descriptor_sets(
        &mut self,
        pl: PipelineLayoutHandle,
        fs: u32,
        sets: &[render_core::DescriptorSetHandle],
        do_: &[u32],
    ) -> Result<(), render_core::RhiError> {
        if !sets.is_empty() {
            return Err(render_core::RhiError::UnsupportedFeature {
                feature: "binding external Vulkan descriptor-set handles through the portable RHI"
                    .to_owned(),
            });
        }

        let layout = self
            .pipeline_layout_cache
            .get(pl.index as usize)
            .and_then(|s| {
                s.as_ref()
                    .filter(|(g, _)| *g == pl.generation)
                    .map(|(_, l)| *l)
            })
            .ok_or(render_core::RhiError::InvalidHandle)?;
        let set = self.current_desc_set;
        if set == vk::DescriptorSet::null() {
            return Err(render_core::RhiError::UnsupportedFeature {
                feature: "binding the per-frame Vulkan descriptor set before it is allocated"
                    .to_owned(),
            });
        }
        let frame_sets = [set];
        unsafe {
            self.device.cmd_bind_descriptor_sets(
                self.cmd,
                vk::PipelineBindPoint::GRAPHICS,
                layout,
                fs,
                &frame_sets,
                do_,
            );
        }
        Ok(())
    }
    fn set_viewport(&mut self, x: f32, y: f32, w: f32, h: f32, md: f32, mxd: f32) {
        unsafe {
            self.device.cmd_set_viewport(
                self.cmd,
                0,
                &[vk::Viewport {
                    x,
                    y,
                    width: w,
                    height: h,
                    min_depth: md,
                    max_depth: mxd,
                }],
            );
        }
    }
    fn set_scissor(&mut self, x: i32, y: i32, w: u32, h: u32) {
        unsafe {
            self.device.cmd_set_scissor(
                self.cmd,
                0,
                &[vk::Rect2D {
                    offset: vk::Offset2D { x, y },
                    extent: vk::Extent2D {
                        width: w,
                        height: h,
                    },
                }],
            );
        }
    }
    fn draw(&mut self, vc: u32, ic: u32, fv: u32, fi: u32) {
        unsafe {
            self.device.cmd_draw(self.cmd, vc, ic, fv, fi);
        }
    }
    fn draw_indexed(&mut self, ic: u32, ins: u32, fi: u32, vo: i32, fii: u32) {
        unsafe {
            self.device.cmd_draw_indexed(self.cmd, ic, ins, fi, vo, fii);
        }
    }
    fn draw_indexed_indirect(
        &mut self,
        buffer: BufferHandle,
        offset: u64,
        draw_count: u32,
        stride: u32,
    ) -> Result<(), render_core::RhiError> {
        let buf = self
            .buffer_cache
            .get(buffer.index as usize)
            .and_then(|slot| {
                slot.as_ref()
                    .filter(|(generation, _)| *generation == buffer.generation)
                    .map(|(_, buffer)| *buffer)
            })
            .ok_or(render_core::RhiError::InvalidHandle)?;
        // SAFETY: command buffer is in recording state; `buf` is a valid
        // VkBuffer owned by this device. Buffer usage/range validation is done
        // when the RHI buffer and draw list are built.
        unsafe {
            self.device
                .cmd_draw_indexed_indirect(self.cmd, buf, offset, draw_count, stride);
        }
        Ok(())
    }
    fn push_constants(&mut self, pl: PipelineLayoutHandle, sf: u32, off: u32, data: &[u8]) {
        if let Some(&layout) = self
            .pipeline_layout_cache
            .get(pl.index as usize)
            .and_then(|s| {
                s.as_ref()
                    .filter(|(g, _)| *g == pl.generation)
                    .map(|(_, l)| l)
            })
        {
            unsafe {
                self.device.cmd_push_constants(
                    self.cmd,
                    layout,
                    vk::ShaderStageFlags::from_raw(sf),
                    off,
                    data,
                );
            }
        }
    }
    fn end_render_pass(&mut self) {
        if self.render_pass_active {
            unsafe {
                self.device.cmd_end_render_pass(self.cmd);
            }
            self.render_pass_active = false;
        }
    }
}
