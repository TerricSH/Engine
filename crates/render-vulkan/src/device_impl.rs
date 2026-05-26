//! VulkanDevice — implements `render_core::Device` plus MVP triangle path.

use std::collections::HashMap;
use std::ffi::CStr;

use ash::vk;
use ash::Device as AshDevice;

use render_core::{
    self, AdapterInfo, BackendKind, BufferDescriptor, BufferHandle,
    CommandEncoder as CmdEncoderTrait, FramebufferDescriptor, FramebufferHandle, IndexFormat,
    PipelineDescriptor, PipelineHandle, PipelineLayoutDescriptor, PipelineLayoutHandle,
    RenderPassDescriptor, RenderPassHandle, RendererStatistics, ResourceLimits, ShaderFormat,
    ShaderModuleDescriptor, ShaderModuleHandle, SurfaceDescriptor, SurfaceHandle,
    SwapchainDescriptor, SwapchainHandle, TextureDescriptor, TextureFormat, TextureHandle,
};

use crate::device::Device as VkLogicalDevice;
use crate::error::{VkResult, VulkanError};
use crate::instance::Instance;
use crate::surface::Surface;

unsafe impl Send for VulkanDevice {}
unsafe impl Sync for VulkanDevice {}

// ============================================================================
// Handle slab
// ============================================================================

struct BufEntry {
    buffer: vk::Buffer,
    allocator: crate::device::SharedAllocator,
    allocation: Option<gpu_allocator::vulkan::Allocation>,
}

impl Drop for BufEntry {
    fn drop(&mut self) {
        if let Some(a) = self.allocation.take() {
            let _ = self.allocator.borrow_mut().free(a);
        }
    }
}

struct Slab<T> {
    slots: Vec<Option<(u32, T)>>,
}
impl<T> Slab<T> {
    fn new() -> Self {
        Self { slots: Vec::new() }
    }
    fn insert(&mut self, v: T) -> (u32, u32) {
        for (i, s) in self.slots.iter_mut().enumerate() {
            if s.is_none() {
                *s = Some((1, v));
                return (i as u32, 1);
            }
        }
        let i = self.slots.len();
        self.slots.push(Some((1, v)));
        (i as u32, 1)
    }
    fn get(&self, idx: u32, gen: u32) -> Option<&T> {
        self.slots
            .get(idx as usize)
            .and_then(|s| s.as_ref().filter(|(g, _)| *g == gen).map(|(_, v)| v))
    }
    fn get_mut(&mut self, idx: u32, gen: u32) -> Option<&mut T> {
        self.slots
            .get_mut(idx as usize)
            .and_then(|s| s.as_mut().filter(|(g, _)| *g == gen).map(|(_, v)| v))
    }
}

// ============================================================================
// Frame sync
// ============================================================================

struct FrameSync {
    image_available: vk::Semaphore,
    render_finished: vk::Semaphore,
    in_flight_fence: vk::Fence,
    command_pool: vk::CommandPool,
    command_buffer: vk::CommandBuffer,
}

// ============================================================================
// VkCmdEncoder
// ============================================================================

struct PipeEntry {
    pipeline: vk::Pipeline,
}

struct PlEntry {
    layout: vk::PipelineLayout,
    device: AshDevice,
}

pub struct VkCmdEncoder {
    device: AshDevice,
    cmd: vk::CommandBuffer,
    pipelines: *const Slab<PipeEntry>,
    buffers: *const Slab<BufEntry>,
    render_passes: *const Slab<vk::RenderPass>,
    framebuffers: *const Slab<vk::Framebuffer>,
    pipeline_layouts: *const Slab<PlEntry>,
    // Per-frame descriptor set (set=0 per FD-041), set by begin_frame
    current_desc_set: vk::DescriptorSet,
}
unsafe impl Send for VkCmdEncoder {}

