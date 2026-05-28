//! VulkanDevice — implements `render_core::Device` plus MVP triangle path.

pub(crate) mod depth;
pub(crate) mod descriptor;
pub(crate) mod encoder;
pub(crate) mod slab;

use std::collections::HashMap;
use std::ffi::CStr;
use std::mem::ManuallyDrop;

use ash::vk;
use ash::Device as AshDevice;

use render_core::{
    self, AdapterInfo, BackendKind, BufferDescriptor, BufferHandle,
    CommandEncoder as CmdEncoderTrait, FramebufferDescriptor, FramebufferHandle,
    PipelineDescriptor, PipelineHandle, PipelineLayoutDescriptor, PipelineLayoutHandle,
    RenderPassDescriptor, RenderPassHandle, RendererStatistics, ResourceLimits, ShaderFormat,
    ShaderModuleDescriptor, ShaderModuleHandle, SurfaceDescriptor, SurfaceHandle,
    SwapchainDescriptor, SwapchainHandle, TextureDescriptor, TextureFormat, TextureHandle,
};

use crate::device::Device as VkLogicalDevice;
use crate::error::{VkResult, VulkanError};
use crate::instance::Instance;
use crate::surface::Surface;

use self::encoder::VkCmdEncoder;
use self::slab::{BufEntry, FrameSync, PipeEntry, PlEntry, Slab};

// SAFETY: all fields are Send-safe: Vulkan handles are integers or wrapped in
// ManuallyDrop which is Send; Instance/Surface are Send; allocator Mutex is Send.
unsafe impl Send for VulkanDevice {}
// SAFETY: all fields are Sync-safe: mutable access requires &mut self; Vulkan
// handles are integers; allocator Mutex provides interior mutability safely.
unsafe impl Sync for VulkanDevice {}

// ============================================================================
// VulkanDevice
// ============================================================================

pub struct VulkanDevice {
    // IMPORTANT: Drop order follows field declaration order.
    // logical_device MUST be dropped BEFORE instance/surface
    // (Vulkan spec: VkDevice destroyed before VkInstance).
    pub(crate) logical_device: ManuallyDrop<VkLogicalDevice>,
    pub(crate) instance: Option<Instance>,
    pub(crate) surface: Option<Surface>,
    pub(crate) adapter: crate::adapter::AdapterSelection,

    pub(crate) swapchain: Option<crate::swapchain::Swapchain>,
    pub(crate) swapchain_extent: vk::Extent2D,
    pub(crate) window_width: u32,
    pub(crate) window_height: u32,
    pub(crate) minimized: bool,

    // MVP triangle
    pub(crate) mvp_framebuffers: Vec<vk::Framebuffer>,
    pub(crate) mvp_rp: Option<vk::RenderPass>,
    pub(crate) mvp_pipeline_layout: Option<vk::PipelineLayout>,
    pub(crate) mvp_pipeline: Option<vk::Pipeline>,
    pub(crate) mvp_vert_spv: Option<&'static [u8]>,
    pub(crate) mvp_frag_spv: Option<&'static [u8]>,

    // Model rendering pipeline (forward shaders + vertex input state)
    pub(crate) model_pipeline: Option<vk::Pipeline>,
    pub(crate) model_pipeline_layout: Option<vk::PipelineLayout>,
    pub(crate) model_rp: Option<vk::RenderPass>,
    pub(crate) model_framebuffers: Vec<vk::Framebuffer>,

    pub(crate) frame_sync: Vec<FrameSync>,
    pub(crate) current_frame: usize,
    pub(crate) cached_adapter_info: AdapterInfo,
    /// Last swapchain image index acquired (used by read_pixels).
    last_image_index: u32,


    // Phase 2: handle tables
    pub(crate) buffers: Slab<BufEntry>,
    pub(crate) pipelines: Slab<PipeEntry>,
    pub(crate) render_passes: Slab<vk::RenderPass>,
    pub(crate) framebuffers: Slab<vk::Framebuffer>,
    pub(crate) pipeline_layouts: Slab<PlEntry>,

    // Render pass metadata
    pub(crate) rp_has_depth: HashMap<u32, bool>,

    // Per-frame descriptor infrastructure (set=0 per FD-041)
    pub(crate) desc_set_layout_0: Option<vk::DescriptorSetLayout>,
    pub(crate) desc_pool: Option<vk::DescriptorPool>,
    pub(crate) frame_desc_sets: Vec<vk::DescriptorSet>,
    pub(crate) frame_ubos: Vec<vk::Buffer>,
    pub(crate) ubo_size: vk::DeviceSize,
    pub(crate) ubo_allocations: Vec<crate::allocator::Allocation>,
    pub(crate) ubo_alignment: u64,

    // Depth texture (matching swapchain size)
    pub(crate) depth_image: Option<vk::Image>,
    pub(crate) depth_image_view: Option<vk::ImageView>,
    pub(crate) depth_allocation: Option<crate::allocator::Allocation>,

    // Shadow mapping (directional light, 2048×2048)
    pub(crate) shadow_map: Option<vk::Image>,
    pub(crate) shadow_map_view: Option<vk::ImageView>,
    pub(crate) shadow_allocation: Option<crate::allocator::Allocation>,
    pub(crate) shadow_sampler: Option<vk::Sampler>,
    pub(crate) shadow_rp: Option<vk::RenderPass>,
    pub(crate) shadow_pipeline_layout: Option<vk::PipelineLayout>,
    pub(crate) shadow_pipeline: Option<vk::Pipeline>,
    pub(crate) shadow_fb: Option<vk::Framebuffer>,
    pub(crate) shadow_desc_set: Option<vk::DescriptorSet>,
    pub(crate) shadow_desc_layout: Option<vk::DescriptorSetLayout>,
    pub(crate) shadow_desc_pool: Option<vk::DescriptorPool>,
    /// Pipeline layout containing only set=1 (shadow map), used to bind the
    /// shadow descriptor set in `begin_frame` before the encoder takes over.
    pub(crate) shadow_bind_layout: Option<vk::PipelineLayout>,
}

