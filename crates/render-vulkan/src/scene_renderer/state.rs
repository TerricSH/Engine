use super::*;

/// Vulkan implementation of [`BackendRenderer`].
///
/// Wraps a [`VulkanDevice`] and processes [`RenderFrameInput`] by creating
/// GPU buffers for each referenced mesh on first encounter and then issuing
/// indexed draw calls through a forward-shaded graphics pipeline.
pub struct SceneRenderer {
    pub(super) device: VulkanDevice,
    pub(super) initialized: bool,

    /// Cache of loaded meshes indexed by their [`AssetId`](engine_serialize::AssetId) string.
    pub(super) meshes: BTreeMap<String, GpuMesh>,
    pub(super) texture_uploads: HashMap<String, UploadedResourceState>,
    pub(super) environment_uploads: HashMap<String, EnvironmentMapUpload>,
    pub(super) environment_revisions: HashMap<String, UploadedResourceState>,
    pub(super) active_environment_id: Option<String>,
    pub(super) morph_target_sets: HashMap<String, GpuMorphTargetSet>,
    pub(super) fallback_morph_buffer: Option<(BufferHandle, vk::Buffer)>,
    pub(super) uploaded_materials: HashMap<String, UploadedMaterialState>,

    /// Cache of material descriptor sets + buffers, keyed by material_id.
    /// Limited to [`MAX_MATERIALS`] entries; oldest entries evicted when full.
    pub(super) material_cache: HashMap<String, MaterialCacheEntry>,
    /// Insertion order for LRU eviction of the material cache.
    pub(super) material_cache_order: Vec<String>,

    /// Cache of bone palette UBO buffers, keyed by skeleton_id (AssetId string).
    /// Each entry contains the BufferHandle (for data updates) and the raw VkBuffer (for descriptor binding).
    pub(super) bone_palette_buffers: HashMap<String, CachedBoneBuffer>,
    /// Insertion order for LRU eviction of the bone buffer cache.
    pub(super) bone_palette_buffers_order: Vec<String>,

    /// Cache of combined skinning descriptor sets, keyed by "material_id:skeleton_id".
    /// Each entry has a descriptor set (material UBO at binding=0 + bone UBO at binding=2)
    /// and the raw VkBuffer for the bone palette.
    pub(super) skinned_desc_cache: HashMap<String, BonePaletteCacheEntry>,
    /// Insertion order for LRU eviction of the skinned descriptor cache.
    pub(super) skinned_desc_cache_order: Vec<String>,

    pub(super) rp: Option<RenderPassHandle>,
    pub(super) pll: Option<PipelineLayoutHandle>,
    pub(super) forward_shader_modules: Vec<ShaderModuleHandle>,
    pub(super) skinned_shader_modules: Vec<ShaderModuleHandle>,

    /// Per-swapchain-image framebuffer handles (color + depth).
    pub(super) framebuffers: Vec<FramebufferHandle>,
    /// Index into `framebuffers` for the current swapchain image.
    pub(super) cur_fb_index: u32,

    // Frame lifecycle state (stored between begin_frame / execute_pass / end_frame).
    pub(super) cur_sc: Option<SwapchainHandle>,
    pub(super) cur_ii: Option<u32>,
    pub(super) cur_enc: Option<Box<dyn CommandEncoder>>,

    /// Window dimensions (logical pixels).
    pub(super) width: u32,
    pub(super) height: u32,

    /// Registry of pluggable render passes.
    pub(super) pass_registry: PassRegistry,

    /// One CPU-visible UI vertex buffer per in-flight frame. Reusing a single
    /// buffer would race the other frame slot while the GPU is reading it.
    pub(super) ui_vbs: [Option<BufferHandle>; 2],
    pub(super) ui_vb_capacities: [u64; 2],
    /// Per-frame GPU particle instance streams. They are isolated by
    /// in-flight frame slot for the same reason as the UI stream.
    pub(super) particle_instance_vbs: [Option<BufferHandle>; 2],
    pub(super) particle_instance_capacities: [u64; 2],
    pub(super) static_instance_vbs: [Option<BufferHandle>; 2],
    pub(super) static_instance_capacities: [u64; 2],

    /// Per-pass GPU timestamp state machine (ENG-04). Async read-back lands
    /// frames-in-flight frames after recording; unavailable/disabled states
    /// degrade to status reporting only.
    pub(super) gpu_timestamps: crate::timestamps::GpuTimestampProfiler,
    /// Lazily created timestamp query pools (one per frame-in-flight slot).
    pub(super) timestamp_pools: crate::timestamps::TimestampQueryPools,
    /// Engine configuration switch for GPU timestamps.
    pub(super) gpu_timing_enabled: bool,
    /// Whether device timestamp support was already evaluated.
    pub(super) gpu_timing_configured: bool,
}
