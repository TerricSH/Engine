//! VulkanDevice — implements `render_core::Device` plus MVP triangle path.

pub(crate) mod depth;
pub(crate) mod descriptor;
pub(crate) mod device_trait;
pub(crate) mod drop;
pub(crate) mod encoder;
pub(crate) mod frame;
pub(crate) mod pipeline;
pub(crate) mod reload;
pub(crate) mod rendering;
pub(crate) mod shadow;
pub(crate) mod slab;

use std::collections::HashMap;
use std::ffi::CStr;
use std::mem::ManuallyDrop;

use ash::vk;
use ash::Device as AshDevice;

use render_core::{
    self, AdapterInfo, BackendKind, FramebufferHandle, ResourceLimits, ShaderFormat, TextureFormat,
};

use crate::device::Device as VkLogicalDevice;
use crate::error::{VkResult, VulkanError};
use crate::instance::Instance;
use crate::surface::Surface;

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
    pub(crate) retired_pipelines: Vec<Vec<vk::Pipeline>>,
    pub(crate) cached_adapter_info: AdapterInfo,
    /// Last swapchain image index acquired (used by read_pixels).
    last_image_index: u32,

    // Phase 2: handle tables
    pub(crate) buffers: Slab<BufEntry>,
    pub(crate) pipelines: Slab<PipeEntry>,
    pub(crate) render_passes: Slab<vk::RenderPass>,
    pub(crate) framebuffers: Slab<vk::Framebuffer>,
    pub(crate) pipeline_layouts: Slab<PlEntry>,

    // P1.2: Shader module storage (handle → (vk::ShaderModule, stage))
    pub(crate) shader_modules: Slab<(vk::ShaderModule, vk::ShaderStageFlags)>,

    // PSO cache (Vulkan pipeline cache for faster subsequent compilations)
    pub(crate) pipeline_cache: vk::PipelineCache,
    /// Path to the PSO cache file (empty = no persistence).
    pub(crate) pso_cache_path: Option<std::path::PathBuf>,

    // Render pass metadata
    pub(crate) rp_has_depth: HashMap<u32, bool>,
    /// Render-pass index → depth-only (no color attachments).
    /// TODO(Gate 3): populate when registering shadow RP in the encoder slab.
    #[allow(dead_code)]
    pub(crate) rp_is_depth_only: HashMap<u32, bool>,

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
        cache_dir: Option<&std::path::Path>,
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
        let mut device = Self {
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
            retired_pipelines: vec![Vec::new(), Vec::new()],
            cached_adapter_info: info,
            last_image_index: 0,
            buffers: Slab::new(),
            pipelines: Slab::new(),
            render_passes: Slab::new(),
            framebuffers: Slab::new(),
            pipeline_layouts: Slab::new(),
            shader_modules: Slab::new(),
            pipeline_cache: vk::PipelineCache::null(),
            pso_cache_path: None,
            rp_has_depth: HashMap::new(),
            rp_is_depth_only: HashMap::new(),
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
        };

        // Phase 3.3: Initialize PSO cache (load from disk if cache_dir provided).
        device.init_pipeline_cache(cache_dir);

        Ok(device)
    }

    /// Create a new VulkanDevice without PSO cache persistence.
    /// Convenience wrapper that passes `cache_dir: None`.
    pub fn new_without_cache(
        display_handle: raw_window_handle::RawDisplayHandle,
        window_handle: raw_window_handle::RawWindowHandle,
        width: u32,
        height: u32,
        enable_validation: bool,
    ) -> Result<Self, VulkanError> {
        Self::new(
            display_handle,
            window_handle,
            width,
            height,
            enable_validation,
            None,
        )
    }

    /// Initialize the pipeline cache, optionally loading from a file.
    pub fn init_pipeline_cache(&mut self, cache_dir: Option<&std::path::Path>) {
        let mut initial_data = Vec::new();
        let d = &self.logical_device.device;

        if let Some(dir) = cache_dir {
            let path = dir.join("pso_cache.bin");
            if let Ok(data) = std::fs::read(&path) {
                // Validate header: first 4 bytes should be "PSC\0" or similar
                // For now just try to use whatever was on disk; corrupt data
                // will be rejected by the driver at creation time.
                if data.len() >= 4 {
                    initial_data = data;
                }
                tracing::info!(size = initial_data.len(), "loaded PSO cache from disk");
            } else {
                tracing::debug!("no existing PSO cache file, starting fresh");
            }
            self.pso_cache_path = Some(path);
        }

        let ci = vk::PipelineCacheCreateInfo::default().initial_data(&initial_data);
        // SAFETY: `d` is a valid device; `ci` is correctly constructed.
        match unsafe { d.create_pipeline_cache(&ci, None) } {
            Ok(cache) => {
                self.pipeline_cache = cache;
                tracing::info!("pipeline cache created");
            }
            Err(r) => {
                tracing::warn!(error = %r, "failed to create pipeline cache, continuing without");
                self.pipeline_cache = vk::PipelineCache::null();
            }
        }
    }

    /// Save the pipeline cache to disk (call on shutdown or after bulk compiles).
    pub fn save_pipeline_cache(&self) {
        let Some(ref path) = self.pso_cache_path else {
            return;
        };
        if self.pipeline_cache == vk::PipelineCache::null() {
            return;
        }
        let d = &self.logical_device.device;
        // SAFETY: `d` is a valid device; `self.pipeline_cache` is valid or null.
        let data = match unsafe { d.get_pipeline_cache_data(self.pipeline_cache) } {
            Ok(d) => d,
            Err(r) => {
                tracing::warn!(error = %r, "failed to get pipeline cache data");
                return;
            }
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match std::fs::write(path, &data) {
            Ok(_) => tracing::info!(bytes = data.len(), "PSO cache saved"),
            Err(e) => tracing::warn!(error = %e, "failed to save PSO cache"),
        }
    }

    pub fn set_mvp_shaders(&mut self, vert: &'static [u8], frag: &'static [u8]) {
        self.mvp_vert_spv = Some(vert);
        self.mvp_frag_spv = Some(frag);
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
        render_pass: vk::RenderPass,
    ) -> VkResult<Vec<FramebufferHandle>> {
        let sc = self.swapchain.as_ref().ok_or(VulkanError::Loader("no swapchain".into()))?;
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
            let (idx, gen) = self.framebuffers.insert(fb);
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
                unsafe { d.destroy_framebuffer(fb, None); }
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

    pub fn resize(&mut self, w: u32, h: u32) {
        self.window_width = w.max(1);
        self.window_height = h.max(1);
        self.minimized = w == 0 || h == 0;
        // SAFETY: `self.logical_device` is alive by type invariant (ManuallyDrop
        // ensures VkLogicalDevice is not dropped before VulkanDevice).
        unsafe { let _ = self.logical_device.device.device_wait_idle(); };
        self.destroy_mvp();
    }
    pub fn wait_idle(&self) {
        // SAFETY: `self.logical_device` is alive by type invariant (ManuallyDrop
        // ensures VkLogicalDevice is not dropped before VulkanDevice).
        unsafe { let _ = self.logical_device.device.device_wait_idle(); };
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
        "Alpha" => (
            true,
            vk::BlendFactor::SRC_ALPHA,
            vk::BlendFactor::ONE_MINUS_SRC_ALPHA,
            vk::BlendFactor::ONE,
            vk::BlendFactor::ONE_MINUS_SRC_ALPHA,
        ),
        "Additive" => (
            true,
            vk::BlendFactor::SRC_ALPHA,
            vk::BlendFactor::ONE,
            vk::BlendFactor::ONE,
            vk::BlendFactor::ONE,
        ),
        "Multiply" => (
            true,
            vk::BlendFactor::ZERO,
            vk::BlendFactor::SRC_COLOR,
            vk::BlendFactor::ZERO,
            vk::BlendFactor::SRC_ALPHA,
        ),
        _ => (
            false,
            vk::BlendFactor::ONE,
            vk::BlendFactor::ZERO,
            vk::BlendFactor::ONE,
            vk::BlendFactor::ZERO,
        ),
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
///
/// Map a resource kind string to a `VkDescriptorType`.
fn resource_kind_to_descriptor_type(kind: &str) -> vk::DescriptorType {
    match kind {
        "uniform_buffer" => vk::DescriptorType::UNIFORM_BUFFER,
        "storage_buffer" => vk::DescriptorType::STORAGE_BUFFER,
        "sampler" | "combined_image_sampler" => vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
        "sampled_image" => vk::DescriptorType::SAMPLED_IMAGE,
        "storage_image" => vk::DescriptorType::STORAGE_IMAGE,
        "uniform_texel_buffer" => vk::DescriptorType::UNIFORM_TEXEL_BUFFER,
        "storage_texel_buffer" => vk::DescriptorType::STORAGE_TEXEL_BUFFER,
        "input_attachment" => vk::DescriptorType::INPUT_ATTACHMENT,
        _ => vk::DescriptorType::UNIFORM_BUFFER,
    }
}

fn parse_topology(s: &Option<String>) -> vk::PrimitiveTopology {
    match s.as_deref() {
        Some("point_list") => vk::PrimitiveTopology::POINT_LIST,
        Some("line_list") => vk::PrimitiveTopology::LINE_LIST,
        Some("line_strip") => vk::PrimitiveTopology::LINE_STRIP,
        Some("triangle_strip") => vk::PrimitiveTopology::TRIANGLE_STRIP,
        Some("triangle_fan") => vk::PrimitiveTopology::TRIANGLE_FAN,
        _ => vk::PrimitiveTopology::TRIANGLE_LIST,
    }
}

fn parse_polygon_mode(s: &Option<String>) -> vk::PolygonMode {
    match s.as_deref() {
        Some("line") => vk::PolygonMode::LINE,
        Some("point") => vk::PolygonMode::POINT,
        _ => vk::PolygonMode::FILL,
    }
}

fn parse_sample_count(s: Option<u8>) -> vk::SampleCountFlags {
    match s {
        Some(2) => vk::SampleCountFlags::TYPE_2,
        Some(4) => vk::SampleCountFlags::TYPE_4,
        Some(8) => vk::SampleCountFlags::TYPE_8,
        Some(16) => vk::SampleCountFlags::TYPE_16,
        Some(32) => vk::SampleCountFlags::TYPE_32,
        Some(64) => vk::SampleCountFlags::TYPE_64,
        _ => vk::SampleCountFlags::TYPE_1,
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