impl VulkanDevice {
    pub fn new(
        display_handle: raw_window_handle::RawDisplayHandle,
        window_handle: raw_window_handle::RawWindowHandle,
        width: u32,
        height: u32,
        enable_validation: bool,
    ) -> Result<Self, VulkanError> {
        // SAFETY: `Instance::new` wraps the Vulkan C entry-point creation; the
        // returned value owns the instance handle.
        let instance = unsafe { Instance::new(display_handle, enable_validation) }?;
        // SAFETY: `Surface::new` calls Vulkan FFI to create a surface; handles
        // are valid and owned by the newly-created Surface value.
        let surface = unsafe {
            Surface::new(
                &instance.entry,
                &instance.instance,
                display_handle,
                window_handle,
            )
        }?;
        // SAFETY: `select` iterates physical devices and picks one; the
        // instance/physical-device handles are valid.
        let adapter = unsafe {
            crate::adapter::select(&instance.instance, &surface.loader, surface.surface)
        }?;
        // SAFETY: `VkLogicalDevice::new` creates a Vulkan logical device; all
        // inputs (instance, physical device) are valid.
        let ld = unsafe { VkLogicalDevice::new(&instance.instance, &adapter) }?;
        // SAFETY: `device_name` is a null-terminated `VkPhysicalDeviceProperties`
        // field guaranteed by the Vulkan spec to be a valid NUL-terminated char
        // array.
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
            logical_device: ManuallyDrop::new(ld),
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
            model_pipeline: None,
            model_pipeline_layout: None,
            model_rp: None,
            model_framebuffers: Vec::new(),
            frame_sync: Vec::new(),
            current_frame: 0,
            cached_adapter_info: info,
            last_image_index: 0,
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

            // Shadow mapping
            shadow_map: None,
            shadow_map_view: None,
            shadow_allocation: None,
            shadow_sampler: None,
            shadow_rp: None,
            shadow_pipeline_layout: None,
            shadow_pipeline: None,
            shadow_fb: None,
            shadow_desc_set: None,
            shadow_desc_layout: None,
            shadow_desc_pool: None,
            shadow_bind_layout: None,
        })
    }

    pub fn set_mvp_shaders(&mut self, vert: &'static [u8], frag: &'static [u8]) {
        self.mvp_vert_spv = Some(vert);
        self.mvp_frag_spv = Some(frag);
    }

    /// Returns the index of the current in-flight frame (0 or 1 for double
    /// buffering).  Used by sandbox code that writes per-frame UBO data via
    /// [`write_ubo`](Self::write_ubo).
    pub fn current_frame_index(&self) -> usize {
        self.current_frame
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

    pub fn resize(&mut self, w: u32, h: u32) {
        self.window_width = w.max(1);
        self.window_height = h.max(1);
        self.minimized = w == 0 || h == 0;
        // SAFETY: `self.logical_device` is alive by type invariant (ManuallyDrop
        // ensures VkLogicalDevice is not dropped before VulkanDevice).
        let _ = unsafe { self.logical_device.device.device_wait_idle() };
        self.destroy_mvp();
    }
    pub fn wait_idle(&self) {
        // SAFETY: `self.logical_device` is alive by type invariant (ManuallyDrop
        // ensures VkLogicalDevice is not dropped before VulkanDevice).
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
        if self.mvp_pipeline.is_none() {
            self.build_mvp()?;
        }
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
        self.record_shadow_pass(fi, &light_mvp, vertex_buf, index_buf, index_count)?;
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

    fn build_model_pipeline(&mut self) -> VkResult<()> {
        let vert = self
            .mvp_vert_spv
            .ok_or(VulkanError::MissingShader("model.vert"))?;
        let frag = self
            .mvp_frag_spv
            .ok_or(VulkanError::MissingShader("model.frag"))?;
        let sc = self.swapchain.as_ref().ok_or(VulkanError::Loader("swapchain not initialized".into()))?;
        let fmt = sc.format;
        let ext = self.swapchain_extent;
        let d = &self.logical_device.device;
        // SAFETY: `d` is a valid AshDevice; `vert` contains valid SPIR-V code.
        let vm = unsafe { mk_sm(d, vert)? };
        // SAFETY: `d` is a valid AshDevice; `frag` contains valid SPIR-V code.
        let fm = unsafe { mk_sm(d, frag)? };

        // --- Pipeline layout: set=0 (UBO) + set=1 (shadow map) + no push constants ---
        let mut set_layouts: Vec<vk::DescriptorSetLayout> = Vec::new();
        if let Some(dsl) = self.desc_set_layout_0 {
            set_layouts.push(dsl);
        }
        if let Some(sdl) = self.shadow_desc_layout {
            set_layouts.push(sdl);
        }
        let pli = vk::PipelineLayoutCreateInfo::default().set_layouts(&set_layouts);
        // SAFETY: `d` is a valid AshDevice; `pli` describes a valid pipeline
        // layout; `None` means no custom allocator.
        let pl = unsafe { d.create_pipeline_layout(&pli, None) }
            .map_err(|r| VulkanError::vk("cpl_model", r))?;

        // --- Render pass: color + depth ---
        let color_at = vk::AttachmentDescription::default()
            .format(fmt)
            .samples(vk::SampleCountFlags::TYPE_1)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::STORE)
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .final_layout(vk::ImageLayout::PRESENT_SRC_KHR);
        let depth_at = vk::AttachmentDescription::default()
            .format(vk::Format::D32_SFLOAT)
            .samples(vk::SampleCountFlags::TYPE_1)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::DONT_CARE)
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .final_layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL);
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
        let atts = [color_at, depth_at];
        let subpasses = [subpass];
        let deps = [dep];
        let rp_info = vk::RenderPassCreateInfo::default()
            .attachments(&atts)
            .subpasses(&subpasses)
            .dependencies(&deps);
        // SAFETY: `d` is a valid AshDevice; `rp_info` describes a valid render
        // pass; `None` means no custom allocator.
        let rp = unsafe { d.create_render_pass(&rp_info, None) }
            .map_err(|r| VulkanError::vk("crp_model", r))?;

        // --- Framebuffers (one per swapchain image, color + depth) ---
        let sc = self.swapchain.as_ref().ok_or(VulkanError::Loader("swapchain not initialized".into()))?;
        let depth_view = self
            .depth_image_view
            .unwrap_or(vk::ImageView::null());
        let mut fbs = Vec::new();
        for iv in &sc.image_views {
            let att_views = [*iv, depth_view];
            fbs.push(
                // SAFETY: `d` is a valid AshDevice; framebuffer creation info
                // references a valid render pass and image views; `None` means
                // no custom allocator.
                unsafe {
                    d.create_framebuffer(
                        &vk::FramebufferCreateInfo::default()
                            .render_pass(rp)
                            .attachments(&att_views)
                            .width(ext.width)
                            .height(ext.height)
                            .layers(1),
                        None,
                    )
                }
                .map_err(|r| VulkanError::vk("cfb_model", r))?,
            );
        }

        // --- Vertex input: position(loc=0), normal(loc=1), UV(loc=2) ---
        let stride = 32u32; // 3 + 3 + 2 floats = 8 * 4 = 32 bytes
        let vb = [vk::VertexInputBindingDescription::default()
            .binding(0)
            .stride(stride)
            .input_rate(vk::VertexInputRate::VERTEX)];
        let va = [
            vk::VertexInputAttributeDescription {
                location: 0,
                binding: 0,
                format: vk::Format::R32G32B32_SFLOAT,
                offset: 0,
            },
            vk::VertexInputAttributeDescription {
                location: 1,
                binding: 0,
                format: vk::Format::R32G32B32_SFLOAT,
                offset: 12,
            },
            vk::VertexInputAttributeDescription {
                location: 2,
                binding: 0,
                format: vk::Format::R32G32_SFLOAT,
                offset: 24,
            },
        ];
        let vi = vk::PipelineVertexInputStateCreateInfo::default()
            .vertex_binding_descriptions(&vb)
            .vertex_attribute_descriptions(&va);

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
        let ia = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST);
        let vs2 = vk::PipelineViewportStateCreateInfo::default()
            .viewport_count(1)
            .scissor_count(1);
        let rs = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(vk::CullModeFlags::BACK)
            .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
            .line_width(1.0);
        let ms = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);
        let blend_attachment = blend_attachment_from_mode("Alpha");
        let cba = [blend_attachment];
        let cb = vk::PipelineColorBlendStateCreateInfo::default()
            .logic_op_enable(false)
            .attachments(&cba);
        let ds = vk::PipelineDepthStencilStateCreateInfo::default()
            .depth_test_enable(true)
            .depth_write_enable(true)
            .depth_compare_op(vk::CompareOp::LESS);
        let dyns = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let ds2 = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dyns);
        let pinfo = vk::GraphicsPipelineCreateInfo::default()
            .stages(&sr)
            .vertex_input_state(&vi)
            .input_assembly_state(&ia)
            .viewport_state(&vs2)
            .rasterization_state(&rs)
            .multisample_state(&ms)
            .depth_stencil_state(&ds)
            .color_blend_state(&cb)
            .dynamic_state(&ds2)
            .layout(pl)
            .render_pass(rp)
            .subpass(0);
        // SAFETY: `d` is a valid AshDevice; `pinfo` describes a valid graphics
        // pipeline; `vk::PipelineCache::null()` is allowed; `None` means no
        // custom allocator.
        let p = unsafe { d.create_graphics_pipelines(vk::PipelineCache::null(), &[pinfo], None) }
            .map_err(|(_, r)| VulkanError::vk("cgp_model", r))?[0];
        // SAFETY: `vm` and `fm` were created by this device and are no longer
        // needed after pipeline creation; `None` means no custom allocator.
        unsafe {
            d.destroy_shader_module(vm, None);
            d.destroy_shader_module(fm, None);
        }
        self.model_pipeline_layout = Some(pl);
        self.model_pipeline = Some(p);
        self.model_rp = Some(rp);
        self.model_framebuffers = fbs;
        Ok(())
    }

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
        let sc = self.swapchain.as_ref().ok_or(VulkanError::Loader("swapchain not initialized".into()))?;
        let rp = self.model_rp.ok_or(VulkanError::Loader("model render pass not initialized".into()))?;
        let pl = self.model_pipeline.ok_or(VulkanError::Loader("model pipeline not initialized".into()))?;
        let pll = self.model_pipeline_layout.ok_or(VulkanError::Loader("model pipeline layout not initialized".into()))?;

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

    // --- Phase 2 helpers ---

    fn ensure_sc(&mut self) -> VkResult<()> {
        if self.swapchain.is_none() {
            let instance = self.instance.as_ref().ok_or(VulkanError::Loader("instance not initialized".into()))?;
            let surface = self.surface.as_ref().ok_or(VulkanError::Loader("surface not initialized".into()))?;
            // SAFETY: all handles (instance, device, physical device, surface)
            // are valid; `Swapchain::new` takes ownership of the cloned device.
            match unsafe {
                crate::swapchain::Swapchain::new(
                    &instance.instance,
                    self.logical_device.device.clone(),
                    self.adapter.physical_device,
                    self.logical_device.queue_family_index,
                    &surface.loader,
                    surface.surface,
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
                    // Create shadow mapping resources
                    self.ensure_shadow()?;
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
        let sc = self.swapchain.as_ref().ok_or(VulkanError::Loader("swapchain not initialized".into()))?;
        let f = &self.frame_sync[fi];
        // SAFETY: `f.in_flight_fence` is a valid fence created by this device;
        // waiting with `u64::MAX` timeout is safe.
        unsafe {
            self.logical_device
                .device
                .wait_for_fences(&[f.in_flight_fence], true, u64::MAX)
                .map_err(|r| VulkanError::vk("wf", r))?;
        }
        // SAFETY: `sc.loader` is a valid swapchain loader; `sc.swapchain` is a
        // valid VkSwapchainKHR; `f.image_available` is a valid semaphore;
        // timeout parameters are standard Vulkan.
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
        // SAFETY: `f.in_flight_fence` has been signaled (wait completed above);
        // resetting a signaled fence is valid.
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
        // SAFETY: `f.command_buffer` is a valid command buffer allocated from a
        // pool with `RESET_COMMAND_BUFFER` flag; `f.command_pool` owns it.
        unsafe {
            self.logical_device
                .device
                .reset_command_buffer(f.command_buffer, vk::CommandBufferResetFlags::empty())
                .map_err(|r| VulkanError::vk("rcb", r))?;
            // SAFETY: after reset the command buffer is in the initial state;
            // `begin_command_buffer` transitions it to recording state.
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
        let sc = self.swapchain.as_ref().ok_or(VulkanError::Loader("swapchain not initialized".into()))?;
        let rp = self.mvp_rp.ok_or(VulkanError::Loader("MVP render pass not initialized".into()))?;
        let pl = self.mvp_pipeline.ok_or(VulkanError::Loader("MVP pipeline not initialized".into()))?;
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

    fn submit_and_present(&self, fi: usize, ii: u32) -> VkResult<bool> {
        let d = &self.logical_device.device;
        let f = &self.frame_sync[fi];
        let sc = self.swapchain.as_ref().ok_or(VulkanError::Loader("swapchain not initialized".into()))?;
        // SAFETY: command buffer is in recording state; `end_command_buffer`
        // transitions it to completed state for submission.
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
        // SAFETY: `queue` is a valid VkQueue; command buffer is in completed
        // state; semaphores and fence are valid; submit info is correctly
        // structured.
        unsafe {
            d.queue_submit(self.logical_device.queue, &[si], f.in_flight_fence)
                .map_err(|r| VulkanError::vk("qs", r))?;
        }
        let sca = [sc.swapchain];
        let ia = [ii];
        // SAFETY: `queue` is valid; swapchain, semaphores, and image indices
        // are valid; `PresentInfoKHR` is correctly structured.
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
            // SAFETY: `fb` was created by this device and is still alive; not
            // already destroyed; `None` means no custom allocator.
            unsafe {
                d.destroy_framebuffer(fb, None);
            }
        }
        for fb in self.model_framebuffers.drain(..) {
            // SAFETY: `fb` was created by this device and is still alive.
            unsafe {
                d.destroy_framebuffer(fb, None);
            }
        }
        if let Some(p) = self.mvp_pipeline.take() {
            // SAFETY: `p` was created by this device and is still alive.
            unsafe {
                d.destroy_pipeline(p, None);
            }
        }
        if let Some(l) = self.mvp_pipeline_layout.take() {
            // SAFETY: `l` was created by this device and is still alive.
            unsafe {
                d.destroy_pipeline_layout(l, None);
            }
        }
        if let Some(p) = self.model_pipeline.take() {
            // SAFETY: `p` was created by this device and is still alive.
            unsafe {
                d.destroy_pipeline(p, None);
            }
        }
        if let Some(l) = self.model_pipeline_layout.take() {
            // SAFETY: `l` was created by this device and is still alive.
            unsafe {
                d.destroy_pipeline_layout(l, None);
            }
        }
        if let Some(rp) = self.mvp_rp.take() {
            // SAFETY: `rp` was created by this device and is still alive.
            unsafe {
                d.destroy_render_pass(rp, None);
            }
        }
        if let Some(rp) = self.model_rp.take() {
            // SAFETY: `rp` was created by this device and is still alive.
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
        let sc = self.swapchain.as_ref().ok_or(VulkanError::Loader("swapchain not initialized".into()))?;
        let fmt = sc.format;
        let ext = self.swapchain_extent;
        let d = &self.logical_device.device;
        // SAFETY: `d` is a valid AshDevice; `vert` contains valid SPIR-V code.
        let vm = unsafe { mk_sm(d, vert)? };
        // SAFETY: `d` is a valid AshDevice; `frag` contains valid SPIR-V code.
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
        // SAFETY: `d` is a valid AshDevice; `rpi` describes a valid render
        // pass; `None` means no custom allocator.
        let rp =
            unsafe { d.create_render_pass(&rpi, None) }.map_err(|r| VulkanError::vk("crp", r))?;
        let pli = vk::PipelineLayoutCreateInfo::default();
        // SAFETY: `d` is a valid AshDevice; `pli` describes a valid pipeline
        // layout; `None` means no custom allocator.
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
        // SAFETY: `d` is a valid AshDevice; `pinfo` describes a valid graphics
        // pipeline; `vk::PipelineCache::null()` is allowed; `None` means no
        // custom allocator.
        let p = unsafe { d.create_graphics_pipelines(vk::PipelineCache::null(), &[pinfo], None) }
            .map_err(|(_, r)| VulkanError::vk("cgp", r))?[0];
        // SAFETY: `vm` and `fm` were created by this device and are no longer
        // needed after pipeline creation; `None` means no custom allocator.
        unsafe {
            d.destroy_shader_module(vm, None);
            d.destroy_shader_module(fm, None);
        }
        let mut fbs = Vec::new();
        for iv in &sc.image_views {
            let iva = [*iv];
            fbs.push(
                // SAFETY: `d` is a valid AshDevice; framebuffer creation info
                // references a valid render pass and image views; `None` means
                // no custom allocator.
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

    fn build_frames(&mut self) -> VkResult<()> {
        let d = &self.logical_device.device;
        for _ in 0..2 {
            // SAFETY: `d` is a valid AshDevice; the queue family index is valid
            // for this device; `None` means no custom allocator.
            let cp = unsafe {
                d.create_command_pool(
                    &vk::CommandPoolCreateInfo::default()
                        .queue_family_index(self.logical_device.queue_family_index)
                        .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                    None,
                )
            }
            .map_err(|r| VulkanError::vk("ccp", r))?;
            // SAFETY: `cp` was just created and is valid; allocation info
            // correctly references the pool with PRIMARY level.
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
            // SAFETY: `d` is a valid AshDevice; `si` describes a valid
            // semaphore; `None` means no custom allocator.
            let ia =
                unsafe { d.create_semaphore(&si, None) }.map_err(|r| VulkanError::vk("cs", r))?;
            // SAFETY: same as above for the render-finished semaphore.
            let rf =
                unsafe { d.create_semaphore(&si, None) }.map_err(|r| VulkanError::vk("cs", r))?;
            // SAFETY: `d` is a valid AshDevice; fence is created in SIGNALED
            // state; `None` means no custom allocator.
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
// Shadow mapping
// ============================================================================

impl VulkanDevice {
    /// Ensure shadow mapping resources exist (idempotent).
    pub(crate) fn ensure_shadow(&mut self) -> VkResult<()> {
        if self.shadow_map.is_some() {
            return Ok(());
        }
        self.create_shadow_resources()
    }

    /// Create 2048×2048 directional-light shadow mapping resources.
    fn create_shadow_resources(&mut self) -> VkResult<()> {
        let d = &self.logical_device.device;
        let allocator = self.logical_device.allocator();
        const SHADOW_SIZE: u32 = 2048;

        // ---- 1. Shadow map image (D32_SFLOAT, GPU-only) ----
        let image_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(vk::Format::D32_SFLOAT)
            .extent(vk::Extent3D {
                width: SHADOW_SIZE,
                height: SHADOW_SIZE,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT | vk::ImageUsageFlags::SAMPLED)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        // SAFETY: `d` is a valid AshDevice; `image_info` describes a valid
        // 2D depth image; `None` means no custom allocator.
        let image = unsafe { d.create_image(&image_info, None) }
            .map_err(|r| VulkanError::vk("create_shadow_image", r))?;
        // SAFETY: `image` was just created by this device; querying memory
        // requirements for a valid image is safe.
        let req = unsafe { d.get_image_memory_requirements(image) };
        let allocation = allocator
            .lock().map_err(|e| VulkanError::Loader(format!("allocator lock: {e}")))?
            .allocate(&crate::allocator::AllocationCreateDesc {
                name: "shadow-map",
                requirements: req,
                location: crate::allocator::MemoryLocation::GpuOnly,
                linear: false,
                allocation_scheme: crate::allocator::AllocationScheme::GpuAllocatorManaged,
            })
            .map_err(|e| VulkanError::Allocation(e.to_string()))?;
        // SAFETY: `image` was created by this device; `allocation` was created
        // for this image's memory requirements; the memory and offset are valid.
        unsafe { d.bind_image_memory(image, allocation.memory(), allocation.offset()) }
            .map_err(|r| VulkanError::vk("bind_shadow_image", r))?;

        // ---- 2. Image view ----
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
        // SAFETY: `d` is a valid AshDevice; `view_info` references a valid
        // image and subresource range; `None` means no custom allocator.
        let image_view = unsafe { d.create_image_view(&view_info, None) }
            .map_err(|r| VulkanError::vk("create_shadow_image_view", r))?;

        // ---- 3. Sampler (PCF: COMPARE_MODE + LINEAR + CLAMP_TO_EDGE) ----
        let sampler_info = vk::SamplerCreateInfo::default()
            .mag_filter(vk::Filter::LINEAR)
            .min_filter(vk::Filter::LINEAR)
            .mipmap_mode(vk::SamplerMipmapMode::LINEAR)
            .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .compare_enable(true)
            .compare_op(vk::CompareOp::LESS)
            .min_lod(0.0)
            .max_lod(1.0)
            .mip_lod_bias(0.0)
            .anisotropy_enable(false);
        // SAFETY: `d` is a valid AshDevice; `sampler_info` describes a valid
        // sampler; `None` means no custom allocator.
        let sampler = unsafe { d.create_sampler(&sampler_info, None) }
            .map_err(|r| VulkanError::vk("create_shadow_sampler", r))?;

        // ---- 4. Render pass (depth-only, CLEAR load op) ----
        let depth_at = vk::AttachmentDescription::default()
            .format(vk::Format::D32_SFLOAT)
            .samples(vk::SampleCountFlags::TYPE_1)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::STORE)
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .final_layout(vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL);
        let depth_ref = vk::AttachmentReference::default()
            .attachment(0)
            .layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL);
        let subpass = vk::SubpassDescription::default()
            .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
            .depth_stencil_attachment(&depth_ref);
        // Subpass dependencies: external → shadow (write), shadow → external (read)
        let deps = [
            vk::SubpassDependency::default()
                .src_subpass(vk::SUBPASS_EXTERNAL)
                .dst_subpass(0)
                .src_stage_mask(vk::PipelineStageFlags::TOP_OF_PIPE)
                .dst_stage_mask(vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS)
                .dst_access_mask(vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE),
            vk::SubpassDependency::default()
                .src_subpass(0)
                .dst_subpass(vk::SUBPASS_EXTERNAL)
                .src_stage_mask(vk::PipelineStageFlags::LATE_FRAGMENT_TESTS)
                .src_access_mask(vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE)
                .dst_stage_mask(vk::PipelineStageFlags::FRAGMENT_SHADER)
                .dst_access_mask(vk::AccessFlags::SHADER_READ),
        ];
        let atts = [depth_at];
        let subpasses = [subpass];
        let rp_info = vk::RenderPassCreateInfo::default()
            .attachments(&atts)
            .subpasses(&subpasses)
            .dependencies(&deps);
        // SAFETY: `d` is a valid AshDevice; `rp_info` describes a valid
        // depth-only render pass; `None` means no custom allocator.
        let rp = unsafe { d.create_render_pass(&rp_info, None) }
            .map_err(|r| VulkanError::vk("crp_shadow", r))?;

        // ---- 5. Pipeline layout (push constant: mat4 light MVP at vertex stage) ----
        let pc_range = [vk::PushConstantRange {
            stage_flags: vk::ShaderStageFlags::VERTEX,
            offset: 0,
            size: 64,
        }];
        let pli = vk::PipelineLayoutCreateInfo::default()
            .push_constant_ranges(&pc_range);
        // SAFETY: `d` is a valid AshDevice; `pli` describes a valid pipeline
        // layout with push constant ranges; `None` means no custom allocator.
        let pll = unsafe { d.create_pipeline_layout(&pli, None) }
            .map_err(|r| VulkanError::vk("cpl_shadow", r))?;

        // ---- 6. Shadow pipeline (depth-only, no color writes) ----
        let vert_spv = crate::shaders_embedded::SHADOW_VERT_SPV;
        let frag_spv = crate::shaders_embedded::SHADOW_FRAG_SPV;
        // SAFETY: `d` is a valid AshDevice; `vert_spv` contains valid SPIR-V.
        let vm = unsafe { mk_sm(d, vert_spv) }?;
        // SAFETY: `d` is a valid AshDevice; `frag_spv` contains valid SPIR-V.
        let fm = unsafe { mk_sm(d, frag_spv) }?;

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

        // Vertex input (position at loc=0, stride 32)
        let vb = [vk::VertexInputBindingDescription::default()
            .binding(0)
            .stride(32)
            .input_rate(vk::VertexInputRate::VERTEX)];
        let va = [vk::VertexInputAttributeDescription {
            location: 0,
            binding: 0,
            format: vk::Format::R32G32B32_SFLOAT,
            offset: 0,
        }];
        let vi = vk::PipelineVertexInputStateCreateInfo::default()
            .vertex_binding_descriptions(&vb)
            .vertex_attribute_descriptions(&va);

        let ia = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST);
        let vs = vk::PipelineViewportStateCreateInfo::default()
            .viewport_count(1)
            .scissor_count(1);
        let rs = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(vk::CullModeFlags::BACK)
            .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
            .line_width(1.0)
            .depth_bias_enable(true)
            .depth_bias_constant_factor(1.5)
            .depth_bias_slope_factor(1.5);
        let ms = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);
        // No color attachments – empty blend state
        let cba: [vk::PipelineColorBlendAttachmentState; 0] = [];
        let cb = vk::PipelineColorBlendStateCreateInfo::default()
            .logic_op_enable(false)
            .attachments(&cba);
        let ds_state = vk::PipelineDepthStencilStateCreateInfo::default()
            .depth_test_enable(true)
            .depth_write_enable(true)
            .depth_compare_op(vk::CompareOp::LESS_OR_EQUAL);
        let dyns = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let ds = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dyns);

        let pinfo = vk::GraphicsPipelineCreateInfo::default()
            .stages(&sr)
            .vertex_input_state(&vi)
            .input_assembly_state(&ia)
            .viewport_state(&vs)
            .rasterization_state(&rs)
            .multisample_state(&ms)
            .depth_stencil_state(&ds_state)
            .color_blend_state(&cb)
            .dynamic_state(&ds)
            .layout(pll)
            .render_pass(rp)
            .subpass(0);
        // SAFETY: `d` is a valid AshDevice; `pinfo` describes a valid graphics
        // pipeline (depth-only, no color attachments); `vk::PipelineCache::null()`
        // is allowed; `None` means no custom allocator.
        let pipeline =
            unsafe { d.create_graphics_pipelines(vk::PipelineCache::null(), &[pinfo], None) }
                .map_err(|(_, r)| VulkanError::vk("cgp_shadow", r))?[0];

        // SAFETY: `vm` and `fm` were created by this device and are no longer
        // needed after pipeline creation; `None` means no custom allocator.
        unsafe {
            d.destroy_shader_module(vm, None);
            d.destroy_shader_module(fm, None);
        }

        // ---- 7. Framebuffer ----
        // SAFETY: `d` is a valid AshDevice; framebuffer info references a valid
        // render pass and image view; `None` means no custom allocator.
        let fb = unsafe {
            d.create_framebuffer(
                &vk::FramebufferCreateInfo::default()
                    .render_pass(rp)
                    .attachments(&[image_view])
                    .width(SHADOW_SIZE)
                    .height(SHADOW_SIZE)
                    .layers(1),
                None,
            )
        }
        .map_err(|r| VulkanError::vk("cfb_shadow", r))?;

        // ---- 8. Descriptor set layout (set=1, binding 0 = combined image sampler) ----
        let ds_bindings = [vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT)];
        let ds_layout_info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&ds_bindings);
        // SAFETY: `d` is a valid AshDevice; `ds_layout_info` describes a valid
        // layout with one combined image sampler binding; `None` means no custom
        // allocator.
        let ds_layout = unsafe { d.create_descriptor_set_layout(&ds_layout_info, None) }
            .map_err(|r| VulkanError::vk("create_shadow_ds_layout", r))?;

        // ---- 9. Descriptor pool + set ----
        let pool_sizes = [vk::DescriptorPoolSize {
            ty: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
            descriptor_count: 1,
        }];
        let pool_info = vk::DescriptorPoolCreateInfo::default()
            .max_sets(1)
            .pool_sizes(&pool_sizes);
        // SAFETY: `d` is a valid AshDevice; `pool_info` describes a valid pool;
        // `None` means no custom allocator.
        let pool = unsafe { d.create_descriptor_pool(&pool_info, None) }
            .map_err(|r| VulkanError::vk("create_shadow_ds_pool", r))?;

        let ds_layouts = [ds_layout];
        let alloc_info = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(pool)
            .set_layouts(&ds_layouts);
        // SAFETY: `d` is a valid AshDevice; `alloc_info` references a valid
        // pool and layout; the pool has enough capacity.
        let desc_sets = unsafe { d.allocate_descriptor_sets(&alloc_info) }
            .map_err(|r| VulkanError::vk("alloc_shadow_ds", r))?;
        let desc_set = desc_sets[0];

        // Write descriptor: combine shadow map view + sampler
        let image_info = [vk::DescriptorImageInfo::default()
            .sampler(sampler)
            .image_view(image_view)
            .image_layout(vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL)];
        let writes = [vk::WriteDescriptorSet::default()
            .dst_set(desc_set)
            .dst_binding(0)
            .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
            .image_info(&image_info)];
        // SAFETY: `d` is a valid AshDevice; write descriptor references valid
        // descriptor set, sampler, and image view; no zero handles.
        unsafe {
            d.update_descriptor_sets(&writes, &[]);
        }

        // ---- Store ----
        self.shadow_map = Some(image);
        self.shadow_map_view = Some(image_view);
        self.shadow_allocation = Some(allocation);
        self.shadow_sampler = Some(sampler);
        self.shadow_rp = Some(rp);
        self.shadow_pipeline_layout = Some(pll);
        self.shadow_pipeline = Some(pipeline);
        self.shadow_fb = Some(fb);
        self.shadow_desc_layout = Some(ds_layout);
        self.shadow_desc_pool = Some(pool);
        self.shadow_desc_set = Some(desc_set);

        // ---- 10. Bind-only pipeline layout (set=1 only, for early binding in begin_frame) ----
        let bind_set_layouts = [ds_layout];
        let bind_pli = vk::PipelineLayoutCreateInfo::default()
            .set_layouts(&bind_set_layouts);
        // SAFETY: `d` is a valid AshDevice; `bind_pli` describes a valid layout;
        // `None` means no custom allocator.
        let bind_pll = unsafe { d.create_pipeline_layout(&bind_pli, None) }
            .map_err(|r| VulkanError::vk("cpl_shadow_bind", r))?;
        self.shadow_bind_layout = Some(bind_pll);

        Ok(())
    }

    /// Destroy all shadow mapping resources (reverse order of creation).
    pub(crate) fn destroy_shadow_resources(&mut self) {
        let d = &self.logical_device.device;

        // Descriptor pool automatically frees its descriptor sets
        if let Some(pool) = self.shadow_desc_pool.take() {
            // SAFETY: `pool` was created by this device and is still alive.
            unsafe { d.destroy_descriptor_pool(pool, None); }
        }
        if let Some(layout) = self.shadow_desc_layout.take() {
            // SAFETY: `layout` was created by this device and is still alive.
            unsafe { d.destroy_descriptor_set_layout(layout, None); }
        }
        if let Some(layout) = self.shadow_bind_layout.take() {
            // SAFETY: `layout` was created by this device and is still alive.
            unsafe { d.destroy_pipeline_layout(layout, None); }
        }
        if let Some(fb) = self.shadow_fb.take() {
            // SAFETY: `fb` was created by this device and is still alive.
            unsafe { d.destroy_framebuffer(fb, None); }
        }
        if let Some(p) = self.shadow_pipeline.take() {
            // SAFETY: `p` was created by this device and is still alive.
            unsafe { d.destroy_pipeline(p, None); }
        }
        if let Some(l) = self.shadow_pipeline_layout.take() {
            // SAFETY: `l` was created by this device and is still alive.
            unsafe { d.destroy_pipeline_layout(l, None); }
        }
        if let Some(rp) = self.shadow_rp.take() {
            // SAFETY: `rp` was created by this device and is still alive.
            unsafe { d.destroy_render_pass(rp, None); }
        }
        if let Some(s) = self.shadow_sampler.take() {
            // SAFETY: `s` was created by this device and is still alive.
            unsafe { d.destroy_sampler(s, None); }
        }
        if let Some(iv) = self.shadow_map_view.take() {
            // SAFETY: `iv` was created by this device and is still alive.
            unsafe { d.destroy_image_view(iv, None); }
        }
        if let Some(img) = self.shadow_map.take() {
            // SAFETY: `img` was created by this device and is still alive.
            unsafe { d.destroy_image(img, None); }
        }
        if let Some(mut a) = self.shadow_allocation.take() {
            if let Ok(mut guard) = self.logical_device.allocator().lock() {
                let _ = guard.free(&mut a);
            }
        }
    }

    /// Compute a light view-projection matrix for directional shadow mapping.
    fn compute_light_mvp(&self) -> [[f32; 4]; 4] {
        let light_dir = glam::Vec3::new(0.5, -0.707, 0.5).normalize();
        // Position the light far away, looking at the origin
        let light_pos = -light_dir * 10.0;
        let view =
            glam::Mat4::look_at_rh(light_pos, glam::Vec3::ZERO, glam::Vec3::Y);
        let ortho = glam::Mat4::orthographic_rh(-5.0, 5.0, -5.0, 5.0, 0.1, 20.0);
        let light_mvp = ortho * view;
        light_mvp.to_cols_array_2d()
    }

    /// Record a shadow-mapping render pass into the already-begun command buffer.
    ///
    /// The command buffer MUST have been started via [`begin_cb`] before calling
    /// this method. The shadow map is bound as a depth attachment and the scene
    /// is rendered from the light's point of view using the given `light_mvp`
    /// push constant.
    fn record_shadow_pass(
        &self,
        fi: usize,
        light_mvp: &[[f32; 4]; 4],
        vertex_buf: render_core::BufferHandle,
        index_buf: render_core::BufferHandle,
        index_count: u32,
    ) -> VkResult<()> {
        let d = &self.logical_device.device;
        let f = &self.frame_sync[fi];
        let rp = self.shadow_rp.ok_or(VulkanError::Loader("shadow render pass not initialized".into()))?;
        let pl = self.shadow_pipeline.ok_or(VulkanError::Loader("shadow pipeline not initialized".into()))?;
        let pll = self.shadow_pipeline_layout.ok_or(VulkanError::Loader("shadow pipeline layout not initialized".into()))?;
        const SHADOW_SIZE: u32 = 2048;

        // Look up Vulkan buffer handles
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

        // Begin shadow render pass
        let clear_depth = vk::ClearValue {
            depth_stencil: vk::ClearDepthStencilValue {
                depth: 1.0,
                stencil: 0,
            },
        };
        let clear_values = [clear_depth];
        let rpbi = vk::RenderPassBeginInfo::default()
            .render_pass(rp)
            .framebuffer(self.shadow_fb.ok_or(VulkanError::Loader("shadow framebuffer not initialized".into()))?)
            .render_area(vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: vk::Extent2D {
                    width: SHADOW_SIZE,
                    height: SHADOW_SIZE,
                },
            })
            .clear_values(&clear_values);

        // SAFETY: command buffer is in recording state; render pass and
        // framebuffer are valid; `SubpassContents::INLINE` is correct.
        unsafe {
            d.cmd_begin_render_pass(f.command_buffer, &rpbi, vk::SubpassContents::INLINE);
        }

        // Viewport + scissor
        let vp = vk::Viewport {
            x: 0.0,
            y: 0.0,
            width: SHADOW_SIZE as f32,
            height: SHADOW_SIZE as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        };
        // SAFETY: command buffer is inside a render pass instance; all handles
        // (pipeline, push constants, buffers) are valid; `light_mvp` is a
        // stack-local array valid for the duration of the unsafe block.
        unsafe {
            d.cmd_set_viewport(f.command_buffer, 0, &[vp]);
            d.cmd_set_scissor(
                f.command_buffer,
                0,
                &[vk::Rect2D {
                    offset: vk::Offset2D { x: 0, y: 0 },
                    extent: vk::Extent2D {
                        width: SHADOW_SIZE,
                        height: SHADOW_SIZE,
                    },
                }],
            );
            d.cmd_bind_pipeline(f.command_buffer, vk::PipelineBindPoint::GRAPHICS, pl);

            // Push constant: mat4 light MVP (64 bytes)
            // SAFETY: `light_mvp` pointer is valid for the size of the matrix;
            // push constant range (64 bytes at offset 0) matches the pipeline
            // layout declaration.
            let mvp_bytes: &[u8] = std::slice::from_raw_parts(
                light_mvp.as_ptr() as *const u8,
                std::mem::size_of::<[[f32; 4]; 4]>(),
            );
            d.cmd_push_constants(
                f.command_buffer,
                pll,
                vk::ShaderStageFlags::VERTEX,
                0,
                mvp_bytes,
            );

            // Vertex + index buffers
            let vbs = [vk_vb];
            let offsets = [0u64];
            d.cmd_bind_vertex_buffers(f.command_buffer, 0, &vbs, &offsets);
            d.cmd_bind_index_buffer(f.command_buffer, vk_ib, 0, vk::IndexType::UINT32);
            d.cmd_draw_indexed(f.command_buffer, index_count, 1, 0, 0, 0);
            d.cmd_end_render_pass(f.command_buffer);
        }

        // Pipeline barrier: make shadow depth writes visible to fragment shader reads
        // in the subsequent forward render pass.
        let barrier = vk::ImageMemoryBarrier::default()
            .image(self.shadow_map.ok_or(VulkanError::Loader("shadow map image not initialized".into()))?)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::DEPTH,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            })
            .src_access_mask(vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE)
            .dst_access_mask(vk::AccessFlags::SHADER_READ)
            .old_layout(vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL)
            .new_layout(vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL);
        // SAFETY: command buffer is still in recording state (not yet ended);
        // the barrier references a valid image; stage masks are correct for
        // depth write → shader read transition.
        unsafe {
            d.cmd_pipeline_barrier(
                f.command_buffer,
                vk::PipelineStageFlags::LATE_FRAGMENT_TESTS,
                vk::PipelineStageFlags::FRAGMENT_SHADER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[barrier],
            );
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
        // SAFETY: `d.device` is a valid AshDevice; `bi` describes a valid
        // buffer; `None` means no custom allocator.
        let buffer = unsafe { d.device.create_buffer(&bi, None) }.map_err(|r| {
            render_core::RhiError::Backend {
                detail: format!("{r:?}"),
            }
        })?;
        // SAFETY: `buffer` was just created by this device; querying memory
        // requirements for a valid buffer is safe.
        let req = unsafe { d.device.get_buffer_memory_requirements(buffer) };
        let alloc_handle = d.allocator();
        let location = crate::allocator::MemoryLocation::CpuToGpu;
        let mut allocation = alloc_handle
            .lock().map_err(|e| render_core::RhiError::Backend {
                detail: format!("allocator lock: {e}"),
            })?
            .allocate(&crate::allocator::AllocationCreateDesc {
                name: "device-buffer",
                requirements: req,
                location,
                linear: true,
                allocation_scheme: crate::allocator::AllocationScheme::GpuAllocatorManaged,
            })
            .map_err(|e| render_core::RhiError::Backend {
                detail: format!("{e}"),
            })?;
        // SAFETY: `buffer` was created by this device; `allocation` was created
        // for this buffer's memory requirements; the memory and offset are valid.
        if let Err(r) = unsafe {
            d.device
                .bind_buffer_memory(buffer, allocation.memory(), allocation.offset())
        } {
            if let Ok(mut alloc_guard) = alloc_handle.lock() {
                let _ = alloc_guard.free(&mut allocation);
            }
            // SAFETY: `buffer` was just created; not bound to memory; destroying
            // a freshly-created buffer is safe even on failed bind.
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
            // SAFETY: `d` is a valid AshDevice; `rp_info` describes a valid
            // render pass with color + depth attachments; `None` means no
            // custom allocator.
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
            // SAFETY: `d` is a valid AshDevice; `rp_info` describes a valid
            // render pass with color attachment only; `None` means no custom
            // allocator.
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
            // SAFETY: `d` is a valid AshDevice; `fi` references a valid render
            // pass and image views (null image views are allowed for placeholders);
            // `None` means no custom allocator.
            unsafe { d.create_framebuffer(&fi, None) }
        } else {
            let fi = vk::FramebufferCreateInfo::default()
                .render_pass(rp)
                .width(desc.width)
                .height(desc.height)
                .layers(1);
            // SAFETY: `d` is a valid AshDevice; `fi` references a valid render
            // pass; `None` means no custom allocator.
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
        // and the shadow map descriptor set layout (set=1) when available.
        let mut set_layouts: Vec<vk::DescriptorSetLayout> = Vec::new();
        if let Some(dsl) = self.desc_set_layout_0 {
            set_layouts.push(dsl);
        }
        if let Some(sdl) = self.shadow_desc_layout {
            set_layouts.push(sdl);
        }
        // Also add any from the descriptor
        for _bg in &desc.bind_group_layouts {
            // For now, bind_group_layouts from descriptor are ignored
            // since set=0 is always the per-frame layout
        }
        let info = vk::PipelineLayoutCreateInfo::default()
            .set_layouts(&set_layouts)
            .push_constant_ranges(&pc_ranges);
        // SAFETY: `d` is a valid AshDevice; `info` describes a valid pipeline
        // layout with descriptor set layouts and push constant ranges; `None`
        // means no custom allocator.
        let layout = unsafe { d.create_pipeline_layout(&info, None) }.map_err(|r| {
            render_core::RhiError::Backend {
                detail: format!("{r:?}"),
            }
        })?;
        let (idx, gen) = self.pipeline_layouts.insert(PlEntry {
            layout,
            _device: d.clone(),
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
        // SAFETY: `d` is a valid AshDevice; `vs` contains valid SPIR-V code.
        let vm = (unsafe { mk_sm(d, vs) }).map_err(|e| render_core::RhiError::Backend {
            detail: format!("{e}"),
        })?;
        // SAFETY: `d` is a valid AshDevice; `fs` contains valid SPIR-V code.
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
            // SAFETY: `d` is a valid AshDevice; `rpi` describes a valid render
            // pass; `None` means no custom allocator.
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
        // SAFETY: `d` is a valid AshDevice; `pinfo` describes a valid graphics
        // pipeline; `vk::PipelineCache::null()` is allowed; `None` means no
        // custom allocator.
        let pipeline =
            unsafe { d.create_graphics_pipelines(vk::PipelineCache::null(), &[pinfo], None) }
                .map_err(|(_, r)| render_core::RhiError::Backend {
                    detail: format!("{r:?}"),
                })?[0];
        // SAFETY: `vm` and `fm` were created by this device and are no longer
        // needed after pipeline creation; `None` means no custom allocator.
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
        self.last_image_index = ii;
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
            // Snapshot slab entries into owned Vec caches — no raw pointers.
            pipeline_cache: self
                .pipelines
                .slots
                .iter()
                .map(|s| s.as_ref().map(|(g, e)| (*g, e.pipeline)))
                .collect(),
            buffer_cache: self
                .buffers
                .slots
                .iter()
                .map(|s| s.as_ref().map(|(g, e)| (*g, e.buffer)))
                .collect(),
            render_pass_cache: self.render_passes.slots.clone(),
            framebuffer_cache: self.framebuffers.slots.clone(),
            pipeline_layout_cache: self
                .pipeline_layouts
                .slots
                .iter()
                .map(|s| s.as_ref().map(|(g, e)| (*g, e.layout)))
                .collect(),
            current_desc_set: desc_set,
        });

        // Pre-bind the shadow descriptor set at set=1 (if available) so that
        // subsequent encoder operations do not leave it unbound.  The encoder
        // later binds the UBO at set=0 via `bind_descriptor_sets`.
        if let Some(sds) = self.shadow_desc_set {
            if let Some(bind_pll) = self.shadow_bind_layout {
                let shadow_sets = [sds];
                // SAFETY: command buffer is in recording state; descriptor set,
                // pipeline layout, and command buffer are valid Vulkan objects
                // created by the same device.
                unsafe {
                    self.logical_device.device.cmd_bind_descriptor_sets(
                        f.command_buffer,
                        vk::PipelineBindPoint::GRAPHICS,
                        bind_pll,
                        1,
                        &shadow_sets,
                        &[],
                    );
                }
            }
        }

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
            // SAFETY: `self.logical_device` is alive by type invariant
            // (ManuallyDrop ensures destruction order).
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
        // SAFETY: `self.logical_device` is alive by type invariant
        // (ManuallyDrop ensures destruction order).
        let _ = unsafe { self.logical_device.device.device_wait_idle() };
        self.window_width = w.max(1);
        self.window_height = h.max(1);
        self.swapchain = None;
        Ok(())
    }

    fn wait_idle(&self) {
        // SAFETY: `self.logical_device` is alive by type invariant
        // (ManuallyDrop ensures destruction order).
        let _ = unsafe { self.logical_device.device.device_wait_idle() };
    }

    fn read_pixels(
        &mut self,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    ) -> Result<Vec<u8>, render_core::RhiError> {
        // Flush all pending GPU work so the swapchain images are in a
        // deterministic layout (PRESENT_SRC_KHR after the last render pass).
        // SAFETY: `self.logical_device` is alive by type invariant (ManuallyDrop
        // ensures destruction order).
        let _ = unsafe { self.logical_device.device.device_wait_idle() };

        let sc = self
            .swapchain
            .as_ref()
            .ok_or_else(|| render_core::RhiError::Backend {
                detail: "no swapchain".into(),
            })?;

        // Validate the requested region against the swapchain extent.
        if x + width > sc.extent.width || y + height > sc.extent.height || width == 0 || height == 0
        {
            return Err(render_core::RhiError::Backend {
                detail: format!(
                    "readback region ({x},{y}) {width}×{height} exceeds swapchain {}×{}",
                    sc.extent.width, sc.extent.height
                ),
            });
        }

        // Pixel buffer: 4 bytes per pixel (RGBA return format).
        let pixel_size: vk::DeviceSize = 4;
        let buffer_size = (width as vk::DeviceSize) * (height as vk::DeviceSize) * pixel_size;

        let d = &self.logical_device;
        let device = &d.device;

        // -----------------------------------------------------------------
        // 1. Create a staging buffer (GPU write → CPU read).
        //    NOTE: The swapchain images MUST have been created with
        //    VK_IMAGE_USAGE_TRANSFER_SRC_BIT for vkCmdCopyImageToBuffer to
        //    work.  Add this to the usage flags in swapchain::new().
        // -----------------------------------------------------------------
        // SAFETY: `device` is a valid AshDevice; buffer creation describes a
        // valid TRANSFER_DST buffer; `None` means no custom allocator.
        let staging_buffer = unsafe {
            device.create_buffer(
                &vk::BufferCreateInfo::default()
                    .size(buffer_size)
                    .usage(vk::BufferUsageFlags::TRANSFER_DST)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE),
                None,
            )
        }
        .map_err(|r| render_core::RhiError::Backend {
            detail: format!("create staging buffer: {r:?}"),
        })?;

        // SAFETY: `staging_buffer` was just created by this device; querying
        // memory requirements for a valid buffer is safe.
        let req = unsafe { device.get_buffer_memory_requirements(staging_buffer) };
        let alloc_handle = d.allocator();
        let mut staging_alloc = alloc_handle
            .lock().map_err(|e| render_core::RhiError::Backend {
                detail: format!("allocator lock: {e}"),
            })?
            .allocate(&crate::allocator::AllocationCreateDesc {
                name: "read_pixels staging",
                requirements: req,
                location: crate::allocator::MemoryLocation::GpuToCpu,
                linear: true,
                allocation_scheme: crate::allocator::AllocationScheme::GpuAllocatorManaged,
            })
            .map_err(|e| {
                // SAFETY: buffer was just created by this device and is not
                // in use; destroying it on allocation failure is correct.
                unsafe { device.destroy_buffer(staging_buffer, None) };
                render_core::RhiError::Backend {
                    detail: format!("alloc staging: {e}"),
                }
            })?;

        // SAFETY: `staging_buffer` was created by this device; `staging_alloc`
        // was created for this buffer's memory requirements; memory and offset
        // are valid.
        if let Err(r) = unsafe {
            device.bind_buffer_memory(
                staging_buffer,
                staging_alloc.memory(),
                staging_alloc.offset(),
            )
        } {
            // SAFETY: buffer/allocation were just created and are not in use
            // after the failed bind; cleanup is safe.
            if let Ok(mut guard) = alloc_handle.lock() { let _ = guard.free(&mut staging_alloc); }
            unsafe { device.destroy_buffer(staging_buffer, None) };
            return Err(render_core::RhiError::Backend {
                detail: format!("bind staging: {r:?}"),
            });
        }

        // -----------------------------------------------------------------
        // 2. One-shot command pool + command buffer.
        // -----------------------------------------------------------------
        // SAFETY: `device` is a valid AshDevice; the queue family index is
        // valid for this device; `None` means no custom allocator.
        let cmd_pool = unsafe {
            device.create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .queue_family_index(d.queue_family_index)
                    .flags(vk::CommandPoolCreateFlags::TRANSIENT),
                None,
            )
        }
        .map_err(|r| {
            if let Ok(mut guard) = alloc_handle.lock() { let _ = guard.free(&mut staging_alloc); }
            // SAFETY: cleanup only happen on error; all handles are valid.
            unsafe { device.destroy_buffer(staging_buffer, None) };
            render_core::RhiError::Backend {
                detail: format!("create pool: {r:?}"),
            }
        })?;

        // SAFETY: `cmd_pool` was just created and is valid; allocation info
        // correctly references the pool with PRIMARY level and 1 buffer.
        let cmd_buffer = unsafe {
            device.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(cmd_pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(1),
            )
        }
        .map_err(|r| {
            // SAFETY: cleanup only on error; all handles created so far are valid.
            unsafe { device.destroy_command_pool(cmd_pool, None) };
            if let Ok(mut guard) = alloc_handle.lock() { let _ = guard.free(&mut staging_alloc); }
            unsafe { device.destroy_buffer(staging_buffer, None) };
            render_core::RhiError::Backend {
                detail: format!("alloc cb: {r:?}"),
            }
        })?[0];

        // -----------------------------------------------------------------
        // 3. Record the copy command buffer.
        // -----------------------------------------------------------------
        // SAFETY: command buffer is in the initial state (just allocated from
        // a transient pool); begin transitions it to recording state.
        unsafe {
            device.begin_command_buffer(
                cmd_buffer,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )
        }
        .map_err(|r| {
            // SAFETY: cleanup only on error; all handles created so far are valid.
            unsafe { device.destroy_command_pool(cmd_pool, None) };
            if let Ok(mut guard) = alloc_handle.lock() { let _ = guard.free(&mut staging_alloc); }
            unsafe { device.destroy_buffer(staging_buffer, None) };
            render_core::RhiError::Backend {
                detail: format!("begin cb: {r:?}"),
            }
        })?;

        // Use the last acquired image index (tracked from render_model_frame).
        let img_idx = self.last_image_index.min(sc.images.len() as u32 - 1);
        let swapchain_image = sc.images[img_idx as usize];

        // 3a. PRESENT_SRC_KHR → TRANSFER_SRC_OPTIMAL
        let to_transfer_barrier = vk::ImageMemoryBarrier::default()
            .old_layout(vk::ImageLayout::PRESENT_SRC_KHR)
            .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(swapchain_image)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            })
            .src_access_mask(vk::AccessFlags::empty())
            .dst_access_mask(vk::AccessFlags::TRANSFER_READ);
        // SAFETY: command buffer is in recording state; barrier references a
        // live swapchain image; stage and access masks match the layout
        // transition semantics.
        unsafe {
            device.cmd_pipeline_barrier(
                cmd_buffer,
                vk::PipelineStageFlags::TOP_OF_PIPE,
                vk::PipelineStageFlags::TRANSFER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[to_transfer_barrier],
            );
        }

        // 3b. Copy the requested region from image → staging buffer.
        let copy_region = vk::BufferImageCopy::default()
            .buffer_offset(0)
            .buffer_row_length(0)
            .buffer_image_height(0)
            .image_subresource(vk::ImageSubresourceLayers {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                mip_level: 0,
                base_array_layer: 0,
                layer_count: 1,
            })
            .image_offset(vk::Offset3D {
                x: x as i32,
                y: y as i32,
                z: 0,
            })
            .image_extent(vk::Extent3D {
                width,
                height,
                depth: 1,
            });
        // SAFETY: both image and buffer are valid Vulkan objects; image is in
        // TRANSFER_SRC_OPTIMAL layout; copy region is within bounds.
        unsafe {
            device.cmd_copy_image_to_buffer(
                cmd_buffer,
                swapchain_image,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                staging_buffer,
                &[copy_region],
            );
        }

        // 3c. TRANSFER_SRC_OPTIMAL → PRESENT_SRC_KHR (restore).
        let to_present_barrier = vk::ImageMemoryBarrier::default()
            .old_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
            .new_layout(vk::ImageLayout::PRESENT_SRC_KHR)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(swapchain_image)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            })
            .src_access_mask(vk::AccessFlags::TRANSFER_READ)
            .dst_access_mask(vk::AccessFlags::empty());
        // SAFETY: command buffer is still recording; image is live; restoring
        // the original layout matches the swapchain contract.
        unsafe {
            device.cmd_pipeline_barrier(
                cmd_buffer,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[to_present_barrier],
            );
        }

        // SAFETY: command buffer is in recording state; after this call it
        // transitions to completed state, ready for submission.
        unsafe { device.end_command_buffer(cmd_buffer) }.map_err(|r| {
            // SAFETY: cleanup only on error; all handles created so far are valid.
            unsafe { device.destroy_command_pool(cmd_pool, None) };
            if let Ok(mut guard) = alloc_handle.lock() { let _ = guard.free(&mut staging_alloc); }
            unsafe { device.destroy_buffer(staging_buffer, None) };
            render_core::RhiError::Backend {
                detail: format!("end cb: {r:?}"),
            }
        })?;

        // -----------------------------------------------------------------
        // 4. Submit and wait for completion.
        // -----------------------------------------------------------------
        // SAFETY: `device` is a valid AshDevice; fence is created with default
        // (unsignaled) state; `None` means no custom allocator.
        let fence =
            unsafe { device.create_fence(&vk::FenceCreateInfo::default(), None) }.map_err(|r| {
                // SAFETY: cleanup only on error; all handles are valid.
                unsafe { device.destroy_command_pool(cmd_pool, None) };
                if let Ok(mut guard) = alloc_handle.lock() { let _ = guard.free(&mut staging_alloc); }
                unsafe { device.destroy_buffer(staging_buffer, None) };
                render_core::RhiError::Backend {
                    detail: format!("create fence: {r:?}"),
                }
            })?;

        let cmd_buffers = [cmd_buffer];
        let submit_info = vk::SubmitInfo::default().command_buffers(&cmd_buffers);
        // SAFETY: `d.queue` is a valid VkQueue; command buffer is in completed
        // state; fence is valid and unsignaled; submit info is correctly
        // structured.
        unsafe { device.queue_submit(d.queue, &[submit_info], fence) }.map_err(|r| {
            // SAFETY: cleanup only on error; all handles are valid.
            unsafe { device.destroy_fence(fence, None) };
            unsafe { device.destroy_command_pool(cmd_pool, None) };
            if let Ok(mut guard) = alloc_handle.lock() { let _ = guard.free(&mut staging_alloc); }
            unsafe { device.destroy_buffer(staging_buffer, None) };
            render_core::RhiError::Backend {
                detail: format!("queue submit: {r:?}"),
            }
        })?;

        // SAFETY: fence is valid and associated with the submitted work;
        // waiting with `u64::MAX` timeout and `true` (waitAll) is standard.
        unsafe { device.wait_for_fences(&[fence], true, u64::MAX) }.map_err(|r| {
            // SAFETY: cleanup only on error; all handles are valid.
            unsafe { device.destroy_fence(fence, None) };
            unsafe { device.destroy_command_pool(cmd_pool, None) };
            if let Ok(mut guard) = alloc_handle.lock() { let _ = guard.free(&mut staging_alloc); }
            unsafe { device.destroy_buffer(staging_buffer, None) };
            render_core::RhiError::Backend {
                detail: format!("wait fence: {r:?}"),
            }
        })?;

        // SAFETY: fence has been waited on and is no longer needed; destroying
        // a signaled fence is safe.
        unsafe { device.destroy_fence(fence, None) };

        // -----------------------------------------------------------------
        // 5. Map staging buffer and copy pixel data to a Vec<u8>.
        // -----------------------------------------------------------------
        let raw_pixels = match staging_alloc.mapped_slice_mut() {
            Some(slice) => slice[..buffer_size as usize].to_vec(),
            None => {
                // SAFETY: cleanup only on error; all handles are valid.
                unsafe { device.destroy_command_pool(cmd_pool, None) };
                if let Ok(mut guard) = alloc_handle.lock() { let _ = guard.free(&mut staging_alloc); }
                unsafe { device.destroy_buffer(staging_buffer, None) };
                return Err(render_core::RhiError::Backend {
                    detail: "staging buffer is not CPU mapped".into(),
                });
            }
        };

        // -----------------------------------------------------------------
        // 6. Convert BGRA → RGBA if the swapchain uses a B8G8R8A8 format.
        //    The custom allocator's GpuToCpu allocations are host-mapped, so the
        //    raw data is available immediately after fence wait.
        // -----------------------------------------------------------------
        let result: Vec<u8> =
            if sc.format == vk::Format::B8G8R8A8_UNORM || sc.format == vk::Format::B8G8R8A8_SRGB {
                raw_pixels
                    .chunks_exact(4)
                    .flat_map(|p| [p[2], p[1], p[0], p[3]])
                    .collect()
            } else {
                raw_pixels
            };

        // -----------------------------------------------------------------
        // 7. Clean up temporary resources.
        // -----------------------------------------------------------------
        // SAFETY: all objects were created from this device and are no longer
        // in use after fence wait; reverse order of creation is respected.
        unsafe { device.destroy_command_pool(cmd_pool, None) };
        if let Ok(mut guard) = alloc_handle.lock() { let _ = guard.free(&mut staging_alloc); }
        unsafe { device.destroy_buffer(staging_buffer, None) };

        Ok(result)
    }
}

