//! VulkanDevice — implements the Vulkan `render_core::Device` backend.

pub(crate) mod depth;
pub(crate) mod descriptor;
pub(crate) mod device_trait;
pub(crate) mod drop;
pub(crate) mod encoder;
pub(crate) mod env;
pub(crate) mod frame;
pub(crate) mod graph_barriers;
pub(crate) mod hdr;
pub(crate) mod reload;
pub(crate) mod shadow;
pub(crate) mod slab;
pub(crate) mod texture;
pub(crate) mod ui;

mod base;

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

use self::slab::{BufEntry, FbEntry, FrameSync, PipeEntry, PlEntry, Slab, TexEntry};
use base::{
    blend_attachment_from_mode, compare_op, default_dep, mk_sm, mrt_blend_attachments,
    parse_polygon_mode, parse_sample_count, parse_topology, resource_kind_to_descriptor_type, vfmt,
};

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
    /// A suboptimal present keeps the old swapchain alive until SceneRenderer
    /// can first destroy framebuffers that reference its image views.
    pub(crate) swapchain_recreate_pending: bool,

    // Canonical SceneRenderer shaders used by the HDR forward path.
    pub(crate) forward_vert_spv: Option<Vec<u8>>,
    pub(crate) forward_frag_spv: Option<Vec<u8>>,
    pub(crate) skinned_vert_spv: Option<Vec<u8>>,
    pub(crate) vfx_billboard_vert_spv: Option<Vec<u8>>,
    pub(crate) gpu_vfx_billboard_vert_spv: Option<Vec<u8>>,
    pub(crate) vfx_billboard_frag_spv: Option<Vec<u8>>,
    pub(crate) instanced_vert_spv: Option<Vec<u8>>,
    pub(crate) skybox_vert_spv: Option<Vec<u8>>,
    pub(crate) skybox_frag_spv: Option<Vec<u8>>,

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
    pub(crate) rhi_textures: Slab<TexEntry>,
    pub(crate) pipelines: Slab<PipeEntry>,
    pub(crate) render_passes: Slab<vk::RenderPass>,
    pub(crate) framebuffers: Slab<FbEntry>,
    pub(crate) pipeline_layouts: Slab<PlEntry>,

    // P1.2: Shader module storage (handle → (vk::ShaderModule, stage))
    pub(crate) shader_modules: Slab<(vk::ShaderModule, vk::ShaderStageFlags)>,

    // PSO cache (Vulkan pipeline cache for faster subsequent compilations)
    pub(crate) pipeline_cache: vk::PipelineCache,
    /// Path to the PSO cache file (empty = no persistence).
    pub(crate) pso_cache_path: Option<std::path::PathBuf>,

    // Render pass metadata
    pub(crate) rp_has_depth: HashMap<u32, bool>,
    pub(crate) rp_color_formats: HashMap<u32, Vec<vk::Format>>,
    pub(crate) rp_depth_formats: HashMap<u32, vk::Format>,
    pub(crate) rp_sample_counts: HashMap<u32, u8>,

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
    pub(crate) cluster_grid_ssbo: Option<vk::Buffer>,
    pub(crate) cluster_grid_ssbo_allocation: Option<crate::allocator::Allocation>,
    pub(crate) cluster_index_ssbo: Option<vk::Buffer>,
    pub(crate) cluster_index_ssbo_allocation: Option<crate::allocator::Allocation>,

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
    /// Multisampled render target resolved into `hdr_color_image`.
    pub(crate) hdr_msaa_color_image: Option<vk::Image>,
    pub(crate) hdr_msaa_color_view: Option<vk::ImageView>,
    pub(crate) hdr_msaa_color_allocation: Option<crate::allocator::Allocation>,
    pub(crate) oit_accum_image: Option<vk::Image>,
    pub(crate) oit_accum_view: Option<vk::ImageView>,
    pub(crate) oit_accum_allocation: Option<crate::allocator::Allocation>,
    pub(crate) oit_msaa_accum_image: Option<vk::Image>,
    pub(crate) oit_msaa_accum_view: Option<vk::ImageView>,
    pub(crate) oit_msaa_accum_allocation: Option<crate::allocator::Allocation>,
    pub(crate) oit_optical_depth_image: Option<vk::Image>,
    pub(crate) oit_optical_depth_view: Option<vk::ImageView>,
    pub(crate) oit_optical_depth_allocation: Option<crate::allocator::Allocation>,
    pub(crate) oit_msaa_optical_depth_image: Option<vk::Image>,
    pub(crate) oit_msaa_optical_depth_view: Option<vk::ImageView>,
    pub(crate) oit_msaa_optical_depth_allocation: Option<crate::allocator::Allocation>,
    /// Multisampled depth attachment used by the HDR forward pass. The
    /// legacy direct-to-swapchain pass keeps its single-sampled depth image.
    pub(crate) hdr_msaa_depth_image: Option<vk::Image>,
    pub(crate) hdr_msaa_depth_view: Option<vk::ImageView>,
    pub(crate) hdr_msaa_depth_allocation: Option<crate::allocator::Allocation>,
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
    pub(crate) hdr_forward_double_sided_pipeline: Option<vk::Pipeline>,
    pub(crate) hdr_forward_blend_pipeline: Option<vk::Pipeline>,
    pub(crate) hdr_forward_blend_double_sided_pipeline: Option<vk::Pipeline>,
    pub(crate) hdr_forward_oit_pipeline: Option<vk::Pipeline>,
    pub(crate) hdr_forward_oit_double_sided_pipeline: Option<vk::Pipeline>,
    pub(crate) hdr_forward_additive_pipeline: Option<vk::Pipeline>,
    pub(crate) hdr_forward_additive_double_sided_pipeline: Option<vk::Pipeline>,
    /// Instanced camera-facing particle pipeline. It shares the forward
    /// fragment material/light path but disables depth writes.
    pub(crate) hdr_vfx_billboard_pipeline: Option<vk::Pipeline>,
    pub(crate) hdr_vfx_billboard_additive_pipeline: Option<vk::Pipeline>,
    pub(crate) hdr_vfx_billboard_oit_pipeline: Option<vk::Pipeline>,
    pub(crate) hdr_gpu_vfx_billboard_pipeline: Option<vk::Pipeline>,
    pub(crate) hdr_gpu_vfx_billboard_additive_pipeline: Option<vk::Pipeline>,
    pub(crate) hdr_gpu_vfx_billboard_oit_pipeline: Option<vk::Pipeline>,
    pub(crate) hdr_instanced_pipeline: Option<vk::Pipeline>,
    pub(crate) hdr_instanced_double_sided_pipeline: Option<vk::Pipeline>,
    /// Skybox pipeline rendered before opaque geometry in the HDR pass.
    pub(crate) hdr_skybox_pipeline: Option<vk::Pipeline>,
    pub(crate) hdr_forward_pipeline_layout: Option<vk::PipelineLayout>,
    /// Framebuffer for forward HDR pass (HDR color view + depth view).
    pub(crate) hdr_forward_fb: Option<vk::Framebuffer>,

    /// What sample count the HDR forward resources were created with.
    pub(crate) hdr_msaa_samples: vk::SampleCountFlags,

    // Material texture cache (Phase 3.1)
    /// Uploaded GPU textures indexed by asset ID string.
    pub(crate) textures: HashMap<String, GpuTexture>,

    // Editor UI overlay (load-op pass over the tone-mapped swapchain image).
    pub(crate) ui_overlay_rp: Option<vk::RenderPass>,
    pub(crate) ui_overlay_pipeline_layout: Option<vk::PipelineLayout>,
    pub(crate) ui_overlay_pipeline: Option<vk::Pipeline>,
    pub(crate) ui_overlay_desc_layout: Option<vk::DescriptorSetLayout>,
    pub(crate) ui_overlay_desc_pool: Option<vk::DescriptorPool>,
    pub(crate) ui_overlay_desc_sets: HashMap<String, vk::DescriptorSet>,
    pub(crate) ui_overlay_framebuffers: Vec<vk::Framebuffer>,
}
