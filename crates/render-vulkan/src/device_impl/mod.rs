//! VulkanDevice — implements `render_core::Device` plus MVP triangle path.

pub(crate) mod depth;
pub(crate) mod descriptor;
pub(crate) mod device_trait;
pub(crate) mod drop;
pub(crate) mod encoder;
pub(crate) mod env;
pub(crate) mod frame;
pub(crate) mod hdr;
pub(crate) mod pipeline;
pub(crate) mod post_process;
pub(crate) mod reload;
pub(crate) mod rendering;
pub(crate) mod shadow;
pub(crate) mod slab;
pub(crate) mod texture;

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
// GpuTexture — GPU-side resources for a sampled 2D texture
// ============================================================================

/// GPU resources for a single 2D texture (image, view, allocation, sampler).
pub(crate) struct GpuTexture {
    pub image: vk::Image,
    pub view: vk::ImageView,
    pub allocation: crate::allocator::Allocation,
    pub sampler: vk::Sampler,
}

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
    pub(crate) skinned_vert_spv: Option<&'static [u8]>,

    // Model rendering pipeline (forward shaders + vertex input state)
    pub(crate) model_pipeline: Option<vk::Pipeline>,
    pub(crate) model_pipeline_layout: Option<vk::PipelineLayout>,
    pub(crate) model_rp: Option<vk::RenderPass>,
    pub(crate) model_framebuffers: Vec<vk::Framebuffer>,

    // Phase 5.2: Async compute queue
    pub(crate) compute_queue: Option<vk::Queue>,
    pub(crate) compute_pool: Option<vk::CommandPool>,
    pub(crate) compute_cmd_buffer: Option<vk::CommandBuffer>,

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

    // Environment cubemap (IBL, set=1 binding=1)
    pub(crate) env_cubemap: Option<vk::Image>,
    pub(crate) env_cubemap_view: Option<vk::ImageView>,
    pub(crate) env_cubemap_allocation: Option<crate::allocator::Allocation>,
    pub(crate) env_sampler: Option<vk::Sampler>,

    // Material descriptor infrastructure (set=2, binding 0 = UBO, binding 1 = texture)
    pub(crate) material_desc_set_layout: Option<vk::DescriptorSetLayout>,
    pub(crate) material_desc_pool: Option<vk::DescriptorPool>,

    // Light storage buffer (set=1, binding=2) — clustered lighting
    pub(crate) light_ssbo: Option<vk::Buffer>,
    pub(crate) light_ssbo_allocation: Option<crate::allocator::Allocation>,
    pub(crate) light_ssbo_size: vk::DeviceSize,
    pub(crate) max_lights: u32,

    // Phase 5.1: Indirect draw buffer (for GPU-driven culling)
    pub(crate) indirect_draw_buffer: Option<vk::Buffer>,
    pub(crate) indirect_draw_alloc: Option<crate::allocator::Allocation>,
    /// Buffer for compute-shader cull arguments (future use).
    pub(crate) cull_args_buffer: Option<vk::Buffer>,
    pub(crate) cull_args_alloc: Option<crate::allocator::Allocation>,

    // Shadow mapping (directional light, 2048×2048, 3-cascade CSM)
    pub(crate) shadow_map: Option<vk::Image>,
    /// Layered image view (TYPE_2D_ARRAY) for shader sampling.
    pub(crate) shadow_map_view: Option<vk::ImageView>,
    /// Per-layer image views (TYPE_2D) for cascade framebuffer attachments.
    pub(crate) shadow_layer_views: Vec<vk::ImageView>,
    pub(crate) shadow_allocation: Option<crate::allocator::Allocation>,
    pub(crate) shadow_sampler: Option<vk::Sampler>,
    pub(crate) shadow_rp: Option<vk::RenderPass>,
    pub(crate) shadow_pipeline_layout: Option<vk::PipelineLayout>,
    pub(crate) shadow_pipeline: Option<vk::Pipeline>,
    /// Per-cascade framebuffers (one per array layer).
    pub(crate) shadow_fbs: Vec<vk::Framebuffer>,
    pub(crate) shadow_desc_set: Option<vk::DescriptorSet>,
    pub(crate) shadow_desc_layout: Option<vk::DescriptorSetLayout>,
    pub(crate) shadow_desc_pool: Option<vk::DescriptorPool>,
    /// Pipeline layout containing only set=1 (shadow map), used to bind the
    /// shadow descriptor set in `begin_frame` before the encoder takes over.
    pub(crate) shadow_bind_layout: Option<vk::PipelineLayout>,

    // HDR offscreen rendering (Phase 2.1)
    pub(crate) hdr_color_image: Option<vk::Image>,
    pub(crate) hdr_color_view: Option<vk::ImageView>,
    pub(crate) hdr_color_allocation: Option<crate::allocator::Allocation>,
    pub(crate) hdr_color_sampler: Option<vk::Sampler>,
    pub(crate) tone_rp: Option<vk::RenderPass>,
    pub(crate) tone_pipeline: Option<vk::Pipeline>,
    pub(crate) tone_pipeline_layout: Option<vk::PipelineLayout>,
    pub(crate) tone_framebuffers: Vec<vk::Framebuffer>,
    /// Descriptor set + infrastructure for HDR texture binding in tonemap.
    pub(crate) tone_desc_set: Option<vk::DescriptorSet>,
    pub(crate) tone_desc_pool: Option<vk::DescriptorPool>,
    pub(crate) tone_desc_layout: Option<vk::DescriptorSetLayout>,
    /// Forward HDR render pass (RGBA16F color + D32 depth).
    pub(crate) hdr_forward_rp: Option<vk::RenderPass>,
    /// Forward HDR pipeline (targets hdr_forward_rp).
    pub(crate) hdr_forward_pipeline: Option<vk::Pipeline>,
    pub(crate) hdr_forward_pipeline_layout: Option<vk::PipelineLayout>,
    /// Framebuffer for forward HDR pass (HDR color view + depth view).
    pub(crate) hdr_forward_fb: Option<vk::Framebuffer>,

    /// What sample count the HDR forward resources were created with.
    pub(crate) hdr_msaa_samples: vk::SampleCountFlags,

    // MSAA resources (Phase 4.2)
    /// Maximum sample count supported by the device.
    pub(crate) max_msaa_samples: vk::SampleCountFlags,
    /// Multisampled color image for HDR forward pass (MSAA > 1 only).
    pub(crate) msaa_color_image: Option<vk::Image>,
    pub(crate) msaa_color_view: Option<vk::ImageView>,
    pub(crate) msaa_color_allocation: Option<crate::allocator::Allocation>,
    /// Multisampled depth image for HDR forward pass (MSAA > 1 only).
    pub(crate) msaa_depth_image: Option<vk::Image>,
    pub(crate) msaa_depth_view: Option<vk::ImageView>,
    pub(crate) msaa_depth_allocation: Option<crate::allocator::Allocation>,

    // Material texture cache (Phase 3.1)
    /// Uploaded GPU textures indexed by asset ID string.
    pub(crate) textures: HashMap<String, GpuTexture>,
    /// Cached descriptor sets per material ID (allocated from material_desc_pool).
    pub(crate) material_desc_sets: HashMap<String, vk::DescriptorSet>,

    // Post-processing (Phase 4.5)
    // -- Bloom resources --
    /// Downsample compute pipelines (one per mip level).
    pub(crate) bloom_downsample_pipelines: Vec<vk::Pipeline>,
    /// Upsample compute pipelines (one per mip level).
    pub(crate) bloom_upsample_pipelines: Vec<vk::Pipeline>,
    /// Pipeline layout shared by all bloom compute shaders.
    pub(crate) bloom_pipeline_layout: Option<vk::PipelineLayout>,
    /// Bloom mip-chain images (RGBA16F, halves per level).
    pub(crate) bloom_images: Vec<vk::Image>,
    pub(crate) bloom_image_views: Vec<vk::ImageView>,
    pub(crate) bloom_allocations: Vec<crate::allocator::Allocation>,
    /// Bloom mip descriptor sets (storage image access per level).
    pub(crate) bloom_desc_sets: Vec<vk::DescriptorSet>,
    pub(crate) bloom_desc_pool: Option<vk::DescriptorPool>,
    pub(crate) bloom_desc_layout: Option<vk::DescriptorSetLayout>,
    /// Sampler for bloom texture reads (linear clamp).
    pub(crate) bloom_sampler: Option<vk::Sampler>,

    // -- SSAO resources --
    /// SSAO compute pipeline.
    pub(crate) ssao_pipeline: Option<vk::Pipeline>,
    /// SSAO pipeline layout.
    pub(crate) ssao_pipeline_layout: Option<vk::PipelineLayout>,
    /// SSAO noise texture (4x4 random rotations, RGBA8).
    pub(crate) ssao_noise_image: Option<vk::Image>,
    pub(crate) ssao_noise_view: Option<vk::ImageView>,
    pub(crate) ssao_noise_allocation: Option<crate::allocator::Allocation>,
    /// SSAO output texture (R8_UNORM occlusion factor).
    pub(crate) ssao_output_image: Option<vk::Image>,
    pub(crate) ssao_output_view: Option<vk::ImageView>,
    pub(crate) ssao_output_allocation: Option<crate::allocator::Allocation>,
    /// Descriptor set for SSAO (depth + noise + output).
    pub(crate) ssao_desc_set: Option<vk::DescriptorSet>,
    pub(crate) ssao_desc_pool: Option<vk::DescriptorPool>,
    pub(crate) ssao_desc_layout: Option<vk::DescriptorSetLayout>,
    /// Sampler for SSAO depth reads (nearest clamp).
    pub(crate) ssao_depth_sampler: Option<vk::Sampler>,
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

        // ---- Determine max MSAA sample count ----
        let sample_flags = adapter.properties.limits.framebuffer_color_sample_counts;
        let max_msaa = if sample_flags.contains(vk::SampleCountFlags::TYPE_8) {
            vk::SampleCountFlags::TYPE_8
        } else if sample_flags.contains(vk::SampleCountFlags::TYPE_4) {
            vk::SampleCountFlags::TYPE_4
        } else if sample_flags.contains(vk::SampleCountFlags::TYPE_2) {
            vk::SampleCountFlags::TYPE_2
        } else {
            vk::SampleCountFlags::TYPE_1
        };
        let max_sample_count_u8 = match max_msaa {
            vk::SampleCountFlags::TYPE_8 => 8u8,
            vk::SampleCountFlags::TYPE_4 => 4u8,
            vk::SampleCountFlags::TYPE_2 => 2u8,
            _ => 1u8,
        };

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
                    max_sample_count: max_sample_count_u8,
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
            skinned_vert_spv: None,
            model_pipeline: None,
            model_pipeline_layout: None,
            model_rp: None,
            model_framebuffers: Vec::new(),
            compute_queue: None,
            compute_pool: None,
            compute_cmd_buffer: None,
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
            ubo_size: 512,
            ubo_allocations: Vec::new(),
            ubo_alignment: 256,
            depth_image: None,
            depth_image_view: None,
            depth_allocation: None,

            // Environment cubemap (IBL)
            env_cubemap: None,
            env_cubemap_view: None,
            env_cubemap_allocation: None,
            env_sampler: None,

            // Material descriptor infrastructure (set=2)
            material_desc_set_layout: None,
            material_desc_pool: None,

            // Shadow mapping
            shadow_map: None,
            shadow_map_view: None,
            shadow_layer_views: Vec::new(),
            shadow_allocation: None,
            shadow_sampler: None,
            shadow_rp: None,
            shadow_pipeline_layout: None,
            shadow_pipeline: None,
            shadow_fbs: Vec::new(),
            shadow_desc_set: None,
            shadow_desc_layout: None,
            shadow_desc_pool: None,
            shadow_bind_layout: None,

            // HDR offscreen rendering
            hdr_color_image: None,
            hdr_color_view: None,
            hdr_color_allocation: None,
            hdr_color_sampler: None,
            tone_rp: None,
            tone_pipeline: None,
            tone_pipeline_layout: None,
            tone_framebuffers: Vec::new(),
            tone_desc_set: None,
            tone_desc_pool: None,
            tone_desc_layout: None,
            hdr_forward_rp: None,
            hdr_forward_pipeline: None,
            hdr_forward_pipeline_layout: None,
            hdr_forward_fb: None,
            hdr_msaa_samples: vk::SampleCountFlags::TYPE_1,

            // MSAA resources (Phase 4.2)
            max_msaa_samples: max_msaa,
            msaa_color_image: None,
            msaa_color_view: None,
            msaa_color_allocation: None,
            msaa_depth_image: None,
            msaa_depth_view: None,
            msaa_depth_allocation: None,

            // Material texture cache (Phase 3.1)
            textures: HashMap::new(),
            material_desc_sets: HashMap::new(),

            // Light SSBO (Phase 4.3)
            light_ssbo: None,
            light_ssbo_allocation: None,
            light_ssbo_size: 16384, // 256 lights × 64 bytes each
            max_lights: 256,

            // Phase 5.1: Indirect draw buffers
            indirect_draw_buffer: None,
            indirect_draw_alloc: None,
            cull_args_buffer: None,
            cull_args_alloc: None,

            // Post-processing (Phase 4.5)
            bloom_downsample_pipelines: Vec::new(),
            bloom_upsample_pipelines: Vec::new(),
            bloom_pipeline_layout: None,
            bloom_images: Vec::new(),
            bloom_image_views: Vec::new(),
            bloom_allocations: Vec::new(),
            bloom_desc_sets: Vec::new(),
            bloom_desc_pool: None,
            bloom_desc_layout: None,
            bloom_sampler: None,
            ssao_pipeline: None,
            ssao_pipeline_layout: None,
            ssao_noise_image: None,
            ssao_noise_view: None,
            ssao_noise_allocation: None,
            ssao_output_image: None,
            ssao_output_view: None,
            ssao_output_allocation: None,
            ssao_desc_set: None,
            ssao_desc_pool: None,
            ssao_desc_layout: None,
            ssao_depth_sampler: None,
        };

        // Phase 3.3: Initialize PSO cache (load from disk if cache_dir provided).
        device.init_pipeline_cache(cache_dir);

        // Phase 5.2: Create compute queue, pool, and command buffer.
        {
            let d = &device.logical_device.device;
            let compute_queue = device.logical_device.compute_queue;
            let compute_qfi = device.logical_device.compute_queue_family_index;
            device.compute_queue = compute_queue;

            // SAFETY: `d` is a valid AshDevice; `compute_qfi` is a valid queue
            // family index for this device; `None` means no custom allocator.
            let cp = unsafe {
                d.create_command_pool(
                    &vk::CommandPoolCreateInfo::default()
                        .queue_family_index(compute_qfi)
                        .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                    None,
                )
            }
            .map_err(|r| VulkanError::vk("ccp_compute", r))?;

            // SAFETY: `cp` was just created and is valid; allocate one primary
            // command buffer from it.
            let cbs = unsafe {
                d.allocate_command_buffers(
                    &vk::CommandBufferAllocateInfo::default()
                        .command_pool(cp)
                        .level(vk::CommandBufferLevel::PRIMARY)
                        .command_buffer_count(1),
                )
            }
            .map_err(|r| VulkanError::vk("acb_compute", r))?;

            device.compute_pool = Some(cp);
            device.compute_cmd_buffer = Some(cbs[0]);
        }

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

    pub fn set_skinned_vertex_shader(&mut self, vert: &'static [u8]) {
        self.skinned_vert_spv = Some(vert);
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
                unsafe {
                    d.destroy_framebuffer(fb, None);
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

    /// Create (or recreate) the indirect draw and cull-args buffers.
    ///
    /// `max_draws` is the maximum number of `VkDrawIndexedIndirectCommand`
    /// entries the indirect buffer should hold. The buffer is created with
    /// `INDIRECT_BUFFER` usage and `CpuToGpu` memory so the CPU can fill it
    /// each frame before issuing `draw_indexed_indirect`.
    ///
    /// Idempotent: if buffers already exist they are destroyed first.
    pub(crate) fn create_indirect_buffers(&mut self, max_draws: u32) -> VkResult<()> {
        // Destroy previous buffers before creating new immutable references.
        self.destroy_indirect_buffers();

        let d = &self.logical_device.device;
        let allocator = self.logical_device.allocator();

        let indirect_size = max_draws as u64 * 20; // 20 bytes per VkDrawIndexedIndirectCommand

        // ── Indirect draw buffer ────────────────────────────────────────
        let ibi = vk::BufferCreateInfo::default()
            .size(indirect_size.max(64))
            .usage(vk::BufferUsageFlags::INDIRECT_BUFFER | vk::BufferUsageFlags::TRANSFER_DST)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        let indirect_buf = unsafe { d.create_buffer(&ibi, None) }
            .map_err(|r| VulkanError::vk("create_indirect_draw_buffer", r))?;
        let req = unsafe { d.get_buffer_memory_requirements(indirect_buf) };
        let indirect_alloc = allocator
            .lock()
            .map_err(|e| VulkanError::Loader(format!("allocator lock: {e}")))?
            .allocate(&crate::allocator::AllocationCreateDesc {
                name: "indirect-draw-buffer",
                requirements: req,
                location: crate::allocator::MemoryLocation::CpuToGpu,
                linear: true,
                allocation_scheme: crate::allocator::AllocationScheme::GpuAllocatorManaged,
            })
            .map_err(|e| VulkanError::Allocation(e.to_string()))?;
        unsafe {
            d.bind_buffer_memory(
                indirect_buf,
                indirect_alloc.memory(),
                indirect_alloc.offset(),
            )
        }
        .map_err(|r| VulkanError::vk("bind_indirect_draw_buffer", r))?;

        // ── Cull-args buffer (compute shader indirect args, Phase 5.2+) ─
        let cabi = vk::BufferCreateInfo::default()
            .size(64) // Large enough for DispatchIndirectCommand (12 B) or DrawIndirectCommand
            .usage(vk::BufferUsageFlags::INDIRECT_BUFFER | vk::BufferUsageFlags::STORAGE_BUFFER)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        let cull_buf = unsafe { d.create_buffer(&cabi, None) }
            .map_err(|r| VulkanError::vk("create_cull_args_buffer", r))?;
        let cull_req = unsafe { d.get_buffer_memory_requirements(cull_buf) };
        let cull_alloc = allocator
            .lock()
            .map_err(|e| VulkanError::Loader(format!("allocator lock: {e}")))?
            .allocate(&crate::allocator::AllocationCreateDesc {
                name: "cull-args-buffer",
                requirements: cull_req,
                location: crate::allocator::MemoryLocation::CpuToGpu,
                linear: true,
                allocation_scheme: crate::allocator::AllocationScheme::GpuAllocatorManaged,
            })
            .map_err(|e| VulkanError::Allocation(e.to_string()))?;
        unsafe { d.bind_buffer_memory(cull_buf, cull_alloc.memory(), cull_alloc.offset()) }
            .map_err(|r| VulkanError::vk("bind_cull_args_buffer", r))?;

        self.indirect_draw_buffer = Some(indirect_buf);
        self.indirect_draw_alloc = Some(indirect_alloc);
        self.cull_args_buffer = Some(cull_buf);
        self.cull_args_alloc = Some(cull_alloc);

        Ok(())
    }

    /// Write draw-command data into the indirect draw buffer at the given
    /// byte offset.  Silently returns if the buffer has not been created.
    pub(crate) fn write_indirect_draw_buffer(&mut self, data: &[u8], offset: u64) {
        if let Some(ref mut alloc) = self.indirect_draw_alloc {
            if let Some(slice) = alloc.mapped_slice_mut() {
                let start = offset as usize;
                let end = (start + data.len()).min(slice.len());
                slice[start..end].copy_from_slice(&data[..end - start]);
            }
        }
    }

    /// Destroy the indirect draw and cull-args buffers.
    pub(crate) fn destroy_indirect_buffers(&mut self) {
        let d = &self.logical_device.device;
        if let Some(buf) = self.indirect_draw_buffer.take() {
            unsafe {
                d.destroy_buffer(buf, None);
            }
        }
        if let Some(mut a) = self.indirect_draw_alloc.take() {
            if let Ok(mut guard) = self.logical_device.allocator().lock() {
                guard.free(&mut a);
            }
        }
        if let Some(buf) = self.cull_args_buffer.take() {
            unsafe {
                d.destroy_buffer(buf, None);
            }
        }
        if let Some(mut a) = self.cull_args_alloc.take() {
            if let Ok(mut guard) = self.logical_device.allocator().lock() {
                guard.free(&mut a);
            }
        }
    }

    /// Return the MSAA sample count flags for the given requested count,
    /// capped to the device's maximum.
    pub(crate) fn msaa_samples(&self, requested: u8) -> vk::SampleCountFlags {
        let capped = match requested {
            8 if self.max_msaa_samples.contains(vk::SampleCountFlags::TYPE_8) => 8,
            4 if self.max_msaa_samples.contains(vk::SampleCountFlags::TYPE_4) => 4,
            2 if self.max_msaa_samples.contains(vk::SampleCountFlags::TYPE_2) => 2,
            _ => 1,
        };
        match capped {
            8 => vk::SampleCountFlags::TYPE_8,
            4 => vk::SampleCountFlags::TYPE_4,
            2 => vk::SampleCountFlags::TYPE_2,
            _ => vk::SampleCountFlags::TYPE_1,
        }
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
        self.destroy_mvp();
    }
    pub fn wait_idle(&self) {
        // SAFETY: `self.logical_device` is alive by type invariant (ManuallyDrop
        // ensures VkLogicalDevice is not dropped before VulkanDevice).
        unsafe {
            let _ = self.logical_device.device.device_wait_idle();
        };
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
        "uint32x4" => vk::Format::R32G32B32A32_UINT,
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