// ============================================================================
// Drop
// ============================================================================

impl Drop for VulkanDevice {
    fn drop(&mut self) {
        // SAFETY: `self.logical_device` is alive by type invariant (ManuallyDrop
        // ensures it is not dropped before this destructor runs).
        let _ = unsafe { self.logical_device.device.device_wait_idle() };
        let d = &self.logical_device.device;
        for fb in self.mvp_framebuffers.drain(..) {
            // SAFETY: `fb` was created by this device and is not yet destroyed.
            unsafe {
                d.destroy_framebuffer(fb, None);
            }
        }
        for fb in self.model_framebuffers.drain(..) {
            // SAFETY: `fb` was created by this device and is not yet destroyed.
            unsafe {
                d.destroy_framebuffer(fb, None);
            }
        }
        if let Some(p) = self.mvp_pipeline.take() {
            // SAFETY: `p` was created by this device and is not yet destroyed.
            unsafe {
                d.destroy_pipeline(p, None);
            }
        }
        if let Some(l) = self.mvp_pipeline_layout.take() {
            // SAFETY: `l` was created by this device and is not yet destroyed.
            unsafe {
                d.destroy_pipeline_layout(l, None);
            }
        }
        if let Some(p) = self.model_pipeline.take() {
            // SAFETY: `p` was created by this device and is not yet destroyed.
            unsafe {
                d.destroy_pipeline(p, None);
            }
        }
        if let Some(l) = self.model_pipeline_layout.take() {
            // SAFETY: `l` was created by this device and is not yet destroyed.
            unsafe {
                d.destroy_pipeline_layout(l, None);
            }
        }
        if let Some(rp) = self.mvp_rp.take() {
            // SAFETY: `rp` was created by this device and is not yet destroyed.
            unsafe {
                d.destroy_render_pass(rp, None);
            }
        }
        if let Some(rp) = self.model_rp.take() {
            // SAFETY: `rp` was created by this device and is not yet destroyed.
            unsafe {
                d.destroy_render_pass(rp, None);
            }
        }
        for fs in self.frame_sync.drain(..) {
            // SAFETY: all handles in `fs` were created by this device and are
            // not yet destroyed; destruction order does not matter among
            // fences, semaphores, and pools.
            unsafe {
                d.destroy_fence(fs.in_flight_fence, None);
                d.destroy_semaphore(fs.image_available, None);
                d.destroy_semaphore(fs.render_finished, None);
                d.destroy_command_pool(fs.command_pool, None);
            }
        }
        for s in self.pipelines.slots.drain(..) {
            if let Some((_, e)) = s {
                // SAFETY: `e.pipeline` was created by this device.
                unsafe {
                    d.destroy_pipeline(e.pipeline, None);
                }
            }
        }
        for s in self.buffers.slots.drain(..) {
            if let Some((_, mut e)) = s {
                // SAFETY: `e.buffer` was created by this device.
                unsafe {
                    d.destroy_buffer(e.buffer, None);
                }
                if let Some(mut a) = e.allocation.take() {
                    if let Ok(mut guard) = e.allocator.lock() {
                        let _ = guard.free(&mut a);
                    }
                }
            }
        }
        for s in self.render_passes.slots.drain(..) {
            if let Some((_, rp)) = s {
                // SAFETY: `rp` was created by this device.
                unsafe {
                    d.destroy_render_pass(rp, None);
                }
            }
        }
        for s in self.framebuffers.slots.drain(..) {
            if let Some((_, fb)) = s {
                // SAFETY: `fb` was created by this device.
                unsafe {
                    d.destroy_framebuffer(fb, None);
                }
            }
        }
        for s in self.pipeline_layouts.slots.drain(..) {
            if let Some((_, e)) = s {
                // SAFETY: `e.layout` was created by this device.
                unsafe {
                    d.destroy_pipeline_layout(e.layout, None);
                }
            }
        }
        self.destroy_shadow_resources();
        self.destroy_descriptor_infra();
        self.destroy_depth_texture();
        drop(self.swapchain.take());
        // SAFETY: all device-child objects have been destroyed above.
        // Destroy VkDevice before VkInstance per Vulkan spec.
        unsafe { self.logical_device.device.destroy_device(None) };
        // Drop the allocator (Device::drop would do this, but we use
        // ManuallyDrop so it won't run automatically).
        drop(self.logical_device.allocator.take());
        drop(self.surface.take());
        drop(self.instance.take());
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Build a [`PipelineColorBlendAttachmentState`] from a mode string.
///
/// Supported modes: `"Alpha"`, `"Additive"`, `"Multiply"`, or `None` / `"Opaque"`.
fn blend_attachment_from_mode(mode: &str) -> vk::PipelineColorBlendAttachmentState {
    let (enable, src_color, dst_color, src_alpha, dst_alpha) = match mode {
        "Alpha" => (true, vk::BlendFactor::SRC_ALPHA, vk::BlendFactor::ONE_MINUS_SRC_ALPHA,
                    vk::BlendFactor::ONE, vk::BlendFactor::ONE_MINUS_SRC_ALPHA),
        "Additive" => (true, vk::BlendFactor::SRC_ALPHA, vk::BlendFactor::ONE,
                       vk::BlendFactor::ONE, vk::BlendFactor::ONE),
        "Multiply" => (true, vk::BlendFactor::ZERO, vk::BlendFactor::SRC_COLOR,
                       vk::BlendFactor::ZERO, vk::BlendFactor::SRC_ALPHA),
        _ => (false, vk::BlendFactor::ONE, vk::BlendFactor::ZERO,
              vk::BlendFactor::ONE, vk::BlendFactor::ZERO),
    };
    vk::PipelineColorBlendAttachmentState::default()
        .blend_enable(enable)
        .src_color_blend_factor(src_color)
        .dst_color_blend_factor(dst_color)
        .color_blend_op(vk::BlendOp::ADD)
        .src_alpha_blend_factor(src_alpha)
        .dst_alpha_blend_factor(dst_alpha)
        .alpha_blend_op(vk::BlendOp::ADD)
        .color_write_mask(vk::ColorComponentFlags::RGBA)
}

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

/// Create a Vulkan shader module from SPIR-V bytecode.
///
/// # Safety
///
/// - `d` must be a valid [`AshDevice`] that has not been destroyed.
/// - `spv` must contain valid SPIR-V binary data (word-aligned, correctly
///   sized for the targeted shader stage).
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