impl CmdEncoderTrait for VkCmdEncoder {
    fn begin_render_pass(
        &mut self,
        rp: RenderPassHandle,
        fb: FramebufferHandle,
        area: (u32, u32, u32, u32),
        clear: [f32; 4],
        _depth: Option<f32>,
    ) {
        let rp_entry = unsafe { &*self.render_passes }.get(rp.index, rp.generation);
        let fb_entry = unsafe { &*self.framebuffers }.get(fb.index, fb.generation);
        if let (Some(&rp_), Some(&fb_)) = (rp_entry, fb_entry) {
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
        let entry = unsafe { &*self.pipelines }.get(p.index, p.generation);
        if let Some(e) = entry {
            unsafe {
                self.device.cmd_bind_pipeline(
                    self.cmd,
                    vk::PipelineBindPoint::GRAPHICS,
                    e.pipeline,
                );
            }
        }
    }
    fn bind_vertex_buffers(&mut self, bufs: &[BufferHandle], offs: &[u64]) {
        let v: Vec<vk::Buffer> = bufs
            .iter()
            .filter_map(|h| {
                unsafe { &*self.buffers }
                    .get(h.index, h.generation)
                    .map(|e| e.buffer)
            })
            .collect();
        if !v.is_empty() {
            unsafe {
                self.device.cmd_bind_vertex_buffers(self.cmd, 0, &v, offs);
            }
        }
    }
    fn bind_index_buffer(&mut self, buf: BufferHandle, o: u64, f: IndexFormat) {
        if let Some(e) = unsafe { &*self.buffers }.get(buf.index, buf.generation) {
            unsafe {
                self.device.cmd_bind_index_buffer(
                    self.cmd,
                    e.buffer,
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
        if let Some(entry) = unsafe { &*self.pipeline_layouts }.get(pl.index, pl.generation) {
            if self.current_desc_set != vk::DescriptorSet::null() {
                let sets = [self.current_desc_set];
                unsafe {
                    self.device.cmd_bind_descriptor_sets(
                        self.cmd,
                        vk::PipelineBindPoint::GRAPHICS,
                        entry.layout,
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
    fn push_constants(&mut self, pl: PipelineLayoutHandle, sf: u32, off: u32, data: &[u8]) {
        if let Some(e) = unsafe { &*self.pipeline_layouts }.get(pl.index, pl.generation) {
            unsafe {
                self.device.cmd_push_constants(
                    self.cmd,
                    e.layout,
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
}

// ============================================================================
// VulkanDevice
// ============================================================================

pub struct VulkanDevice {
    instance: Option<Instance>,
    surface: Option<Surface>,
    adapter: crate::adapter::AdapterSelection,
    logical_device: VkLogicalDevice,

    swapchain: Option<crate::swapchain::Swapchain>,
    swapchain_extent: vk::Extent2D,
    window_width: u32,
    window_height: u32,
    minimized: bool,

    // MVP triangle
    mvp_framebuffers: Vec<vk::Framebuffer>,
    mvp_rp: Option<vk::RenderPass>,
    mvp_pipeline_layout: Option<vk::PipelineLayout>,
    mvp_pipeline: Option<vk::Pipeline>,
    mvp_vert_spv: Option<&'static [u8]>,
    mvp_frag_spv: Option<&'static [u8]>,

    frame_sync: Vec<FrameSync>,
    current_frame: usize,
    cached_adapter_info: AdapterInfo,

    // Phase 2: handle tables
    buffers: Slab<BufEntry>,
    pipelines: Slab<PipeEntry>,
    render_passes: Slab<vk::RenderPass>,
    framebuffers: Slab<vk::Framebuffer>,
    pipeline_layouts: Slab<PlEntry>,

    // Render pass metadata
    rp_has_depth: HashMap<u32, bool>,

    // Per-frame descriptor infrastructure (set=0 per FD-041)
    desc_set_layout_0: Option<vk::DescriptorSetLayout>,
    desc_pool: Option<vk::DescriptorPool>,
    frame_desc_sets: Vec<vk::DescriptorSet>,
    frame_ubos: Vec<vk::Buffer>,
    ubo_size: vk::DeviceSize,
    ubo_allocations: Vec<gpu_allocator::vulkan::Allocation>,
    ubo_alignment: u64,

    // Depth texture (matching swapchain size)
    depth_image: Option<vk::Image>,
    depth_image_view: Option<vk::ImageView>,
    depth_allocation: Option<gpu_allocator::vulkan::Allocation>,
}

impl VulkanDevice {
    pub fn new(
        display_handle: raw_window_handle::RawDisplayHandle,
        window_handle: raw_window_handle::RawWindowHandle,
        width: u32,
        height: u32,
        enable_validation: bool,
    ) -> Result<Self, VulkanError> {
        let instance = unsafe { Instance::new(display_handle, enable_validation) }?;
        let surface = unsafe {
            Surface::new(
                &instance.entry,
                &instance.instance,
                display_handle,
                window_handle,
            )
        }?;
        let adapter = unsafe {
            crate::adapter::select(&instance.instance, &surface.loader, surface.surface)
        }?;
        let ld = unsafe { VkLogicalDevice::new(&instance.instance, &adapter) }?;
        let name = unsafe { CStr::from_ptr(adapter.properties.device_name.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        let info = AdapterInfo {
            backend: BackendKind::Vulkan,
            name,
            vendor_id: Some(adapter.properties.vendor_id),
            device_id: Some(adapter.properties.device_id),
            driver_version: None,
            capabilities: render_core::BackendCapabilities {
                max_texture_dimension_2d: 16384,
                max_color_attachments: 8,
                supports_swapchain: true,
                supports_timestamps: false,
                supports_debug_markers: enable_validation,
                supported_shader_formats: vec![ShaderFormat::SpirV],
                supported_surface_formats: vec![TextureFormat::Bgra8Unorm],
                limits: ResourceLimits {
                    max_buffer_bytes: u64::MAX,
                    max_texture_array_layers: 256,
                    max_bind_groups: 4,
                    max_vertex_attributes: 16,
                    max_color_attachments: 8,
                    max_sample_count: 1,
                },
            },
        };
        Ok(Self {
            instance: Some(instance),
            surface: Some(surface),
            adapter,
            logical_device: ld,
            swapchain: None,
            swapchain_extent: vk::Extent2D {
                width: width.max(1),
                height: height.max(1),
            },
            window_width: width.max(1),
            window_height: height.max(1),
            minimized: width == 0 || height == 0,
            mvp_framebuffers: Vec::new(),
            mvp_rp: None,
            mvp_pipeline_layout: None,
            mvp_pipeline: None,
            mvp_vert_spv: None,
            mvp_frag_spv: None,
            frame_sync: Vec::new(),
            current_frame: 0,
            cached_adapter_info: info,
            buffers: Slab::new(),
            pipelines: Slab::new(),
            render_passes: Slab::new(),
            framebuffers: Slab::new(),
            pipeline_layouts: Slab::new(),
            rp_has_depth: HashMap::new(),
            desc_set_layout_0: None,
            desc_pool: None,
            frame_desc_sets: Vec::new(),
            frame_ubos: Vec::new(),
            ubo_size: 256,
            ubo_allocations: Vec::new(),
            ubo_alignment: 256,
            depth_image: None,
            depth_image_view: None,
            depth_allocation: None,
        })
    }

    pub fn set_mvp_shaders(&mut self, vert: &'static [u8], frag: &'static [u8]) {
        self.mvp_vert_spv = Some(vert);
        self.mvp_frag_spv = Some(frag);
    }
    pub fn resize(&mut self, w: u32, h: u32) {
        self.window_width = w.max(1);
        self.window_height = h.max(1);
        self.minimized = w == 0 || h == 0;
        let _ = unsafe { self.logical_device.device.device_wait_idle() };
        self.destroy_mvp();
    }
    pub fn wait_idle(&self) {
        let _ = unsafe { self.logical_device.device.device_wait_idle() };
    }

    // --- Phase 1: render_triangle_frame ---

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

    // --- Phase 2 helpers ---

    fn ensure_sc(&mut self) -> VkResult<()> {
        if self.swapchain.is_none() {
            match unsafe {
                crate::swapchain::Swapchain::new(
                    &self.instance.as_ref().unwrap().instance,
                    self.logical_device.device.clone(),
                    self.adapter.physical_device,
                    self.logical_device.queue_family_index,
                    &self.surface.as_ref().unwrap().loader,
                    self.surface.as_ref().unwrap().surface,
                    self.window_width,
                    self.window_height,
                )
            } {
                Ok(sc) => {
                    self.swapchain_extent = sc.extent;
                    self.swapchain = Some(sc);
                    // Create depth texture matching swapchain
                    self.create_depth_texture()?;
                    // Create descriptor set infrastructure
                    self.create_descriptor_infra()?;
                }
                Err(VulkanError::SurfaceMinimized) => {
                    self.minimized = true;
                    return Err(VulkanError::SurfaceMinimized);
                }
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    fn acquire(&self, fi: usize) -> VkResult<(u32, bool)> {
        let sc = self.swapchain.as_ref().unwrap();
        let f = &self.frame_sync[fi];
        unsafe {
            self.logical_device
                .device
                .wait_for_fences(&[f.in_flight_fence], true, u64::MAX)
                .map_err(|r| VulkanError::vk("wf", r))?;
        }
        let (ii, sub) = unsafe {
            sc.loader.acquire_next_image(
                sc.swapchain,
                u64::MAX,
                f.image_available,
                vk::Fence::null(),
            )
        }
        .map_err(|r| {
            if r == vk::Result::ERROR_OUT_OF_DATE_KHR {
                VulkanError::SwapchainOutOfDate
            } else {
                VulkanError::vk("aq", r)
            }
        })?;
        unsafe {
            self.logical_device
                .device
                .reset_fences(&[f.in_flight_fence])
                .map_err(|r| VulkanError::vk("rf", r))?;
        }
        Ok((ii, sub))
    }

    fn begin_cb(&self, fi: usize) -> VkResult<()> {
        let f = &self.frame_sync[fi];
        unsafe {
            self.logical_device
                .device
                .reset_command_buffer(f.command_buffer, vk::CommandBufferResetFlags::empty())
                .map_err(|r| VulkanError::vk("rcb", r))?;
            self.logical_device
                .device
                .begin_command_buffer(
                    f.command_buffer,
                    &vk::CommandBufferBeginInfo::default()
                        .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
                )
                .map_err(|r| VulkanError::vk("bcb", r))?;
        }
        Ok(())
    }

    fn record_triangle(&self, fi: usize, ii: u32) -> VkResult<()> {
        self.begin_cb(fi)?;
        let d = &self.logical_device.device;
        let f = &self.frame_sync[fi];
        let sc = self.swapchain.as_ref().unwrap();
        let rp = self.mvp_rp.unwrap();
        let pl = self.mvp_pipeline.unwrap();
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

    fn submit_and_present(&self, fi: usize, ii: u32) -> VkResult<bool> {
        let d = &self.logical_device.device;
        let f = &self.frame_sync[fi];
        let sc = self.swapchain.as_ref().unwrap();
        unsafe {
            d.end_command_buffer(f.command_buffer)
                .map_err(|r| VulkanError::vk("ecb", r))?;
        }
        let ws = [f.image_available];
        let wst = [vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT];
        let cbs = [f.command_buffer];
        let ss = [f.render_finished];
        let si = vk::SubmitInfo::default()
            .wait_semaphores(&ws)
            .wait_dst_stage_mask(&wst)
            .command_buffers(&cbs)
            .signal_semaphores(&ss);
        unsafe {
            d.queue_submit(self.logical_device.queue, &[si], f.in_flight_fence)
                .map_err(|r| VulkanError::vk("qs", r))?;
        }
        let sca = [sc.swapchain];
        let ia = [ii];
        match unsafe {
            sc.loader.queue_present(
                self.logical_device.queue,
                &vk::PresentInfoKHR::default()
                    .wait_semaphores(&ss)
                    .swapchains(&sca)
                    .image_indices(&ia),
            )
        } {
            Ok(false) => Ok(false),
            Ok(true) => Ok(true),
            Err(r) if r == vk::Result::ERROR_OUT_OF_DATE_KHR || r == vk::Result::SUBOPTIMAL_KHR => {
                Ok(true)
            }
            Err(r) => Err(VulkanError::vk("qp", r)),
        }
    }

    fn destroy_mvp(&mut self) {
        self.destroy_descriptor_infra();
        self.destroy_depth_texture();
        let d = &self.logical_device.device;
        for fb in self.mvp_framebuffers.drain(..) {
            unsafe {
                d.destroy_framebuffer(fb, None);
            }
        }
        if let Some(p) = self.mvp_pipeline.take() {
            unsafe {
                d.destroy_pipeline(p, None);
            }
        }
        if let Some(l) = self.mvp_pipeline_layout.take() {
            unsafe {
                d.destroy_pipeline_layout(l, None);
            }
        }
        if let Some(rp) = self.mvp_rp.take() {
            unsafe {
                d.destroy_render_pass(rp, None);
            }
        }
        self.swapchain = None;
    }

    fn build_mvp(&mut self) -> VkResult<()> {
        let vert = self
            .mvp_vert_spv
            .ok_or(VulkanError::MissingShader("mvp.vert"))?;
        let frag = self
            .mvp_frag_spv
            .ok_or(VulkanError::MissingShader("mvp.frag"))?;
        let fmt = self.swapchain.as_ref().unwrap().format;
        let ext = self.swapchain_extent;
        let d = &self.logical_device.device;
        let vm = unsafe { mk_sm(d, vert)? };
        let fm = unsafe { mk_sm(d, frag)? };
        let at = vk::AttachmentDescription::default()
            .format(fmt)
            .samples(vk::SampleCountFlags::TYPE_1)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::STORE)
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .final_layout(vk::ImageLayout::PRESENT_SRC_KHR);
        let cr = vk::AttachmentReference::default()
            .attachment(0)
            .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);
        let crs = [cr];
        let atts = [at];
        let sp = vk::SubpassDescription::default()
            .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
            .color_attachments(&crs);
        let sps = [sp];
        let dep = default_dep();
        let deps = [dep];
        let rpi = vk::RenderPassCreateInfo::default()
            .attachments(&atts)
            .subpasses(&sps)
            .dependencies(&deps);
        let rp =
            unsafe { d.create_render_pass(&rpi, None) }.map_err(|r| VulkanError::vk("crp", r))?;
        let pli = vk::PipelineLayoutCreateInfo::default();
        let pl = unsafe { d.create_pipeline_layout(&pli, None) }
            .map_err(|r| VulkanError::vk("cpl", r))?;
        let main = c"main";
        let sr = [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(vm)
                .name(main),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(fm)
                .name(main),
        ];
        let vi = vk::PipelineVertexInputStateCreateInfo::default();
        let ia = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST);
        let vs = vk::PipelineViewportStateCreateInfo::default()
            .viewport_count(1)
            .scissor_count(1);
        let rs = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(vk::CullModeFlags::NONE)
            .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
            .line_width(1.0);
        let ms = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);
        let cba = [vk::PipelineColorBlendAttachmentState::default()
            .color_write_mask(vk::ColorComponentFlags::RGBA)
            .blend_enable(false)];
        let cb = vk::PipelineColorBlendStateCreateInfo::default()
            .logic_op_enable(false)
            .attachments(&cba);
        let dyns = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let ds = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dyns);
        let pinfo = vk::GraphicsPipelineCreateInfo::default()
            .stages(&sr)
            .vertex_input_state(&vi)
            .input_assembly_state(&ia)
            .viewport_state(&vs)
            .rasterization_state(&rs)
            .multisample_state(&ms)
            .color_blend_state(&cb)
            .dynamic_state(&ds)
            .layout(pl)
            .render_pass(rp)
            .subpass(0);
        let p = unsafe { d.create_graphics_pipelines(vk::PipelineCache::null(), &[pinfo], None) }
            .map_err(|(_, r)| VulkanError::vk("cgp", r))?[0];
        unsafe {
            d.destroy_shader_module(vm, None);
            d.destroy_shader_module(fm, None);
        }
        let mut fbs = Vec::new();
        for iv in &self.swapchain.as_ref().unwrap().image_views {
            let iva = [*iv];
            fbs.push(
                unsafe {
                    d.create_framebuffer(
                        &vk::FramebufferCreateInfo::default()
                            .render_pass(rp)
                            .attachments(&iva)
                            .width(ext.width)
                            .height(ext.height)
                            .layers(1),
                        None,
                    )
                }
                .map_err(|r| VulkanError::vk("cfb", r))?,
            );
        }
        self.mvp_rp = Some(rp);
        self.mvp_pipeline_layout = Some(pl);
        self.mvp_pipeline = Some(p);
        self.mvp_framebuffers = fbs;
        Ok(())
    }

    // --- Depth texture management ---

    fn create_depth_texture(&mut self) -> VkResult<()> {
        let d = &self.logical_device.device;
        let extent = self
            .swapchain
            .as_ref()
            .map(|s| s.extent)
            .unwrap_or(self.swapchain_extent);
        if extent.width == 0 || extent.height == 0 {
            return Ok(());
        }

        let image_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(vk::Format::D32_SFLOAT)
            .extent(vk::Extent3D {
                width: extent.width,
                height: extent.height,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        let image = unsafe { d.create_image(&image_info, None) }
            .map_err(|r| VulkanError::vk("create_depth_image", r))?;
        let req = unsafe { d.get_image_memory_requirements(image) };
        let allocator = self.logical_device.allocator();
        let allocation = allocator
            .borrow_mut()
            .allocate(&gpu_allocator::vulkan::AllocationCreateDesc {
                name: "depth-buffer",
                requirements: req,
                location: gpu_allocator::MemoryLocation::GpuOnly,
                linear: false,
                allocation_scheme: gpu_allocator::vulkan::AllocationScheme::GpuAllocatorManaged,
            })
            .map_err(|e| VulkanError::Allocation(e.to_string()))?;
        unsafe { d.bind_image_memory(image, allocation.memory(), allocation.offset()) }
            .map_err(|r| VulkanError::vk("bind_depth_image", r))?;

        let view_info = vk::ImageViewCreateInfo::default()
            .image(image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(vk::Format::D32_SFLOAT)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::DEPTH,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            });
        let image_view = unsafe { d.create_image_view(&view_info, None) }
            .map_err(|r| VulkanError::vk("create_depth_image_view", r))?;

        self.depth_image = Some(image);
        self.depth_image_view = Some(image_view);
        self.depth_allocation = Some(allocation);
        Ok(())
    }

    fn destroy_depth_texture(&mut self) {
        let d = &self.logical_device.device;
        if let Some(iv) = self.depth_image_view.take() {
            unsafe {
                d.destroy_image_view(iv, None);
            }
        }
        if let Some(img) = self.depth_image.take() {
            unsafe {
                d.destroy_image(img, None);
            }
        }
        if let Some(a) = self.depth_allocation.take() {
            let _ = self.logical_device.allocator().borrow_mut().free(a);
        }
    }

    fn depth_view(&self) -> Option<vk::ImageView> {
        self.depth_image_view
    }

    // --- Descriptor infrastructure (set=0 per-frame UBO per FD-041) ---

    fn create_descriptor_infra(&mut self) -> VkResult<()> {
        if self.desc_set_layout_0.is_some() {
            return Ok(());
        } // already created
        let d = &self.logical_device.device;

        // Descriptor set layout: binding 0 = VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER at set=0
        let bindings = [vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT)];
        let layout_info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings);
        let ds_layout = unsafe { d.create_descriptor_set_layout(&layout_info, None) }
            .map_err(|r| VulkanError::vk("create_ds_layout", r))?;

        // Descriptor pool: 2 sets (double buffering), 2 UBO descriptors
        let pool_sizes = [vk::DescriptorPoolSize {
            ty: vk::DescriptorType::UNIFORM_BUFFER,
            descriptor_count: 2,
        }];
        let pool_info = vk::DescriptorPoolCreateInfo::default()
            .max_sets(2)
            .pool_sizes(&pool_sizes);
        let pool = unsafe { d.create_descriptor_pool(&pool_info, None) }
            .map_err(|r| VulkanError::vk("create_ds_pool", r))?;

        // Allocate 2 descriptor sets
        let layouts = [ds_layout, ds_layout];
        let alloc_info = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(pool)
            .set_layouts(&layouts);
        let desc_sets = unsafe { d.allocate_descriptor_sets(&alloc_info) }
            .map_err(|r| VulkanError::vk("alloc_ds", r))?;

        // Create 2 UBO buffers (CpuToGpu, sized to ubo_size)
        let mut ubos = Vec::with_capacity(2);
        let mut allocs = Vec::with_capacity(2);
        let allocator = self.logical_device.allocator();
        for i in 0..2 {
            let bi = vk::BufferCreateInfo::default()
                .size(self.ubo_size)
                .usage(vk::BufferUsageFlags::UNIFORM_BUFFER)
                .sharing_mode(vk::SharingMode::EXCLUSIVE);
            let buf = unsafe { d.create_buffer(&bi, None) }
                .map_err(|r| VulkanError::vk("create_ubo", r))?;
            let req = unsafe { d.get_buffer_memory_requirements(buf) };
            self.ubo_alignment = req.alignment as u64;
            let allocation = allocator
                .borrow_mut()
                .allocate(&gpu_allocator::vulkan::AllocationCreateDesc {
                    name: &format!("frame-ubo-{i}"),
                    requirements: req,
                    location: gpu_allocator::MemoryLocation::CpuToGpu,
                    linear: true,
                    allocation_scheme: gpu_allocator::vulkan::AllocationScheme::GpuAllocatorManaged,
                })
                .map_err(|e| VulkanError::Allocation(e.to_string()))?;
            unsafe { d.bind_buffer_memory(buf, allocation.memory(), allocation.offset()) }
                .map_err(|r| VulkanError::vk("bind_ubo", r))?;
            ubos.push(buf);
            allocs.push(allocation);
        }

        // Write descriptor sets
        for (i, ds) in desc_sets.iter().enumerate() {
            let buf_info = [vk::DescriptorBufferInfo::default()
                .buffer(ubos[i])
                .offset(0)
                .range(self.ubo_size)];
            let writes = [vk::WriteDescriptorSet::default()
                .dst_set(*ds)
                .dst_binding(0)
                .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                .buffer_info(&buf_info)];
            unsafe {
                d.update_descriptor_sets(&writes, &[]);
            }
        }

        self.desc_set_layout_0 = Some(ds_layout);
        self.desc_pool = Some(pool);
        self.frame_desc_sets = desc_sets;
        self.frame_ubos = ubos;
        self.ubo_allocations = allocs;
        Ok(())
    }

    /// Get the per-frame descriptor set for the current frame index.
    pub fn frame_descriptor_set(&self, frame_idx: usize) -> Option<vk::DescriptorSet> {
        self.frame_desc_sets.get(frame_idx).copied()
    }

    /// Get the per-frame UBO for the current frame index.
    pub fn frame_ubo(&self, frame_idx: usize) -> Option<vk::Buffer> {
        self.frame_ubos.get(frame_idx).copied()
    }

    /// Write default per-frame UBO data (identity view-proj, a directional light) for the current frame.
    pub fn write_default_ubo(&mut self) {
        let fi = self.current_frame;
        let mut data = Vec::with_capacity(128);
        // View-proj matrix (identity for clip-space rendering)
        for i in 0..16 {
            let v = if i % 5 == 0 { 1.0f32 } else { 0.0f32 };
            data.extend_from_slice(&v.to_ne_bytes());
        }
        // Light direction (normalized, pointing down-left)
        for v in &[0.5f32, -0.707f32, 0.5f32, 0.0f32] {
            data.extend_from_slice(&v.to_ne_bytes());
        }
        // Light color (bright white, intensity 1.5)
        for v in &[1.5f32, 1.5f32, 1.5f32, 1.5f32] {
            data.extend_from_slice(&v.to_ne_bytes());
        }
        // Ambient (0.15 intensity)
        for v in &[0.15f32, 0.15f32, 0.15f32, 0.15f32] {
            data.extend_from_slice(&v.to_ne_bytes());
        }
        // Camera position (world space)
        for v in &[0.0f32, 0.0f32, 2.0f32, 1.0f32] {
            data.extend_from_slice(&v.to_ne_bytes());
        }
        self.write_ubo(fi, &data, 0);
    }
    /// SAFETY: data must not exceed ubo_size - offset.
    pub fn write_ubo(&mut self, frame_idx: usize, data: &[u8], offset: u64) {
        if let Some(allocation) = self.ubo_allocations.get_mut(frame_idx) {
            if let Some(slice) = allocation.mapped_slice_mut() {
                let start = offset as usize;
                let end = (start + data.len()).min(slice.len());
                slice[start..end].copy_from_slice(&data[..end - start]);
            }
        }
    }

    fn destroy_descriptor_infra(&mut self) {
        let d = &self.logical_device.device;
        for a in self.ubo_allocations.drain(..) {
            let _ = self.logical_device.allocator().borrow_mut().free(a);
        }
        for buf in self.frame_ubos.drain(..) {
            unsafe {
                d.destroy_buffer(buf, None);
            }
        }
        if let Some(pool) = self.desc_pool.take() {
            unsafe {
                d.destroy_descriptor_pool(pool, None);
            }
        }
        if let Some(layout) = self.desc_set_layout_0.take() {
            unsafe {
                d.destroy_descriptor_set_layout(layout, None);
            }
        }
    }

    fn build_frames(&mut self) -> VkResult<()> {
        let d = &self.logical_device.device;
        for _ in 0..2 {
            let cp = unsafe {
                d.create_command_pool(
                    &vk::CommandPoolCreateInfo::default()
                        .queue_family_index(self.logical_device.queue_family_index)
                        .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                    None,
                )
            }
            .map_err(|r| VulkanError::vk("ccp", r))?;
            let cbs = unsafe {
                d.allocate_command_buffers(
                    &vk::CommandBufferAllocateInfo::default()
                        .command_pool(cp)
                        .level(vk::CommandBufferLevel::PRIMARY)
                        .command_buffer_count(1),
                )
            }
            .map_err(|r| VulkanError::vk("acb", r))?;
            let si = vk::SemaphoreCreateInfo::default();
            let ia =
                unsafe { d.create_semaphore(&si, None) }.map_err(|r| VulkanError::vk("cs", r))?;
            let rf =
                unsafe { d.create_semaphore(&si, None) }.map_err(|r| VulkanError::vk("cs", r))?;
            let fl = unsafe {
                d.create_fence(
                    &vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED),
                    None,
                )
            }
            .map_err(|r| VulkanError::vk("cf", r))?;
            self.frame_sync.push(FrameSync {
                image_available: ia,
                render_finished: rf,
                in_flight_fence: fl,
                command_pool: cp,
                command_buffer: cbs[0],
            });
        }
        Ok(())
    }
}

// ============================================================================
// Device trait impl
// ============================================================================

impl render_core::Device for VulkanDevice {
    fn adapter_info(&self) -> &AdapterInfo {
        &self.cached_adapter_info
    }
    fn create_surface(
        &mut self,
        _: &SurfaceDescriptor,
    ) -> Result<SurfaceHandle, render_core::RhiError> {
        Ok(SurfaceHandle::new(0, 1))
    }
    fn create_swapchain(
        &mut self,
        _: &SwapchainDescriptor,
    ) -> Result<SwapchainHandle, render_core::RhiError> {
        Ok(SwapchainHandle::new(1, 1))
    }

    fn create_buffer(
        &mut self,
        desc: &BufferDescriptor,
    ) -> Result<BufferHandle, render_core::RhiError> {
        let d = &self.logical_device;
        let size = desc.size_bytes.max(1);
        let usage = vk::BufferUsageFlags::TRANSFER_DST
            | vk::BufferUsageFlags::VERTEX_BUFFER
            | vk::BufferUsageFlags::INDEX_BUFFER;
        let bi = vk::BufferCreateInfo::default()
            .size(size)
            .usage(usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        let buffer = unsafe { d.device.create_buffer(&bi, None) }.map_err(|r| {
            render_core::RhiError::Backend {
                detail: format!("{r:?}"),
            }
        })?;
        let req = unsafe { d.device.get_buffer_memory_requirements(buffer) };
        let alloc_handle = d.allocator();
        let location = gpu_allocator::MemoryLocation::CpuToGpu;
        let allocation = alloc_handle
            .borrow_mut()
            .allocate(&gpu_allocator::vulkan::AllocationCreateDesc {
                name: desc.debug_label.as_deref().unwrap_or("buf"),
                requirements: req,
                location,
                linear: true,
                allocation_scheme: gpu_allocator::vulkan::AllocationScheme::GpuAllocatorManaged,
            })
            .map_err(|e| render_core::RhiError::Backend {
                detail: format!("{e}"),
            })?;
        if let Err(r) = unsafe {
            d.device
                .bind_buffer_memory(buffer, allocation.memory(), allocation.offset())
        } {
            let _ = alloc_handle.borrow_mut().free(allocation);
            unsafe {
                d.device.destroy_buffer(buffer, None);
            }
            return Err(render_core::RhiError::Backend {
                detail: format!("{r:?}"),
            });
        }
        let (idx, gen) = self.buffers.insert(BufEntry {
            buffer,
            allocator: alloc_handle,
            allocation: Some(allocation),
        });
        Ok(BufferHandle::new(idx, gen))
    }

    fn write_buffer(
        &mut self,
        buf: BufferHandle,
        data: &[u8],
        offset: u64,
    ) -> Result<(), render_core::RhiError> {
        let entry = self
            .buffers
            .get_mut(buf.index, buf.generation)
            .ok_or(render_core::RhiError::InvalidHandle)?;
        let alloc = entry
            .allocation
            .as_mut()
            .ok_or_else(|| render_core::RhiError::Backend {
                detail: "no alloc".into(),
            })?;
        let slice = alloc
            .mapped_slice_mut()
            .ok_or_else(|| render_core::RhiError::Backend {
                detail: "not mapped".into(),
            })?;
        let end = (offset as usize + data.len()).min(slice.len());
        slice[offset as usize..end].copy_from_slice(&data[..end - offset as usize]);
        Ok(())
    }

    fn create_texture(
        &mut self,
        _: &TextureDescriptor,
    ) -> Result<TextureHandle, render_core::RhiError> {
        Err(render_core::RhiError::Backend {
            detail: "not in Phase 2".into(),
        })
    }

    fn create_shader_module(
        &mut self,
        _: &ShaderModuleDescriptor,
    ) -> Result<ShaderModuleHandle, render_core::RhiError> {
        Err(render_core::RhiError::Backend {
            detail: "not in Phase 2".into(),
        })
    }

    fn create_render_pass(
        &mut self,
        desc: &RenderPassDescriptor,
    ) -> Result<RenderPassHandle, render_core::RhiError> {
        let d = &self.logical_device.device;
        let vk_fmt = match desc.color_attachments.first() {
            Some(TextureFormat::Bgra8Unorm) => vk::Format::B8G8R8A8_UNORM,
            Some(TextureFormat::Rgba8Unorm) => vk::Format::R8G8B8A8_UNORM,
            Some(TextureFormat::Rgba16Float) => vk::Format::R16G16B16A16_SFLOAT,
            _ => vk::Format::B8G8R8A8_UNORM,
        };
        let has_depth = desc.depth_stencil_format.is_some();

        // Build render pass using a flat approach to avoid ash lifetime issues
        let (rp, has_depth) = if has_depth {
            let atts = [
                vk::AttachmentDescription::default()
                    .format(vk_fmt)
                    .samples(vk::SampleCountFlags::TYPE_1)
                    .load_op(vk::AttachmentLoadOp::CLEAR)
                    .store_op(vk::AttachmentStoreOp::STORE)
                    .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
                    .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
                    .initial_layout(vk::ImageLayout::UNDEFINED)
                    .final_layout(vk::ImageLayout::PRESENT_SRC_KHR),
                vk::AttachmentDescription::default()
                    .format(vk::Format::D32_SFLOAT)
                    .samples(vk::SampleCountFlags::TYPE_1)
                    .load_op(vk::AttachmentLoadOp::CLEAR)
                    .store_op(vk::AttachmentStoreOp::DONT_CARE)
                    .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
                    .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
                    .initial_layout(vk::ImageLayout::UNDEFINED)
                    .final_layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL),
            ];
            let color_ref = [vk::AttachmentReference::default()
                .attachment(0)
                .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)];
            let depth_ref = vk::AttachmentReference::default()
                .attachment(1)
                .layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL);
            let subpass = vk::SubpassDescription::default()
                .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
                .color_attachments(&color_ref)
                .depth_stencil_attachment(&depth_ref);
            let dep = vk::SubpassDependency::default()
                .src_subpass(vk::SUBPASS_EXTERNAL)
                .dst_subpass(0)
                .src_stage_mask(
                    vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
                        | vk::PipelineStageFlags::LATE_FRAGMENT_TESTS,
                )
                .dst_stage_mask(
                    vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
                        | vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS,
                )
                .dst_access_mask(
                    vk::AccessFlags::COLOR_ATTACHMENT_WRITE
                        | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE,
                );
            let subpasses = [subpass];
            let deps = [dep];
            let rp_info = vk::RenderPassCreateInfo::default()
                .attachments(&atts)
                .subpasses(&subpasses)
                .dependencies(&deps);
            (
                unsafe { d.create_render_pass(&rp_info, None) }.map_err(|r| {
                    render_core::RhiError::Backend {
                        detail: format!("{r:?}"),
                    }
                })?,
                true,
            )
        } else {
            let atts = [vk::AttachmentDescription::default()
                .format(vk_fmt)
                .samples(vk::SampleCountFlags::TYPE_1)
                .load_op(vk::AttachmentLoadOp::CLEAR)
                .store_op(vk::AttachmentStoreOp::STORE)
                .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
                .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
                .initial_layout(vk::ImageLayout::UNDEFINED)
                .final_layout(vk::ImageLayout::PRESENT_SRC_KHR)];
            let color_ref = [vk::AttachmentReference::default()
                .attachment(0)
                .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)];
            let subpass = vk::SubpassDescription::default()
                .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
                .color_attachments(&color_ref);
            let dep = default_dep();
            let subpasses = [subpass];
            let deps = [dep];
            let rp_info = vk::RenderPassCreateInfo::default()
                .attachments(&atts)
                .subpasses(&subpasses)
                .dependencies(&deps);
            (
                unsafe { d.create_render_pass(&rp_info, None) }.map_err(|r| {
                    render_core::RhiError::Backend {
                        detail: format!("{r:?}"),
                    }
                })?,
                false,
            )
        };
        let (idx, gen) = self.render_passes.insert(rp);
        self.rp_has_depth.insert(idx, has_depth);
        Ok(RenderPassHandle::new(idx, gen))
    }

    fn create_framebuffer(
        &mut self,
        desc: &FramebufferDescriptor,
    ) -> Result<FramebufferHandle, render_core::RhiError> {
        let d = &self.logical_device.device;
        let rp = self
            .render_passes
            .get(desc.render_pass.index, desc.render_pass.generation)
            .copied()
            .ok_or(render_core::RhiError::InvalidHandle)?;
        let has_depth = self
            .rp_has_depth
            .get(&desc.render_pass.index)
            .copied()
            .unwrap_or(false);
        let fb = if has_depth {
            let depth_view = self.depth_image_view.unwrap_or(vk::ImageView::null());
            let atts = [vk::ImageView::null(), depth_view];
            let fi = vk::FramebufferCreateInfo::default()
                .render_pass(rp)
                .attachments(&atts)
                .width(desc.width)
                .height(desc.height)
                .layers(1);
            unsafe { d.create_framebuffer(&fi, None) }
        } else {
            let fi = vk::FramebufferCreateInfo::default()
                .render_pass(rp)
                .width(desc.width)
                .height(desc.height)
                .layers(1);
            unsafe { d.create_framebuffer(&fi, None) }
        }
        .map_err(|r| render_core::RhiError::Backend {
            detail: format!("{r:?}"),
        })?;
        let (idx, gen) = self.framebuffers.insert(fb);
        Ok(FramebufferHandle::new(idx, gen))
    }

    fn create_pipeline_layout(
        &mut self,
        desc: &PipelineLayoutDescriptor,
    ) -> Result<PipelineLayoutHandle, render_core::RhiError> {
        let d = &self.logical_device.device;
        let pc_ranges: Vec<vk::PushConstantRange> = desc
            .push_constant_ranges
            .iter()
            .map(|pc| vk::PushConstantRange {
                stage_flags: vk::ShaderStageFlags::from_raw(pc.stage_flags),
                offset: pc.offset,
                size: pc.size,
            })
            .collect();
        // Include the per-frame descriptor set layout (set=0 UBO per FD-041)
        let mut set_layouts: Vec<vk::DescriptorSetLayout> = Vec::new();
        if let Some(dsl) = self.desc_set_layout_0 {
            set_layouts.push(dsl);
        }
        // Also add any from the descriptor
        for bg in &desc.bind_group_layouts {
            // For now, bind_group_layouts from descriptor are ignored
            // since set=0 is always the per-frame layout
        }
        let info = vk::PipelineLayoutCreateInfo::default()
            .set_layouts(&set_layouts)
            .push_constant_ranges(&pc_ranges);
        let layout = unsafe { d.create_pipeline_layout(&info, None) }.map_err(|r| {
            render_core::RhiError::Backend {
                detail: format!("{r:?}"),
            }
        })?;
        let (idx, gen) = self.pipeline_layouts.insert(PlEntry {
            layout,
            device: d.clone(),
        });
        Ok(PipelineLayoutHandle::new(idx, gen))
    }

    fn create_pipeline(
        &mut self,
        desc: &PipelineDescriptor,
    ) -> Result<PipelineHandle, render_core::RhiError> {
        let d = &self.logical_device.device;
        let (vert, frag) = (self.mvp_vert_spv, self.mvp_frag_spv);
        let (vs, fs) = (
            vert.ok_or_else(|| render_core::RhiError::Backend {
                detail: "no vert spv".into(),
            })?,
            frag.ok_or_else(|| render_core::RhiError::Backend {
                detail: "no frag spv".into(),
            })?,
        );
        let vm = (unsafe { mk_sm(d, vs) }).map_err(|e| render_core::RhiError::Backend {
            detail: format!("{e}"),
        })?;
        let fm = (unsafe { mk_sm(d, fs) }).map_err(|e| render_core::RhiError::Backend {
            detail: format!("{e}"),
        })?;
        let main = c"main";
        let sr = [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(vm)
                .name(main),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(fm)
                .name(main),
        ];
        let stride = desc.vertex_layout.stride_bytes;
        let vb = [vk::VertexInputBindingDescription::default()
            .binding(0)
            .stride(stride)
            .input_rate(vk::VertexInputRate::VERTEX)];
        let va: Vec<vk::VertexInputAttributeDescription> = desc
            .vertex_layout
            .attributes
            .iter()
            .map(|a| vk::VertexInputAttributeDescription {
                location: 0,
                binding: 0,
                format: vfmt(&a.format),
                offset: a.offset_bytes,
            })
            .collect();
        let vi = vk::PipelineVertexInputStateCreateInfo::default()
            .vertex_binding_descriptions(&vb)
            .vertex_attribute_descriptions(&va);
        let ia = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST);
        let vs2 = vk::PipelineViewportStateCreateInfo::default()
            .viewport_count(1)
            .scissor_count(1);
        let rs = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(vk::CullModeFlags::NONE)
            .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
            .line_width(1.0);
        let ms = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);
        let cba = [vk::PipelineColorBlendAttachmentState::default()
            .color_write_mask(vk::ColorComponentFlags::RGBA)
            .blend_enable(false)];
        let cb = vk::PipelineColorBlendStateCreateInfo::default()
            .logic_op_enable(false)
            .attachments(&cba);
        let dyns = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let ds = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dyns);
        // Get or create a render pass for this pipeline
        let rp = if let Some(rp_) = self.mvp_rp {
            rp_
        } else {
            // Create a default render pass from the descriptor's render targets
            let fmt = match desc.render_targets.first() {
                Some(TextureFormat::Bgra8Unorm) => vk::Format::B8G8R8A8_UNORM,
                Some(TextureFormat::Rgba8Unorm) => vk::Format::R8G8B8A8_UNORM,
                _ => vk::Format::B8G8R8A8_SRGB,
            };
            let at = vk::AttachmentDescription::default()
                .format(fmt)
                .samples(vk::SampleCountFlags::TYPE_1)
                .load_op(vk::AttachmentLoadOp::CLEAR)
                .store_op(vk::AttachmentStoreOp::STORE)
                .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
                .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
                .initial_layout(vk::ImageLayout::UNDEFINED)
                .final_layout(vk::ImageLayout::PRESENT_SRC_KHR);
            let cr = vk::AttachmentReference::default()
                .attachment(0)
                .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);
            let crs = [cr];
            let atts = [at];
            let sp = vk::SubpassDescription::default()
                .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
                .color_attachments(&crs);
            let sps = [sp];
            let dep = default_dep();
            let deps = [dep];
            let rpi = vk::RenderPassCreateInfo::default()
                .attachments(&atts)
                .subpasses(&sps)
                .dependencies(&deps);
            unsafe { d.create_render_pass(&rpi, None) }.map_err(|r| {
                render_core::RhiError::Backend {
                    detail: format!("{r:?}"),
                }
            })?
        };
        let pll = match desc.pipeline_layout {
            Some(h) => self
                .pipeline_layouts
                .get(h.index, h.generation)
                .map(|e| e.layout)
                .unwrap_or(vk::PipelineLayout::null()),
            None => vk::PipelineLayout::null(),
        };
        // Depth stencil state
        let depth_enabled = desc.depth_state.write_enabled || desc.depth_state.compare.is_some();
        let ds_state = vk::PipelineDepthStencilStateCreateInfo::default()
            .depth_test_enable(depth_enabled)
            .depth_write_enable(desc.depth_state.write_enabled)
            .depth_compare_op(compare_op(&desc.depth_state.compare));
        let pinfo = vk::GraphicsPipelineCreateInfo::default()
            .stages(&sr)
            .vertex_input_state(&vi)
            .input_assembly_state(&ia)
            .viewport_state(&vs2)
            .rasterization_state(&rs)
            .multisample_state(&ms)
            .depth_stencil_state(&ds_state)
            .color_blend_state(&cb)
            .dynamic_state(&ds)
            .layout(pll)
            .render_pass(rp)
            .subpass(0);
        let pipeline =
            unsafe { d.create_graphics_pipelines(vk::PipelineCache::null(), &[pinfo], None) }
                .map_err(|(_, r)| render_core::RhiError::Backend {
                    detail: format!("{r:?}"),
                })?[0];
        unsafe {
            d.destroy_shader_module(vm, None);
            d.destroy_shader_module(fm, None);
        }
        let (idx, gen) = self.pipelines.insert(PipeEntry { pipeline });
        Ok(PipelineHandle::new(idx, gen))
    }

    fn begin_frame(
        &mut self,
        _: SwapchainHandle,
    ) -> Result<(u32, Box<dyn CmdEncoderTrait>), render_core::RhiError> {
        self.ensure_sc()
            .map_err(|e| render_core::RhiError::Backend {
                detail: format!("{e}"),
            })?;
        if self.frame_sync.is_empty() {
            self.build_frames()
                .map_err(|e| render_core::RhiError::Backend {
                    detail: format!("{e}"),
                })?;
        }
        let fi = self.current_frame;
        let (ii, _) = self
            .acquire(fi)
            .map_err(|e| render_core::RhiError::Backend {
                detail: format!("{e}"),
            })?;
        self.begin_cb(fi)
            .map_err(|e| render_core::RhiError::Backend {
                detail: format!("{e}"),
            })?;
        let f = &self.frame_sync[fi];
        let desc_set = self
            .frame_desc_sets
            .get(fi)
            .copied()
            .unwrap_or(vk::DescriptorSet::null());
        let encoder = Box::new(VkCmdEncoder {
            device: self.logical_device.device.clone(),
            cmd: f.command_buffer,
            pipelines: &self.pipelines as *const Slab<PipeEntry>,
            buffers: &self.buffers as *const Slab<BufEntry>,
            render_passes: &self.render_passes as *const Slab<vk::RenderPass>,
            framebuffers: &self.framebuffers as *const Slab<vk::Framebuffer>,
            pipeline_layouts: &self.pipeline_layouts as *const Slab<PlEntry>,
            current_desc_set: desc_set,
        });
        Ok((ii, encoder))
    }

    fn end_frame(
        &mut self,
        _: SwapchainHandle,
        _: Box<dyn CmdEncoderTrait>,
        ii: u32,
    ) -> Result<RendererStatistics, render_core::RhiError> {
        let fi = self.current_frame;
        let subopt =
            self.submit_and_present(fi, ii)
                .map_err(|e| render_core::RhiError::Backend {
                    detail: format!("{e}"),
                })?;
        if subopt {
            let _ = unsafe { self.logical_device.device.device_wait_idle() };
            self.swapchain = None;
        }
        self.current_frame = (fi + 1) % 2;
        Ok(RendererStatistics {
            draw_calls: 1,
            triangles: 0,
            gpu_frame_ms: 0.0,
        })
    }

    fn recreate_swapchain(
        &mut self,
        _: SwapchainHandle,
        w: u32,
        h: u32,
    ) -> Result<(), render_core::RhiError> {
        let _ = unsafe { self.logical_device.device.device_wait_idle() };
        self.window_width = w.max(1);
        self.window_height = h.max(1);
        self.swapchain = None;
        Ok(())
    }

    fn wait_idle(&self) {
        let _ = unsafe { self.logical_device.device.device_wait_idle() };
    }
}

// ============================================================================
// Drop
// ============================================================================

impl Drop for VulkanDevice {
    fn drop(&mut self) {
        let _ = unsafe { self.logical_device.device.device_wait_idle() };
        let d = &self.logical_device.device;
        for fb in self.mvp_framebuffers.drain(..) {
            unsafe {
                d.destroy_framebuffer(fb, None);
            }
        }
        if let Some(p) = self.mvp_pipeline.take() {
            unsafe {
                d.destroy_pipeline(p, None);
            }
        }
        if let Some(l) = self.mvp_pipeline_layout.take() {
            unsafe {
                d.destroy_pipeline_layout(l, None);
            }
        }
        if let Some(rp) = self.mvp_rp.take() {
            unsafe {
                d.destroy_render_pass(rp, None);
            }
        }
        for fs in self.frame_sync.drain(..) {
            unsafe {
                d.destroy_fence(fs.in_flight_fence, None);
                d.destroy_semaphore(fs.image_available, None);
                d.destroy_semaphore(fs.render_finished, None);
                d.destroy_command_pool(fs.command_pool, None);
            }
        }
        for s in self.pipelines.slots.drain(..) {
            if let Some((_, e)) = s {
                unsafe {
                    d.destroy_pipeline(e.pipeline, None);
                }
            }
        }
        for s in self.buffers.slots.drain(..) {
            if let Some((_, mut e)) = s {
                unsafe {
                    d.destroy_buffer(e.buffer, None);
                }
                if let Some(a) = e.allocation.take() {
                    let _ = e.allocator.borrow_mut().free(a);
                }
            }
        }
        for s in self.render_passes.slots.drain(..) {
            if let Some((_, rp)) = s {
                unsafe {
                    d.destroy_render_pass(rp, None);
                }
            }
        }
        for s in self.framebuffers.slots.drain(..) {
            if let Some((_, fb)) = s {
                unsafe {
                    d.destroy_framebuffer(fb, None);
                }
            }
        }
        for s in self.pipeline_layouts.slots.drain(..) {
            if let Some((_, e)) = s {
                unsafe {
                    d.destroy_pipeline_layout(e.layout, None);
                }
            }
        }
        self.destroy_descriptor_infra();
        self.destroy_depth_texture();
        drop(self.swapchain.take());
        drop(self.surface.take());
        drop(self.instance.take());
    }
}

// ============================================================================
// Helpers
// ============================================================================

fn default_dep() -> vk::SubpassDependency {
    vk::SubpassDependency::default()
        .src_subpass(vk::SUBPASS_EXTERNAL)
        .dst_subpass(0)
        .src_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
        .dst_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
        .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
}

fn vfmt(f: &str) -> vk::Format {
    match f {
        "float32x2" => vk::Format::R32G32_SFLOAT,
        "float32x3" => vk::Format::R32G32B32_SFLOAT,
        "float32x4" => vk::Format::R32G32B32A32_SFLOAT,
        _ => vk::Format::R32G32B32_SFLOAT,
    }
}

fn compare_op(s: &Option<String>) -> vk::CompareOp {
    match s.as_deref() {
        Some("less") => vk::CompareOp::LESS,
        Some("equal") => vk::CompareOp::EQUAL,
        Some("lequal") => vk::CompareOp::LESS_OR_EQUAL,
        Some("greater") => vk::CompareOp::GREATER,
        Some("always") => vk::CompareOp::ALWAYS,
        _ => vk::CompareOp::ALWAYS,
    }
}

unsafe fn mk_sm(d: &AshDevice, spv: &[u8]) -> VkResult<vk::ShaderModule> {
    if spv.is_empty() {
        return Err(VulkanError::MissingShader(""));
    }
    if spv.len() % 4 != 0 {
        return Err(VulkanError::Loader(format!("len {}", spv.len())));
    }
    let mut code = vec![0u32; spv.len() / 4];
    for (i, c) in spv.chunks_exact(4).enumerate() {
        code[i] = u32::from_ne_bytes([c[0], c[1], c[2], c[3]]);
    }
    unsafe { d.create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&code), None) }
        .map_err(|r| VulkanError::vk("sm", r))
}
