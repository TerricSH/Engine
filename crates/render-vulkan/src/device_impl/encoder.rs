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
    /// Shadow map image for pipeline barrier (null = no shadow map).
    pub(crate) shadow_map: vk::Image,
    /// HDR color image for pipeline barrier (null = no HDR).
    pub(crate) hdr_image: vk::Image,
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
    pub(crate) framebuffer_cache: Vec<Option<(u32, vk::Framebuffer)>>,
    /// Snapshot of slab entries taken at encoder creation.
    /// Each slot: `Some((generation, layout))` if occupied.
    pub(crate) pipeline_layout_cache: Vec<Option<(u32, vk::PipelineLayout)>>,
    // Per-frame descriptor set (set=0 per FD-041), set by begin_frame
    pub(crate) current_desc_set: vk::DescriptorSet,
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
        _depth: Option<f32>,
    ) {
        let rp_ = self.render_pass_cache.get(rp.index as usize).and_then(|s| {
            s.as_ref()
                .filter(|(g, _)| *g == rp.generation)
                .map(|(_, v)| *v)
        });
        let fb_ = self.framebuffer_cache.get(fb.index as usize).and_then(|s| {
            s.as_ref()
                .filter(|(g, _)| *g == fb.generation)
                .map(|(_, v)| *v)
        });
        if let (Some(rp_), Some(fb_)) = (rp_, fb_) {
            let cc = vk::ClearValue {
                color: vk::ClearColorValue { float32: clear },
            };
            let cc_arr = [cc];
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
                .clear_values(&cc_arr);
            unsafe {
                self.device
                    .cmd_begin_render_pass(self.cmd, &rpbi, vk::SubpassContents::INLINE);
            }
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
        _: &[render_core::DescriptorSetHandle],
        do_: &[u32],
    ) {
        if let Some(&layout) = self
            .pipeline_layout_cache
            .get(pl.index as usize)
            .and_then(|s| {
                s.as_ref()
                    .filter(|(g, _)| *g == pl.generation)
                    .map(|(_, l)| l)
            })
        {
            let set = self.current_desc_set;
            if set != vk::DescriptorSet::null() {
                let sets = [set];
                unsafe {
                    self.device.cmd_bind_descriptor_sets(
                        self.cmd,
                        vk::PipelineBindPoint::GRAPHICS,
                        layout,
                        fs,
                        &sets,
                        do_,
                    );
                }
            }
        }
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
    ) {
        if let Some(&buf) = self.buffer_cache.get(buffer.index as usize).and_then(|s| {
            s.as_ref()
                .filter(|(g, _)| *g == buffer.generation)
                .map(|(_, b)| b)
        }) {
            // SAFETY: command buffer is in recording state; `buf` is a valid
            // VkBuffer with INDIRECT_BUFFER usage; draw_count, offset and stride
            // are within the buffer's bounds.
            unsafe {
                self.device
                    .cmd_draw_indexed_indirect(self.cmd, buf, offset, draw_count, stride);
            }
        }
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
        unsafe {
            self.device.cmd_end_render_pass(self.cmd);
        }
    }

    fn hdr_barrier(&mut self) {
        if self.hdr_image == vk::Image::null() {
            return;
        }
        // Transition HDR color image from COLOR_ATTACHMENT_OPTIMAL → SHADER_READ_ONLY_OPTIMAL
        // so the tone-mapping pass can sample it as a texture.
        let barrier = vk::ImageMemoryBarrier::default()
            .image(self.hdr_image)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            })
            .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
            .dst_access_mask(vk::AccessFlags::SHADER_READ)
            .old_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
        // SAFETY: command buffer is in recording state; barrier references
        // a valid HDR color image handle that outlives this encoder.
        unsafe {
            self.device.cmd_pipeline_barrier(
                self.cmd,
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                vk::PipelineStageFlags::FRAGMENT_SHADER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[barrier],
            );
        }
    }

    fn shadow_barrier(&mut self) {
        if self.shadow_map == vk::Image::null() {
            return;
        }
        // Barrier covers all 3 cascade layers (CSM).
        let barrier = vk::ImageMemoryBarrier::default()
            .image(self.shadow_map)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::DEPTH,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 3,
            })
            .src_access_mask(vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE)
            .dst_access_mask(vk::AccessFlags::SHADER_READ)
            .old_layout(vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL)
            .new_layout(vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL);
        // SAFETY: command buffer is in recording state; barrier references
        // a valid shadow image handle that outlives this encoder.
        unsafe {
            self.device.cmd_pipeline_barrier(
                self.cmd,
                vk::PipelineStageFlags::LATE_FRAGMENT_TESTS,
                vk::PipelineStageFlags::FRAGMENT_SHADER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[barrier],
            );
        }
    }
}
