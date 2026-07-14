//! Vulkan implementation of [`BackendRenderer`].
//!
//! Consumes [`RenderFrameInput`] and renders each drawable through a
//! forward-shaded pipeline with lighting.
//!
//! Resources are uploaded through the typed renderer contract before a frame
//! references them. The render graph records shadow, HDR forward, tone-map and
//! present passes with explicit abort handling when any pass fails.
//!
//! The portable material contract currently accepts opaque, single-sided PBR
//! base-color materials. Unsupported alpha and raster states are rejected by
//! the contract instead of being rendered with an implicit approximation.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use ash::vk;
use glam::Mat4;

use engine_renderer::{
    render_graph, AssetId, BackendRenderer, Diagnostic, DiagnosticSeverity, FrameStats, LightItem,
    LightKind, MaterialBinding, MaterialPipelineContext, MaterialResolver, MaterialUpload,
    MeshUpload, ParamBlock, PassRegistry, RenderFrameInput, RenderableItem, ResourceKind,
    ResourceRemoval, SamplerAddressMode, SamplerFilter, ShadowMode, SkinnedItem, TextureSlot,
    TextureUpload, Transparency, UiBatch, UploadReceipt,
};
use render_core::{
    self, BindGroupLayoutBinding, BindGroupLayoutDescriptor, BufferDescriptor, BufferHandle,
    CommandEncoder, Device, FramebufferHandle, IndexFormat, MemoryHint, PipelineHandle,
    PipelineLayoutDescriptor, PipelineLayoutHandle, PipelineVariantKey, PushConstantRange,
    RenderPassDescriptor, RenderPassHandle, SwapchainDescriptor, SwapchainHandle, TextureFormat,
    VertexAttribute, VertexLayout,
};

#[cfg(test)]
use render_core::PipelineDescriptor;

use crate::device_impl::VulkanDevice;
use crate::shaders_embedded::{
    FORWARD_FRAG_SPV, FORWARD_VERT_SPV, SKINNED_VERT_SPV, UI_OVERLAY_FRAG_SPV, UI_OVERLAY_VERT_SPV,
};

// ============================================================================
// GpuMesh
// ============================================================================

/// GPU-side representation of a mesh: vertex buffer, index buffer and the
/// metadata needed to issue an indexed draw call.
#[derive(Clone, Debug)]
pub struct GpuMesh {
    pub vertex_buffer: BufferHandle,
    pub index_buffer: BufferHandle,
    pub index_count: u32,
    pub index_format: IndexFormat,
    pub content_hash: [u8; 32],
    pub revision: u64,
}

#[derive(Clone, Copy, Debug)]
struct UploadedResourceState {
    content_hash: [u8; 32],
    revision: u64,
}

#[derive(Clone, Debug)]
struct UploadedMaterialState {
    binding: MaterialBinding,
    content_hash: [u8; 32],
    revision: u64,
}

// ============================================================================
// Fallback mesh data  �? a coloured quad
// ============================================================================

/// Single vertex for the fallback mesh.
///
/// Layout: position (float32x3) + normal (float32x3) + uv (float32x2).
/// Total stride = 32 bytes, matching both the forward and shadow pipelines.
#[repr(C)]
struct FallbackVertex {
    position: [f32; 3],
    normal: [f32; 3],
    uv: [f32; 2],
}

const FALLBACK_VERTICES: [FallbackVertex; 4] = [
    FallbackVertex {
        position: [-0.5, -0.5, 0.0],
        normal: [0.0, 0.0, 1.0],
        uv: [0.0, 1.0],
    },
    FallbackVertex {
        position: [0.5, -0.5, 0.0],
        normal: [0.0, 0.0, 1.0],
        uv: [1.0, 1.0],
    },
    FallbackVertex {
        position: [0.5, 0.5, 0.0],
        normal: [0.0, 0.0, 1.0],
        uv: [1.0, 0.0],
    },
    FallbackVertex {
        position: [-0.5, 0.5, 0.0],
        normal: [0.0, 0.0, 1.0],
        uv: [0.0, 0.0],
    },
];

fn extraction_stats(input: &RenderFrameInput) -> engine_renderer::ExtractionStats {
    input
        .extraction_stats
        .unwrap_or(engine_renderer::ExtractionStats {
            visible_drawables: u32::try_from(
                input
                    .drawables
                    .len()
                    .saturating_add(input.skinned_items.len()),
            )
            .unwrap_or(u32::MAX),
            culled_drawables: 0,
            visible_lights: u32::try_from(input.lights.len()).unwrap_or(u32::MAX),
            culled_lights: 0,
        })
}

fn apply_extraction_stats(stats: &mut FrameStats, input: &RenderFrameInput) {
    let extraction = extraction_stats(input);
    stats.visible_drawables = extraction.visible_drawables;
    stats.culled_drawables = extraction.culled_drawables;
    stats.visible_lights = extraction.visible_lights;
    stats.culled_lights = extraction.culled_lights;
}

fn fallback_vertex_bytes() -> Vec<u8> {
    let mut bytes = Vec::with_capacity(FALLBACK_VERTICES.len() * 32);
    for v in &FALLBACK_VERTICES {
        for f in v
            .position
            .iter()
            .copied()
            .chain(v.normal.iter().copied())
            .chain(v.uv.iter().copied())
        {
            bytes.extend_from_slice(&f.to_ne_bytes());
        }
    }
    bytes
}

const FALLBACK_INDICES: [u16; 6] = [0, 1, 2, 2, 3, 0];

#[inline]
fn vulkan_index_type(format: IndexFormat) -> vk::IndexType {
    match format {
        IndexFormat::U16 => vk::IndexType::UINT16,
        IndexFormat::U32 => vk::IndexType::UINT32,
    }
}

fn fallback_index_bytes() -> Vec<u8> {
    let mut bytes = Vec::with_capacity(FALLBACK_INDICES.len() * 2);
    for i in &FALLBACK_INDICES {
        bytes.extend_from_slice(&i.to_ne_bytes());
    }
    bytes
}

const SCENE_FORWARD_PIPELINE_ID: &str = "scene-forward";

fn scene_forward_vertex_layout() -> VertexLayout {
    VertexLayout {
        // 32-byte stride: position + normal + UV.
        stride_bytes: 32,
        attributes: vec![
            VertexAttribute {
                semantic: "position".into(),
                format: "float32x3".into(),
                offset_bytes: 0,
            },
            VertexAttribute {
                semantic: "normal".into(),
                format: "float32x3".into(),
                offset_bytes: 12,
            },
            VertexAttribute {
                semantic: "uv".into(),
                format: "float32x2".into(),
                offset_bytes: 24,
            },
        ],
    }
}

fn scene_skinned_vertex_layout() -> VertexLayout {
    VertexLayout {
        // 64-byte stride: position(12) + normal(12) + uv(8) + joints(16) + weights(16)
        stride_bytes: 64,
        attributes: vec![
            VertexAttribute {
                semantic: "position".into(),
                format: "float32x3".into(),
                offset_bytes: 0,
            },
            VertexAttribute {
                semantic: "normal".into(),
                format: "float32x3".into(),
                offset_bytes: 12,
            },
            VertexAttribute {
                semantic: "uv".into(),
                format: "float32x2".into(),
                offset_bytes: 24,
            },
            VertexAttribute {
                semantic: "joints".into(),
                format: "uint32x4".into(),
                offset_bytes: 32,
            },
            VertexAttribute {
                semantic: "weights".into(),
                format: "float32x4".into(),
                offset_bytes: 48,
            },
        ],
    }
}

fn scene_skinned_pipeline_context(
    pll: PipelineLayoutHandle,
    rp: RenderPassHandle,
    sample_count: u8,
) -> MaterialPipelineContext {
    MaterialPipelineContext {
        shader_modules: vec![],
        vertex_layout: scene_skinned_vertex_layout(),
        bind_layouts: vec![
            // Material UBO at set=2, binding=0
            BindGroupLayoutDescriptor {
                set_index: 2,
                bindings: vec![BindGroupLayoutBinding {
                    binding: 0,
                    resource_kind: "uniform_buffer".into(),
                }],
            },
        ],
        pipeline_layout: pll,
        render_pass: rp,
        render_targets: vec![TextureFormat::Bgra8Unorm],
        depth_format: Some(TextureFormat::Depth32Float),
        depth_write_enabled: true,
        depth_compare: Some("less".into()),
        front_face: None,
        topology: Some("triangle_list".into()),
        polygon_mode: Some("fill".into()),
        sample_count,
    }
}

fn scene_forward_pipeline_context(
    pll: PipelineLayoutHandle,
    rp: RenderPassHandle,
    sample_count: u8,
) -> MaterialPipelineContext {
    MaterialPipelineContext {
        shader_modules: vec![],
        vertex_layout: scene_forward_vertex_layout(),
        bind_layouts: vec![
            // Material UBO at set=2
            BindGroupLayoutDescriptor {
                set_index: 2,
                bindings: vec![BindGroupLayoutBinding {
                    binding: 0,
                    resource_kind: "uniform_buffer".into(),
                }],
            },
        ],
        pipeline_layout: pll,
        render_pass: rp,
        render_targets: vec![TextureFormat::Bgra8Unorm],
        depth_format: Some(TextureFormat::Depth32Float),
        depth_write_enabled: true,
        depth_compare: Some("less".into()),
        front_face: None,
        topology: Some("triangle_list".into()),
        polygon_mode: Some("fill".into()),
        sample_count,
    }
}

fn fallback_material_binding(material_id: &AssetId) -> MaterialBinding {
    MaterialBinding {
        material_id: material_id.clone(),
        pipeline: AssetId::new(SCENE_FORWARD_PIPELINE_ID),
        variant_key: 0,
        textures: Vec::new(),
        uniforms: ParamBlock {
            bytes: Vec::new(),
            layout_hash: [0; 32],
        },
        pass_mask: 1,
        transparency: Transparency::Opaque,
        double_sided: false,
    }
}

fn uploaded_material_binding(upload: &MaterialUpload) -> MaterialBinding {
    let mut bytes = Vec::with_capacity(32);
    for value in upload.base_color {
        bytes.extend_from_slice(&value.to_ne_bytes());
    }
    for value in [
        upload.metallic,
        upload.roughness,
        upload.ambient_occlusion,
        0.0,
    ] {
        bytes.extend_from_slice(&value.to_ne_bytes());
    }
    let textures = upload
        .base_color_texture
        .as_ref()
        .map(|texture| {
            vec![TextureSlot {
                binding: 1,
                texture: texture.clone(),
                sampler: AssetId::new(format!("{}::sampler", texture.id)),
                color_space: engine_renderer::ColorSpace::Srgb,
                mip_bias: 0.0,
            }]
        })
        .unwrap_or_default();
    MaterialBinding {
        material_id: upload.material_id.clone(),
        pipeline: AssetId::new(SCENE_FORWARD_PIPELINE_ID),
        variant_key: 0,
        textures,
        uniforms: ParamBlock {
            bytes,
            layout_hash: upload.content_hash,
        },
        pass_mask: 1,
        transparency: upload.transparency.clone(),
        double_sided: upload.double_sided,
    }
}

/// CPU-side material UBO layout (32 bytes total).
///
/// Field layout (std140):
/// | offset | field       | type      | bytes |
/// |--------|-------------|-----------|-------|
/// |      0 | base_color  | vec4     |    16 |
/// |     16 | metallic    | float    |     4 |
/// |     20 | roughness   | float    |     4 |
/// |     24 | ao          | float    |     4 |
/// |     28 | _padding    | float    |     4 |
/// Total: 32 bytes.
#[repr(C)]
struct MaterialUBO {
    base_color: [f32; 4],
    metallic: f32,
    roughness: f32,
    ao: f32,
    _padding: [f32; 1],
}

/// Cache entry for a material descriptor set + UBO buffer.
struct MaterialCacheEntry {
    desc_set: vk::DescriptorSet,
    handle: BufferHandle,
    buffer: vk::Buffer,
    ubo_data: [u8; 32],
    bound_texture_id: String,
}

const MAX_MATERIALS: usize = 256;

/// Cache entry for a bone palette descriptor set + buffer.
#[allow(dead_code)]
struct BonePaletteCacheEntry {
    desc_set: vk::DescriptorSet,
    bone_buffer: vk::Buffer,
    bound_texture_id: String,
}

/// Cached bone UBO buffer (handle for writes + raw VkBuffer for descriptor binding).
struct CachedBoneBuffer {
    handle: BufferHandle,
    vk_buffer: vk::Buffer,
    ubo_data: Vec<u8>,
}

const MAX_BONE_PALETTES: usize = 64;

fn get_or_create_scene_forward_pipeline(
    material_resolver: &mut MaterialResolver,
    device: &mut dyn Device,
    material: &MaterialBinding,
    pll: PipelineLayoutHandle,
    rp: RenderPassHandle,
    variant_key: PipelineVariantKey,
    sample_count: u8,
) -> Result<PipelineHandle, render_core::RhiError> {
    let context = scene_forward_pipeline_context(pll, rp, sample_count);
    let (pipeline_key, pipeline_desc) = material_resolver.resolve(material, &context, variant_key);
    material_resolver
        .library_mut()
        .get_or_create(device, pipeline_key, &pipeline_desc)
}

fn get_or_create_scene_skinned_pipeline(
    material_resolver: &mut MaterialResolver,
    device: &mut dyn Device,
    material: &MaterialBinding,
    pll: PipelineLayoutHandle,
    rp: RenderPassHandle,
    sample_count: u8,
) -> Result<PipelineHandle, render_core::RhiError> {
    let context = scene_skinned_pipeline_context(pll, rp, sample_count);
    let (pipeline_key, pipeline_desc) =
        material_resolver.resolve(material, &context, PipelineVariantKey::SKINNED);
    material_resolver
        .library_mut()
        .get_or_create(device, pipeline_key, &pipeline_desc)
}

// ============================================================================
// Light GPU data packing
// ============================================================================

/// Pack a single [`LightItem`] into the 64-byte GPU Light struct format.
///
/// GPU layout (std430):
///   position[4]    �?xyz = world position, w = type flag (0=dir, 1=point, 2=spot)
///   direction[4]   �?xyz = normalized direction, w = unused
///   color[4]       �?rgb = color, a = intensity
///   attenuation[4] �?x = range, y = linear, z = quadratic, w = spot_cutoff_cos
///
/// Total: 64 bytes per light.
fn pack_light_gpu_bytes(light: &LightItem, dir: [f32; 3], kind_w: f32) -> Vec<u8> {
    let mut buf = Vec::with_capacity(64);

    // position (xyz + kind_w)
    for &v in &light.position {
        buf.extend_from_slice(&v.to_ne_bytes());
    }
    buf.extend_from_slice(&kind_w.to_ne_bytes());

    // direction (xyz + 0.0)
    for &v in &dir {
        buf.extend_from_slice(&v.to_ne_bytes());
    }
    buf.extend_from_slice(&0.0f32.to_ne_bytes());

    // color (rgb + intensity)
    for &v in &light.color {
        buf.extend_from_slice(&v.to_ne_bytes());
    }
    buf.extend_from_slice(&light.intensity.to_ne_bytes());

    // attenuation (range, linear, quadratic, spot_cutoff_cos)
    let range = light.range.max(0.0);
    let quadratic = if range > 0.0 {
        1.0 / (range * range)
    } else {
        0.0
    };
    let spot_cutoff = match (&light.kind, &light.spot_angles) {
        (LightKind::Spot, Some(angles)) => angles.outer.cos(),
        _ => 0.0,
    };
    buf.extend_from_slice(&range.to_ne_bytes());
    buf.extend_from_slice(&0.0f32.to_ne_bytes()); // linear factor
    buf.extend_from_slice(&quadratic.to_ne_bytes());
    buf.extend_from_slice(&spot_cutoff.to_ne_bytes());

    buf
}

/// Normalize a 3-component direction vector. Returns `[0, -1, 0]` for zero length.
fn normalize_dir(d: &[f32; 3]) -> [f32; 3] {
    let len_sq = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
    if len_sq > 0.0 {
        let inv = 1.0 / len_sq.sqrt();
        [d[0] * inv, d[1] * inv, d[2] * inv]
    } else {
        [0.0, -1.0, 0.0]
    }
}

// ============================================================================
// CPU-side indirect-draw command (matches VkDrawIndexedIndirectCommand)
// ============================================================================

/// CPU-side representation of a single `vkCmdDrawIndexedIndirect` command.
///
/// Layout matches `VkDrawIndexedIndirectCommand` exactly (20 bytes total):
/// | offset | field          | type | bytes |
/// |--------|----------------|------|-------|
/// |      0 | index_count    | u32  |     4 |
/// |      4 | instance_count | u32  |     4 |
/// |      8 | first_index    | u32  |     4 |
/// |     12 | vertex_offset  | i32  |     4 |
/// |     16 | first_instance | u32  |     4 |
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct IndirectDrawCommand {
    pub index_count: u32,
    pub instance_count: u32,
    pub first_index: u32,
    pub vertex_offset: i32,
    pub first_instance: u32,
}

/// Maximum number of indirect draw commands we can issue per frame.
pub(crate) const MAX_INDIRECT_DRAWS: u32 = 1024;

// ============================================================================
// SceneRenderer
// ============================================================================

/// Vulkan implementation of [`BackendRenderer`].
///
/// Wraps a [`VulkanDevice`] and processes [`RenderFrameInput`] by creating
/// GPU buffers for each referenced mesh on first encounter and then issuing
/// indexed draw calls through a forward-shaded graphics pipeline.
pub struct SceneRenderer {
    device: VulkanDevice,
    initialized: bool,

    /// Cache of loaded meshes indexed by their [`AssetId`](engine_serialize::AssetId) string.
    meshes: BTreeMap<String, GpuMesh>,
    material_resolver: MaterialResolver,
    texture_uploads: HashMap<String, UploadedResourceState>,
    uploaded_materials: HashMap<String, UploadedMaterialState>,

    /// Cache of material descriptor sets + buffers, keyed by material_id.
    /// Limited to [`MAX_MATERIALS`] entries; oldest entries evicted when full.
    material_cache: HashMap<String, MaterialCacheEntry>,
    /// Insertion order for LRU eviction of the material cache.
    material_cache_order: Vec<String>,

    /// Cache of bone palette UBO buffers, keyed by skeleton_id (AssetId string).
    /// Each entry contains the BufferHandle (for data updates) and the raw VkBuffer (for descriptor binding).
    bone_palette_buffers: HashMap<String, CachedBoneBuffer>,
    /// Insertion order for LRU eviction of the bone buffer cache.
    bone_palette_buffers_order: Vec<String>,

    /// Cache of combined skinning descriptor sets, keyed by "material_id:skeleton_id".
    /// Each entry has a descriptor set (material UBO at binding=0 + bone UBO at binding=2)
    /// and the raw VkBuffer for the bone palette.
    skinned_desc_cache: HashMap<String, BonePaletteCacheEntry>,
    /// Insertion order for LRU eviction of the skinned descriptor cache.
    skinned_desc_cache_order: Vec<String>,

    rp: Option<RenderPassHandle>,
    pll: Option<PipelineLayoutHandle>,

    /// UI overlay pipeline (no depth, alpha blend, 2D positions).
    ui_pl: Option<PipelineHandle>,
    ui_pll: Option<PipelineLayoutHandle>,

    /// Per-swapchain-image framebuffer handles (color + depth).
    framebuffers: Vec<FramebufferHandle>,
    /// Index into `framebuffers` for the current swapchain image.
    cur_fb_index: u32,

    // Frame lifecycle state (stored between begin_frame / execute_pass / end_frame).
    cur_sc: Option<SwapchainHandle>,
    cur_ii: Option<u32>,
    cur_enc: Option<Box<dyn CommandEncoder>>,

    /// Window dimensions (logical pixels).
    width: u32,
    height: u32,

    /// Registry of pluggable render passes.
    pub(crate) pass_registry: PassRegistry,

    /// UI overlay vertex/index buffer cache.
    ui_vb: Option<BufferHandle>,
    ui_vb_capacity: u64,
}

impl SceneRenderer {
    /// Create a new scene renderer backed by the given [`VulkanDevice`].
    ///
    /// `width` and `height` represent the initial swapchain extent in
    /// logical pixels.
    pub fn new(device: VulkanDevice, width: u32, height: u32) -> Self {
        Self {
            device,
            initialized: false,
            material_resolver: MaterialResolver::new(16),
            meshes: BTreeMap::new(),
            texture_uploads: HashMap::new(),
            uploaded_materials: HashMap::new(),
            material_cache: HashMap::new(),
            material_cache_order: Vec::new(),
            bone_palette_buffers: HashMap::new(),
            bone_palette_buffers_order: Vec::new(),
            skinned_desc_cache: HashMap::new(),
            skinned_desc_cache_order: Vec::new(),
            rp: None,
            pll: None,
            ui_pl: None,
            ui_pll: None,
            ui_vb: None,
            ui_vb_capacity: 0,
            framebuffers: Vec::new(),
            cur_fb_index: 0,
            cur_sc: None,
            cur_ii: None,
            cur_enc: None,
            width: width.max(1),
            height: height.max(1),
            pass_registry: PassRegistry::new(),
        }
    }

    /// Forward a resize notification to the underlying device.
    ///
    /// The swapchain will be re-created on the next frame.
    pub fn resize(&mut self, w: u32, h: u32) {
        self.width = w.max(1);
        self.height = h.max(1);
        self.device.resize(w, h);
    }

    /// Block until the GPU is idle.
    pub fn wait_idle(&self) {
        self.device.wait_idle();
    }

    // ------------------------------------------------------------------
    // Pipeline initialisation  (lazy �?called on the first frame)
    // ------------------------------------------------------------------

    fn configure_scene_shaders(&mut self) {
        self.device
            .set_mvp_shaders(FORWARD_VERT_SPV, FORWARD_FRAG_SPV);
        if !SKINNED_VERT_SPV.is_empty() {
            self.device.set_skinned_vertex_shader(SKINNED_VERT_SPV);
        }
    }

    /// Create the render pass and pipeline layout used by scene-forward draws.
    ///
    /// This is called once from [`begin_frame_impl`] when
    /// `self.initialized` is `false`.
    fn init_once(&mut self) -> Result<(), Vec<Diagnostic>> {
        if self.initialized {
            return Ok(());
        }

        // Ensure material descriptor infrastructure (set=2) exists before
        // creating the pipeline layout so the fallback picks it up.
        self.device
            .create_material_descriptor_infra()
            .map_err(|e| {
                vec![Diagnostic::new(
                    "RV0213",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("create_material_descriptor_infra: {e:?}"),
                )]
            })?;

        // --- Render pass  (colour + depth) ---
        // NOTE: the scene-forward render pass renders directly to the
        // swapchain (BGRA8, always single-sampled).  MSAA is handled by
        // the HDR offscreen forward pass instead.
        let rp_desc = RenderPassDescriptor {
            color_attachments: vec![TextureFormat::Bgra8Unorm],
            depth_stencil_format: Some(TextureFormat::Depth32Float),
            sample_count: 1,
            debug_label: Some("scene-rp".into()),
        };
        let rp = self.device.create_render_pass(&rp_desc).map_err(|e| {
            vec![Diagnostic::new(
                "RV0200",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("create_render_pass: {e:?}"),
            )]
        })?;

        // --- Pipeline layout  (push constants for MVP) ---
        let pll_desc = PipelineLayoutDescriptor {
            bind_group_layouts: vec![],
            push_constant_ranges: vec![PushConstantRange {
                // VK_SHADER_STAGE_VERTEX_BIT = 0x01
                stage_flags: 0x01,
                offset: 0,
                size: 128, // 4�? f32 matrix (64 B) + spare uniform data
            }],
            debug_label: Some("scene-pll".into()),
        };
        let pll = self.device.create_pipeline_layout(&pll_desc).map_err(|e| {
            vec![Diagnostic::new(
                "RV0201",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("create_pipeline_layout: {e:?}"),
            )]
        })?;

        // ── Material descriptor infrastructure (set=2: UBO + texture) ─
        self.device
            .create_material_descriptor_infra()
            .map_err(|e| {
                vec![Diagnostic::new(
                    "RV0210",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("create_material_descriptor_infra: {e:?}"),
                )]
            })?;

        // ── Shadow-mapping resources ──────────────────────────────────
        // Ensure the device has created shadow resources (idempotent).
        self.device.ensure_shadow().map_err(|e| {
            vec![Diagnostic::new(
                "RV0211",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("ensure_shadow: {e:?}"),
            )]
        })?;

        // ── Environment cubemap (IBL, set=1 binding=1) ────────────────
        self.device.create_env_cubemap().map_err(|e| {
            vec![Diagnostic::new(
                "RV0212",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("create_env_cubemap: {e:?}"),
            )]
        })?;

        // ── Light SSBO (set=1 binding=2) ───────────────────────────
        self.device.create_light_ssbo().map_err(|e| {
            vec![Diagnostic::new(
                "RV0222",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("create_light_ssbo: {e:?}"),
            )]
        })?;

        // ── Indirect draw buffers (Phase 5.1) ─────────────────────
        self.device
            .create_indirect_buffers(MAX_INDIRECT_DRAWS)
            .map_err(|e| {
                vec![Diagnostic::new(
                    "RV0223",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("create_indirect_buffers: {e:?}"),
                )]
            })?;

        self.rp = Some(rp);
        self.pll = Some(pll);

        // ── Framebuffers (per swapchain image, color + depth) ─────────
        let vk_rp = self
            .device
            .render_passes
            .get(rp.index, rp.generation)
            .copied();
        if let Some(vk_rp) = vk_rp {
            let fbs = self.device.create_scene_framebuffers(vk_rp).map_err(|e| {
                vec![Diagnostic::new(
                    "RV0213",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("create_scene_framebuffers: {e:?}"),
                )]
            })?;
            self.framebuffers = fbs;
        }

        // ── UI overlay pipeline ────────────────────────────────────
        let ui_vert_spv = UI_OVERLAY_VERT_SPV;
        let ui_frag_spv = UI_OVERLAY_FRAG_SPV;
        if !ui_vert_spv.is_empty() && !ui_frag_spv.is_empty() {
            // Temporarily set UI shaders to create the pipeline
            let old_vert = self.device.mvp_vert_spv.clone();
            let old_frag = self.device.mvp_frag_spv.clone();
            self.device.set_mvp_shaders(ui_vert_spv, ui_frag_spv);

            // Create UI pipeline layout: push constants for screen_size (vec2 = 8 bytes)
            let ui_pll_desc = PipelineLayoutDescriptor {
                bind_group_layouts: vec![],
                push_constant_ranges: vec![PushConstantRange {
                    stage_flags: 0x01, // VERTEX
                    offset: 0,
                    size: 8, // vec2 screen_size
                }],
                debug_label: Some("ui-overlay-pll".into()),
            };
            let ui_pll = self
                .device
                .create_pipeline_layout(&ui_pll_desc)
                .map_err(|e| {
                    vec![Diagnostic::new(
                        "RV0214",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        format!("create_ui_pipeline_layout: {e:?}"),
                    )]
                })?;

            // Create UI pipeline: no depth, alpha blending, 2D vertex format
            let ui_desc = render_core::PipelineDescriptor {
                shader_modules: vec![],
                vertex_layout: VertexLayout {
                    stride_bytes: 32,
                    attributes: vec![
                        VertexAttribute {
                            semantic: "position".into(),
                            format: "float32x2".into(),
                            offset_bytes: 0,
                        },
                        VertexAttribute {
                            semantic: "uv".into(),
                            format: "float32x2".into(),
                            offset_bytes: 8,
                        },
                        VertexAttribute {
                            semantic: "color".into(),
                            format: "float32x4".into(),
                            offset_bytes: 16,
                        },
                    ],
                },
                bind_layouts: vec![],
                pipeline_layout: Some(ui_pll),
                raster_state: render_core::RasterState {
                    cull_mode: Some("none".into()),
                    front_face: None,
                },
                depth_state: render_core::DepthState {
                    format: None,
                    write_enabled: false,
                    compare: None,
                },
                blend_state: render_core::BlendState {
                    mode: Some("Alpha".into()),
                },
                render_targets: vec![TextureFormat::Bgra8Unorm],
                debug_label: Some("ui-overlay-pl".into()),
                topology: Some("triangle_list".into()),
                polygon_mode: Some("fill".into()),
                sample_count: Some(1),
                render_pass: Some(rp),
                specialization: Vec::new(),
            };
            let ui_pl = self.device.create_pipeline(&ui_desc).map_err(|e| {
                vec![Diagnostic::new(
                    "RV0215",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("create_ui_pipeline: {e:?}"),
                )]
            })?;

            self.ui_pll = Some(ui_pll);
            self.ui_pl = Some(ui_pl);

            // Restore forward shaders
            if let (Some(v), Some(f)) = (old_vert, old_frag) {
                self.device.set_mvp_shaders(&v, &f);
            }
        }

        self.initialized = true;
        Ok(())
    }

    fn ensure_scene_framebuffers(&mut self) -> Result<(), Vec<Diagnostic>> {
        if !self.framebuffers.is_empty() {
            return Ok(());
        }
        let render_pass = self.rp.ok_or_else(|| {
            vec![Diagnostic::new(
                "RV0232",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "scene render pass is unavailable while rebuilding framebuffers",
            )]
        })?;
        let vk_render_pass = self
            .device
            .render_passes
            .get(render_pass.index, render_pass.generation)
            .copied()
            .ok_or_else(|| {
                vec![Diagnostic::new(
                    "RV0233",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    "scene render-pass handle is stale while rebuilding framebuffers",
                )]
            })?;
        self.framebuffers = self
            .device
            .create_scene_framebuffers(vk_render_pass)
            .map_err(|error| {
                vec![Diagnostic::new(
                    "RV0234",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("create_scene_framebuffers: {error:?}"),
                )]
            })?;
        Ok(())
    }

    fn material_binding_for_drawable(
        &self,
        input: &RenderFrameInput,
        material_id: &AssetId,
    ) -> MaterialBinding {
        input
            .materials
            .iter()
            .find(|material| material.material_id == *material_id)
            .cloned()
            .or_else(|| {
                self.uploaded_materials
                    .get(&material_id.id)
                    .map(|state| state.binding.clone())
            })
            .unwrap_or_else(|| fallback_material_binding(material_id))
    }

    fn prepare_frame_cache_capacity(
        &mut self,
        input: &RenderFrameInput,
    ) -> Result<(), Vec<Diagnostic>> {
        let required_materials: BTreeSet<String> = input
            .drawables
            .iter()
            .map(|item| item.material.id.clone())
            .chain(
                input
                    .skinned_items
                    .iter()
                    .map(|item| item.material.id.clone()),
            )
            .collect();
        if required_materials.len() > MAX_MATERIALS {
            return Err(vec![Diagnostic::new(
                "RV0271",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!(
                    "frame needs {} materials, backend capacity is {MAX_MATERIALS}",
                    required_materials.len()
                ),
            )]);
        }
        let missing_materials = required_materials
            .iter()
            .filter(|id| !self.material_cache.contains_key(*id))
            .count();
        while self.material_cache.len() + missing_materials > MAX_MATERIALS {
            let candidate = self
                .material_cache_order
                .iter()
                .find(|id| !required_materials.contains(*id))
                .cloned()
                .ok_or_else(|| {
                    vec![Diagnostic::new(
                        "RV0272",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        "material cache cannot reserve the current frame's working set",
                    )]
                })?;
            self.evict_material_by_id(&candidate)?;
        }

        let required_skeletons: BTreeSet<String> = input
            .skinned_items
            .iter()
            .map(|item| item.skeleton.id.clone())
            .collect();
        let required_skinned_sets: BTreeSet<String> = input
            .skinned_items
            .iter()
            .map(|item| format!("{}:{}", item.material.id, item.skeleton.id))
            .collect();
        if required_skeletons.len() > MAX_BONE_PALETTES
            || required_skinned_sets.len() > MAX_BONE_PALETTES
        {
            return Err(vec![Diagnostic::new(
                "RV0273",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!(
                    "frame exceeds skinned capacity: {} skeletons, {} material/skeleton pairs, limit {MAX_BONE_PALETTES}",
                    required_skeletons.len(),
                    required_skinned_sets.len()
                ),
            )]);
        }
        let missing_skeletons = required_skeletons
            .iter()
            .filter(|id| !self.bone_palette_buffers.contains_key(*id))
            .count();
        while self.bone_palette_buffers.len() + missing_skeletons > MAX_BONE_PALETTES {
            let candidate = self
                .bone_palette_buffers_order
                .iter()
                .find(|id| !required_skeletons.contains(*id))
                .cloned()
                .ok_or_else(|| {
                    vec![Diagnostic::new(
                        "RV0274",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        "bone cache cannot reserve the current frame's working set",
                    )]
                })?;
            self.evict_skeleton_by_id(&candidate)?;
        }
        let missing_skinned_sets = required_skinned_sets
            .iter()
            .filter(|key| !self.skinned_desc_cache.contains_key(*key))
            .count();
        while self.skinned_desc_cache.len() + missing_skinned_sets > MAX_BONE_PALETTES {
            let candidate = self
                .skinned_desc_cache_order
                .iter()
                .find(|key| !required_skinned_sets.contains(*key))
                .cloned()
                .ok_or_else(|| {
                    vec![Diagnostic::new(
                        "RV0275",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        "skinned descriptor cache cannot reserve the current frame's working set",
                    )]
                })?;
            self.evict_skinned_descriptor_by_key(&candidate)?;
        }
        Ok(())
    }

    fn validate_uploaded_meshes(&self, input: &RenderFrameInput) -> Result<(), Vec<Diagnostic>> {
        for mesh_id in input
            .drawables
            .iter()
            .map(|item| &item.mesh.id)
            .chain(input.skinned_items.iter().map(|item| &item.mesh.id))
        {
            let Some(mesh) = self.meshes.get(mesh_id) else {
                return Err(vec![Diagnostic::new(
                    "RV0230",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("drawable references mesh '{mesh_id}' before a successful upload"),
                )]);
            };
            let vertex_is_live = self
                .device
                .buffers
                .get(mesh.vertex_buffer.index, mesh.vertex_buffer.generation)
                .is_some();
            let index_is_live = self
                .device
                .buffers
                .get(mesh.index_buffer.index, mesh.index_buffer.generation)
                .is_some();
            if !vertex_is_live || !index_is_live {
                return Err(vec![Diagnostic::new(
                    "RV0231",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("mesh '{mesh_id}' refers to released GPU buffers"),
                )]);
            }
        }
        for material_id in input
            .drawables
            .iter()
            .map(|item| &item.material)
            .chain(input.skinned_items.iter().map(|item| &item.material))
        {
            let material = self.material_binding_for_drawable(input, material_id);
            self.selected_material_texture_id(&material)?;
        }
        Ok(())
    }

    fn pipeline_for_drawable(
        &mut self,
        input: &RenderFrameInput,
        drawable: &RenderableItem,
        sample_count: u8,
    ) -> Result<PipelineHandle, Vec<Diagnostic>> {
        let pll = self.pll.ok_or_else(|| {
            vec![Diagnostic::new(
                "RV0202",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "pipeline layout missing during drawable pipeline resolution",
            )]
        })?;
        let rp = self.rp.ok_or_else(|| {
            vec![Diagnostic::new(
                "RV0203",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "render pass missing during drawable pipeline resolution",
            )]
        })?;
        let material = self.material_binding_for_drawable(input, &drawable.material);

        get_or_create_scene_forward_pipeline(
            &mut self.material_resolver,
            &mut self.device,
            &material,
            pll,
            rp,
            PipelineVariantKey::NONE,
            sample_count,
        )
        .map_err(|e| {
            vec![Diagnostic::new(
                "RV0204",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("resolve pipeline: {e:?}"),
            )]
        })
    }

    fn pipeline_for_skinned_drawable(
        &mut self,
        input: &RenderFrameInput,
        skinned: &SkinnedItem,
        sample_count: u8,
    ) -> Result<PipelineHandle, Vec<Diagnostic>> {
        let pll = self.pll.ok_or_else(|| {
            vec![Diagnostic::new(
                "RV0202",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "pipeline layout missing during skinned drawable pipeline resolution",
            )]
        })?;
        let rp = self.rp.ok_or_else(|| {
            vec![Diagnostic::new(
                "RV0203",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "render pass missing during skinned drawable pipeline resolution",
            )]
        })?;
        let material = self.material_binding_for_drawable(input, &skinned.material);

        get_or_create_scene_skinned_pipeline(
            &mut self.material_resolver,
            &mut self.device,
            &material,
            pll,
            rp,
            sample_count,
        )
        .map_err(|e| {
            vec![Diagnostic::new(
                "RV0204",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("resolve skinned pipeline: {e:?}"),
            )]
        })
    }

    /// Look up or create a bone-palette UBO buffer for the given skeleton.
    /// The buffer is sized for up to 64 Mat4 entries (4096 bytes).
    /// The buffer contents are updated with the latest bone palette data each call.
    fn get_or_create_bone_buffer(
        &mut self,
        skeleton_id: &str,
        bone_palette: &[[f32; 16]],
    ) -> Result<vk::Buffer, Vec<Diagnostic>> {
        // Build UBO data: up to 64 Mat4 entries (64 bytes each = 4096 bytes)
        let mut ubo_data = Vec::with_capacity(4096);
        for mat in bone_palette {
            for v in mat {
                ubo_data.extend_from_slice(&v.to_ne_bytes());
            }
        }
        ubo_data.resize(4096, 0u8);

        // Check bone buffer cache �?if found, update data and return.
        if let Some(cached) = self.bone_palette_buffers.get(skeleton_id) {
            let handle = cached.handle;
            let vk_buffer = cached.vk_buffer;
            let needs_update = cached.ubo_data != ubo_data;
            // Promote in LRU order
            if let Some(pos) = self
                .bone_palette_buffers_order
                .iter()
                .position(|k| k == skeleton_id)
            {
                self.bone_palette_buffers_order.remove(pos);
                self.bone_palette_buffers_order
                    .push(skeleton_id.to_string());
            }
            if needs_update {
                self.device.wait_idle_checked().map_err(|error| {
                    vec![Diagnostic::new(
                        "RV0254",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        format!("cannot update in-flight skeleton '{skeleton_id}': {error:?}"),
                    )]
                })?;
                self.device
                    .write_buffer(handle, &ubo_data, 0)
                    .map_err(|error| {
                        vec![Diagnostic::new(
                            "RV0255",
                            DiagnosticSeverity::Error,
                            "scene_renderer",
                            format!("write_buffer(bone UBO): {error:?}"),
                        )]
                    })?;
                if let Some(cached) = self.bone_palette_buffers.get_mut(skeleton_id) {
                    cached.ubo_data.clone_from(&ubo_data);
                }
            }
            return Ok(vk_buffer);
        }

        // Create the buffer
        let buf_desc = BufferDescriptor {
            size_bytes: 4096,
            usage_flags: render_core::BufferUsage(0),
            memory_hint: MemoryHint::CpuToGpu,
            debug_label: Some(format!("bone-{skeleton_id}")),
        };
        let buf = self.device.create_buffer(&buf_desc).map_err(|e| {
            vec![Diagnostic::new(
                "RV0218",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("create_buffer(bone UBO): {e:?}"),
            )]
        })?;
        if let Err(error) = self.device.write_buffer(buf, &ubo_data, 0) {
            self.device.destroy_buffer(buf);
            return Err(vec![Diagnostic::new(
                "RV0219",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("write_buffer(bone UBO): {error:?}"),
            )]);
        }

        // Resolve raw Vulkan buffer handle
        let vk_buf = self
            .device
            .buffers
            .get(buf.index, buf.generation)
            .map(|e| e.buffer)
            .unwrap_or(vk::Buffer::null());
        if vk_buf == vk::Buffer::null() {
            self.device.destroy_buffer(buf);
            return Err(vec![Diagnostic::new(
                "RV0220",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "bone UBO buffer handle invalid",
            )]);
        }

        if self.bone_palette_buffers.len() >= MAX_BONE_PALETTES {
            self.device.destroy_buffer(buf);
            return Err(vec![Diagnostic::new(
                "RV0279",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "bone cache capacity was not reserved before frame recording",
            )]);
        }

        self.bone_palette_buffers.insert(
            skeleton_id.to_string(),
            CachedBoneBuffer {
                handle: buf,
                vk_buffer: vk_buf,
                ubo_data,
            },
        );
        self.bone_palette_buffers_order
            .push(skeleton_id.to_string());
        Ok(vk_buf)
    }

    /// Get or create a combined material + bone descriptor set for a skinned drawable.
    /// The descriptor set has:
    ///   binding=0: material UBO
    ///   binding=1: texture (updated later via bind_material_texture)
    ///   binding=2: bone palette UBO
    fn get_or_create_skinned_desc_set(
        &mut self,
        material_id: &str,
        skeleton_id: &str,
        _mat_desc_set: vk::DescriptorSet,
        mat_buffer: vk::Buffer,
        bone_buffer: vk::Buffer,
    ) -> Result<vk::DescriptorSet, Vec<Diagnostic>> {
        let cache_key = format!("{material_id}:{skeleton_id}");

        // Check cache
        if let Some(entry) = self.skinned_desc_cache.get(&cache_key) {
            // Promote in LRU order
            if let Some(pos) = self
                .skinned_desc_cache_order
                .iter()
                .position(|k| k == &cache_key)
            {
                self.skinned_desc_cache_order.remove(pos);
                self.skinned_desc_cache_order.push(cache_key.clone());
            }
            return Ok(entry.desc_set);
        }

        if self.skinned_desc_cache.len() >= MAX_BONE_PALETTES {
            return Err(vec![Diagnostic::new(
                "RV0280",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "skinned descriptor capacity was not reserved before frame recording",
            )]);
        }

        // Allocate a new skinned descriptor set from the material pool
        let desc_set = self
            .device
            .allocate_skinned_material_descriptor_set(mat_buffer, 32, bone_buffer, 4096)
            .map_err(|e| {
                vec![Diagnostic::new(
                    "RV0221",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("allocate_skinned_material_descriptor_set: {e:?}"),
                )]
            })?;

        // Insert into cache
        self.skinned_desc_cache.insert(
            cache_key.clone(),
            BonePaletteCacheEntry {
                desc_set,
                bone_buffer,
                bound_texture_id: crate::device_impl::texture::FALLBACK_MATERIAL_TEXTURE_ID
                    .to_string(),
            },
        );
        self.skinned_desc_cache_order.push(cache_key);

        Ok(desc_set)
    }

    // ------------------------------------------------------------------
    // Material UBO helpers
    // ------------------------------------------------------------------

    /// Parse `ParamBlock` bytes into a [`MaterialUBO`].
    ///
    /// Expected byte layout (matching the shader's MaterialUBO):
    ///   [0..16)  base_color  �?vec4 f32
    ///   [16..20) metallic    �?f32
    ///   [20..24) roughness   �?f32
    ///   [24..28) ao          �?f32
    ///
    /// If `bytes` is empty or too short, sane defaults are used.
    fn parse_material_ubo(bytes: &[u8]) -> MaterialUBO {
        let read_f32 = |offset: usize, fallback: f32| -> f32 {
            if offset + 4 <= bytes.len() {
                let value = f32::from_ne_bytes(bytes[offset..offset + 4].try_into().unwrap());
                if value.is_finite() {
                    value
                } else {
                    fallback
                }
            } else {
                fallback
            }
        };
        let read_vec4 = |offset: usize| -> [f32; 4] {
            if offset + 16 <= bytes.len() {
                [
                    f32::from_ne_bytes(bytes[offset..offset + 4].try_into().unwrap()),
                    f32::from_ne_bytes(bytes[offset + 4..offset + 8].try_into().unwrap()),
                    f32::from_ne_bytes(bytes[offset + 8..offset + 12].try_into().unwrap()),
                    f32::from_ne_bytes(bytes[offset + 12..offset + 16].try_into().unwrap()),
                ]
            } else {
                [0.8, 0.6, 0.4, 1.0]
            }
        };
        MaterialUBO {
            base_color: read_vec4(0),
            metallic: read_f32(16, 0.0).clamp(0.0, 1.0),
            roughness: read_f32(20, 1.0).clamp(0.04, 1.0),
            ao: read_f32(24, 1.0).clamp(0.0, 1.0),
            _padding: [0.0],
        }
    }

    fn selected_material_texture_id(
        &self,
        material: &MaterialBinding,
    ) -> Result<String, Vec<Diagnostic>> {
        use crate::device_impl::texture::FALLBACK_MATERIAL_TEXTURE_ID;

        let Some(slot) = material
            .textures
            .iter()
            .find(|slot| slot.binding == 1)
            .or_else(|| material.textures.first())
        else {
            return Ok(FALLBACK_MATERIAL_TEXTURE_ID.to_string());
        };
        if !self.device.textures.contains_key(&slot.texture.id) {
            return Err(vec![Diagnostic::new(
                "RV0260",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!(
                    "material '{}' references texture '{}' before a successful upload",
                    material.material_id.id, slot.texture.id
                ),
            )]);
        }
        Ok(slot.texture.id.clone())
    }

    fn bind_material_texture_if_changed(
        &mut self,
        material_id: &str,
        material: &MaterialBinding,
        descriptor_set: vk::DescriptorSet,
    ) -> Result<(), Vec<Diagnostic>> {
        let selected = self.selected_material_texture_id(material)?;
        let current = self
            .material_cache
            .get(material_id)
            .map(|entry| entry.bound_texture_id.clone())
            .unwrap_or_default();
        if current == selected {
            return Ok(());
        }
        self.device.wait_idle_checked().map_err(|error| {
            vec![Diagnostic::new(
                "RV0261",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("cannot update in-flight material texture: {error:?}"),
            )]
        })?;
        let bound = self
            .device
            .bind_material_texture(&selected, descriptor_set)
            .map_err(|error| {
                vec![Diagnostic::new(
                    "RV0262",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("bind material texture '{selected}': {error:?}"),
                )]
            })?;
        if !bound {
            return Err(vec![Diagnostic::new(
                "RV0263",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("material texture '{selected}' disappeared before descriptor update"),
            )]);
        }
        if let Some(entry) = self.material_cache.get_mut(material_id) {
            entry.bound_texture_id = selected;
        }
        Ok(())
    }

    fn bind_skinned_texture_if_changed(
        &mut self,
        cache_key: &str,
        material: &MaterialBinding,
        descriptor_set: vk::DescriptorSet,
    ) -> Result<(), Vec<Diagnostic>> {
        let selected = self.selected_material_texture_id(material)?;
        let current = self
            .skinned_desc_cache
            .get(cache_key)
            .map(|entry| entry.bound_texture_id.clone())
            .unwrap_or_default();
        if current == selected {
            return Ok(());
        }
        self.device.wait_idle_checked().map_err(|error| {
            vec![Diagnostic::new(
                "RV0264",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("cannot update in-flight skinned texture: {error:?}"),
            )]
        })?;
        let bound = self
            .device
            .bind_material_texture(&selected, descriptor_set)
            .map_err(|error| {
                vec![Diagnostic::new(
                    "RV0265",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("bind skinned texture '{selected}': {error:?}"),
                )]
            })?;
        if !bound {
            return Err(vec![Diagnostic::new(
                "RV0266",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("skinned texture '{selected}' disappeared before descriptor update"),
            )]);
        }
        if let Some(entry) = self.skinned_desc_cache.get_mut(cache_key) {
            entry.bound_texture_id = selected;
        }
        Ok(())
    }

    /// Look up or create a material descriptor set + buffer for the given
    /// material.  Uses a LRU eviction policy capped at [`MAX_MATERIALS`].
    fn get_or_create_material_desc_set(
        &mut self,
        material_id: &str,
        ubo_data: &[u8],
    ) -> Result<(vk::DescriptorSet, vk::Buffer), Vec<Diagnostic>> {
        let ubo_array: [u8; 32] = ubo_data.try_into().map_err(|_| {
            vec![Diagnostic::new(
                "RV0250",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "material UBO must be exactly 32 bytes",
            )]
        })?;
        // Check cache first (and move to front for LRU)
        if let Some(entry) = self.material_cache.get(material_id) {
            let desc_set = entry.desc_set;
            let buffer = entry.buffer;
            let handle = entry.handle;
            let old_data = entry.ubo_data;
            // Promote in LRU order (simple move-to-front)
            if let Some(pos) = self
                .material_cache_order
                .iter()
                .position(|k| k == material_id)
            {
                self.material_cache_order.remove(pos);
                self.material_cache_order.push(material_id.to_string());
            }
            if old_data.as_slice() != ubo_data {
                self.device.wait_idle_checked().map_err(|error| {
                    vec![Diagnostic::new(
                        "RV0248",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        format!("cannot update in-flight material '{material_id}': {error:?}"),
                    )]
                })?;
                self.device
                    .write_buffer(handle, ubo_data, 0)
                    .map_err(|error| {
                        vec![Diagnostic::new(
                            "RV0249",
                            DiagnosticSeverity::Error,
                            "scene_renderer",
                            format!("write_buffer(material UBO): {error:?}"),
                        )]
                    })?;
                if let Some(entry) = self.material_cache.get_mut(material_id) {
                    entry.ubo_data.copy_from_slice(ubo_data);
                }
            }
            return Ok((desc_set, buffer));
        }

        if self.material_cache.len() >= MAX_MATERIALS {
            return Err(vec![Diagnostic::new(
                "RV0281",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "material cache capacity was not reserved before frame recording",
            )]);
        }

        // Create a small UBO buffer (32 bytes for MaterialUBO)
        let buf_desc = BufferDescriptor {
            size_bytes: 32,
            usage_flags: render_core::BufferUsage(0),
            memory_hint: MemoryHint::CpuToGpu,
            debug_label: Some(format!("mat-ubo-{material_id}")),
        };
        let buf = self.device.create_buffer(&buf_desc).map_err(|e| {
            vec![Diagnostic::new(
                "RV0214",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("create_buffer(material UBO): {e:?}"),
            )]
        })?;
        if let Err(error) = self.device.write_buffer(buf, ubo_data, 0) {
            self.device.destroy_buffer(buf);
            return Err(vec![Diagnostic::new(
                "RV0215",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("write_buffer(material UBO): {error:?}"),
            )]);
        }

        // Resolve raw Vulkan buffer handle for the descriptor set
        let vk_buf = self
            .device
            .buffers
            .get(buf.index, buf.generation)
            .map(|e| e.buffer)
            .unwrap_or(vk::Buffer::null());
        if vk_buf == vk::Buffer::null() {
            self.device.destroy_buffer(buf);
            return Err(vec![Diagnostic::new(
                "RV0216",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "material UBO buffer handle invalid",
            )]);
        }

        // Allocate and update descriptor set via the device
        let desc_set = match self.device.allocate_material_descriptor_set(vk_buf, 32) {
            Ok(desc_set) => desc_set,
            Err(error) => {
                self.device.destroy_buffer(buf);
                return Err(vec![Diagnostic::new(
                    "RV0217",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("allocate_material_descriptor_set: {error:?}"),
                )]);
            }
        };

        let entry = MaterialCacheEntry {
            desc_set,
            handle: buf,
            buffer: vk_buf,
            ubo_data: ubo_array,
            bound_texture_id: crate::device_impl::texture::FALLBACK_MATERIAL_TEXTURE_ID.to_string(),
        };
        self.material_cache.insert(material_id.to_string(), entry);
        self.material_cache_order.push(material_id.to_string());

        Ok((desc_set, vk_buf))
    }

    fn evict_material_by_id(&mut self, material_id: &str) -> Result<(), Vec<Diagnostic>> {
        if !self.material_cache.contains_key(material_id) {
            return Ok(());
        }
        self.device.wait_idle_checked().map_err(|error| {
            vec![Diagnostic::new(
                "RV0251",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("cannot evict in-flight material '{material_id}': {error:?}"),
            )]
        })?;

        let skinned_prefix = format!("{material_id}:");
        let skinned_keys: Vec<String> = self
            .skinned_desc_cache
            .keys()
            .filter(|key| key.starts_with(&skinned_prefix))
            .cloned()
            .collect();
        for key in skinned_keys {
            if let Some(entry) = self.skinned_desc_cache.remove(&key) {
                self.device
                    .free_material_descriptor_set(entry.desc_set)
                    .map_err(|error| {
                        vec![Diagnostic::new(
                            "RV0252",
                            DiagnosticSeverity::Error,
                            "scene_renderer",
                            format!("free skinned descriptor set: {error:?}"),
                        )]
                    })?;
            }
        }
        self.skinned_desc_cache_order
            .retain(|key| !key.starts_with(&skinned_prefix));

        self.material_cache_order.retain(|key| key != material_id);
        if let Some(entry) = self.material_cache.remove(material_id) {
            self.device
                .free_material_descriptor_set(entry.desc_set)
                .map_err(|error| {
                    vec![Diagnostic::new(
                        "RV0253",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        format!("free material descriptor set: {error:?}"),
                    )]
                })?;
            self.device.destroy_buffer(entry.handle);
        }
        Ok(())
    }

    fn evict_skinned_descriptor_by_key(&mut self, cache_key: &str) -> Result<(), Vec<Diagnostic>> {
        if !self.skinned_desc_cache.contains_key(cache_key) {
            return Ok(());
        }
        self.device.wait_idle_checked().map_err(|error| {
            vec![Diagnostic::new(
                "RV0276",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("cannot evict in-flight skinned descriptor: {error:?}"),
            )]
        })?;
        self.skinned_desc_cache_order.retain(|key| key != cache_key);
        if let Some(entry) = self.skinned_desc_cache.remove(cache_key) {
            self.device
                .free_material_descriptor_set(entry.desc_set)
                .map_err(|error| {
                    vec![Diagnostic::new(
                        "RV0277",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        format!("free skinned descriptor set: {error:?}"),
                    )]
                })?;
        }
        Ok(())
    }

    fn evict_skeleton_by_id(&mut self, skeleton_id: &str) -> Result<(), Vec<Diagnostic>> {
        if !self.bone_palette_buffers.contains_key(skeleton_id) {
            return Ok(());
        }
        self.device.wait_idle_checked().map_err(|error| {
            vec![Diagnostic::new(
                "RV0278",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("cannot evict in-flight skeleton '{skeleton_id}': {error:?}"),
            )]
        })?;
        let suffix = format!(":{skeleton_id}");
        let descriptor_keys: Vec<String> = self
            .skinned_desc_cache
            .keys()
            .filter(|key| key.ends_with(&suffix))
            .cloned()
            .collect();
        for key in descriptor_keys {
            self.evict_skinned_descriptor_by_key(&key)?;
        }
        self.bone_palette_buffers_order
            .retain(|key| key != skeleton_id);
        if let Some(entry) = self.bone_palette_buffers.remove(skeleton_id) {
            self.device.destroy_buffer(entry.handle);
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // Mesh caching
    // ------------------------------------------------------------------

    /// Return a cached [`GpuMesh`] for `mesh_id`, or create a fallback quad
    /// mesh and cache it.
    fn get_or_create_mesh(&mut self, mesh_id: &str) -> Result<GpuMesh, Vec<Diagnostic>> {
        if let Some(m) = self.meshes.get(mesh_id) {
            return Ok(m.clone());
        }

        // First encounter �?upload a fallback coloured quad.
        let vertex_bytes = fallback_vertex_bytes();
        let index_bytes = fallback_index_bytes();

        // --- Vertex buffer ---
        let vb_desc = BufferDescriptor {
            size_bytes: vertex_bytes.len() as u64,
            usage_flags: render_core::BufferUsage(0),
            memory_hint: MemoryHint::CpuToGpu,
            debug_label: Some(format!("mesh-{mesh_id}-vertices")),
        };
        let vb = self.device.create_buffer(&vb_desc).map_err(|e| {
            vec![Diagnostic::new(
                "RV0203",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("create_buffer(vertices): {e:?}"),
            )]
        })?;
        if let Err(error) = self.device.write_buffer(vb, &vertex_bytes, 0) {
            self.device.destroy_buffer(vb);
            return Err(vec![Diagnostic::new(
                "RV0204",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("write_buffer(vertices): {error:?}"),
            )]);
        }

        // --- Index buffer ---
        let ib_desc = BufferDescriptor {
            size_bytes: index_bytes.len() as u64,
            usage_flags: render_core::BufferUsage(0),
            memory_hint: MemoryHint::CpuToGpu,
            debug_label: Some(format!("mesh-{mesh_id}-indices")),
        };
        let ib = match self.device.create_buffer(&ib_desc) {
            Ok(buffer) => buffer,
            Err(error) => {
                self.device.destroy_buffer(vb);
                return Err(vec![Diagnostic::new(
                    "RV0205",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("create_buffer(indices): {error:?}"),
                )]);
            }
        };
        if let Err(error) = self.device.write_buffer(ib, &index_bytes, 0) {
            self.device.destroy_buffer(ib);
            self.device.destroy_buffer(vb);
            return Err(vec![Diagnostic::new(
                "RV0206",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("write_buffer(indices): {error:?}"),
            )]);
        }

        let index_count = (index_bytes.len() / 2) as u32; // u16 indices
        let mesh = GpuMesh {
            vertex_buffer: vb,
            index_buffer: ib,
            index_count,
            index_format: IndexFormat::U16,
            content_hash: [0; 32],
            revision: 1,
        };

        self.meshes.insert(mesh_id.to_string(), mesh.clone());
        Ok(mesh)
    }

    // ------------------------------------------------------------------
    // Frame lifecycle helpers
    // ------------------------------------------------------------------

    /// Common initialisation + swapchain creation + device begin-frame.
    ///
    /// Called by both [`render_frame`] and [`begin_frame`].
    ///
    /// `msaa_samples` is the MSAA sample count from `RenderOptions`, capped
    /// to the device's maximum. It is set on the device before swapchain/HDR
    /// resource creation.
    fn begin_frame_impl(
        &mut self,
        input: &RenderFrameInput,
        msaa_samples: vk::SampleCountFlags,
    ) -> Result<(SwapchainHandle, u32, Box<dyn CommandEncoder>), Vec<Diagnostic>> {
        let (view, projection) = input
            .views
            .first()
            .map(|view| {
                (
                    Mat4::from_cols_array(&view.view_matrix),
                    Mat4::from_cols_array(&view.projection_matrix),
                )
            })
            .unwrap_or((Mat4::IDENTITY, Mat4::IDENTITY));
        let matrices_are_finite = view
            .to_cols_array()
            .into_iter()
            .chain(projection.to_cols_array())
            .all(f32::is_finite);
        if !matrices_are_finite || view.determinant().abs() <= f32::EPSILON {
            return Err(vec![Diagnostic::new(
                "RV0210",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "view and projection matrices must be finite and the view matrix invertible",
            )]);
        }
        self.validate_uploaded_meshes(input)?;
        self.prepare_frame_cache_capacity(input)?;

        // Apply the requested MSAA sample count to the device before any
        // resource creation takes place (ensure_sc �?ensure_hdr_resources).
        self.device.hdr_msaa_samples = msaa_samples;
        if !self.initialized {
            // Swapchain setup creates the HDR forward pipeline, so the scene
            // shaders must be registered before `create_swapchain`.
            self.configure_scene_shaders();
        }

        let sc_desc = SwapchainDescriptor {
            surface: render_core::SurfaceHandle::new(0, 1),
            width: self.width,
            height: self.height,
            vsync: false,
            debug_label: None,
        };
        let sc_h = self.device.create_swapchain(&sc_desc).map_err(|e| {
            vec![Diagnostic::new(
                "RV0207",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("create_swapchain: {e:?}"),
            )]
        })?;

        // Swapchain creation also establishes the per-frame, shadow, and
        // material descriptor layouts. Scene pipelines and framebuffers must
        // not be created until those Vulkan objects are valid.
        self.init_once()?;
        self.ensure_scene_framebuffers()?;

        let (ii, encoder) = self.device.begin_frame(sc_h).map_err(|e| {
            vec![Diagnostic::new(
                "RV0208",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("begin_frame: {e:?}"),
            )]
        })?;

        self.cur_fb_index = ii;

        // `begin_frame` waits for the current frame fence. Only now is it safe
        // to update the persistently mapped UBO owned by that frame slot.
        self.device.write_default_ubo();
        let view_projection = (projection * view).to_cols_array();
        let mut view_projection_bytes = Vec::with_capacity(64);
        for value in view_projection {
            view_projection_bytes.extend_from_slice(&value.to_ne_bytes());
        }
        self.device.write_ubo_current(&view_projection_bytes, 64);

        let camera_world = view.inverse().w_axis;
        let camera_position = [camera_world.x, camera_world.y, camera_world.z, 1.0f32];
        let mut camera_bytes = Vec::with_capacity(16);
        for value in camera_position {
            camera_bytes.extend_from_slice(&value.to_ne_bytes());
        }
        self.device.write_ubo_current(&camera_bytes, 160);

        Ok((sc_h, ii, encoder))
    }

    // ------------------------------------------------------------------
    // Extracted pass-execution helpers (called by registered passes)
    // ------------------------------------------------------------------

    /// Execute the opaque PBR forward pass (HDR offscreen).
    pub(crate) fn execute_hdr_forward_pass(
        &mut self,
        input: &RenderFrameInput,
        stats: &mut FrameStats,
    ) -> Result<(), Vec<Diagnostic>> {
        let hdr_rp = self.device.hdr_forward_rp.unwrap_or(vk::RenderPass::null());
        let hdr_fb = self
            .device
            .hdr_forward_fb
            .unwrap_or(vk::Framebuffer::null());
        let hdr_pl = self
            .device
            .hdr_forward_pipeline
            .unwrap_or(vk::Pipeline::null());
        let hdr_pll = self
            .device
            .hdr_forward_pipeline_layout
            .unwrap_or(vk::PipelineLayout::null());
        if hdr_rp == vk::RenderPass::null()
            || hdr_fb == vk::Framebuffer::null()
            || hdr_pl == vk::Pipeline::null()
            || hdr_pll == vk::PipelineLayout::null()
        {
            return Err(vec![Diagnostic::new(
                "RV0225",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "HDR forward pass resources are incomplete",
            )]);
        }

        // Clone device + cmd handles to avoid borrow-checker conflicts
        let d = self.device.logical_device.device.clone();
        let fi = self.device.current_frame;
        let cmd = self.device.frame_sync[fi].command_buffer;

        // Light setup: first directional -> UBO, rest -> SSBO
        let mut light_ssbo_data: Vec<u8> = Vec::new();
        let mut first_directional = true;

        for light in &input.lights {
            match light.kind {
                LightKind::Directional => {
                    let dir = normalize_dir(&light.direction);
                    if first_directional {
                        let mut dir_bytes = [0u8; 16];
                        for (j, &v) in dir.iter().enumerate() {
                            dir_bytes[j * 4..(j + 1) * 4].copy_from_slice(&v.to_ne_bytes());
                        }
                        dir_bytes[12..16].copy_from_slice(&0.0f32.to_ne_bytes());
                        self.device.write_ubo(fi, &dir_bytes, 128);

                        let mut col_bytes = [0u8; 16];
                        for (j, &v) in light.color.iter().enumerate() {
                            col_bytes[j * 4..(j + 1) * 4].copy_from_slice(&v.to_ne_bytes());
                        }
                        col_bytes[12..16].copy_from_slice(&light.intensity.to_ne_bytes());
                        self.device.write_ubo(fi, &col_bytes, 144);

                        first_directional = false;
                    } else {
                        light_ssbo_data.extend_from_slice(&pack_light_gpu_bytes(light, dir, 0.0));
                    }
                }
                LightKind::Point => {
                    let dir = [0.0f32; 3];
                    light_ssbo_data.extend_from_slice(&pack_light_gpu_bytes(light, dir, 1.0));
                }
                LightKind::Spot => {
                    let dir = normalize_dir(&light.direction);
                    light_ssbo_data.extend_from_slice(&pack_light_gpu_bytes(light, dir, 2.0));
                }
            }
        }

        if !light_ssbo_data.is_empty() {
            self.device.write_light_ssbo(&light_ssbo_data, 0);
        }

        // Begin HDR render pass with clear values.
        let msaa_active = self.device.hdr_msaa_samples != vk::SampleCountFlags::TYPE_1;
        let clear_values: &[vk::ClearValue] = &if msaa_active {
            vec![
                vk::ClearValue {
                    color: vk::ClearColorValue {
                        float32: [0.02, 0.02, 0.06, 1.0],
                    },
                },
                vk::ClearValue {
                    depth_stencil: vk::ClearDepthStencilValue {
                        depth: 1.0,
                        stencil: 0,
                    },
                },
                vk::ClearValue {
                    color: vk::ClearColorValue {
                        float32: [0.0, 0.0, 0.0, 0.0],
                    },
                },
            ]
        } else {
            vec![
                vk::ClearValue {
                    color: vk::ClearColorValue {
                        float32: [0.02, 0.02, 0.06, 1.0],
                    },
                },
                vk::ClearValue {
                    depth_stencil: vk::ClearDepthStencilValue {
                        depth: 1.0,
                        stencil: 0,
                    },
                },
            ]
        };
        let rpbi = vk::RenderPassBeginInfo::default()
            .render_pass(hdr_rp)
            .framebuffer(hdr_fb)
            .render_area(vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: vk::Extent2D {
                    width: self.width,
                    height: self.height,
                },
            })
            .clear_values(clear_values);
        // SAFETY: command buffer is in recording state; RP, FB valid.
        unsafe {
            d.cmd_begin_render_pass(cmd, &rpbi, vk::SubpassContents::INLINE);
        }

        // Viewport + scissor
        let vp = vk::Viewport {
            x: 0.0,
            y: 0.0,
            width: self.width as f32,
            height: self.height as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        };
        let sc = vk::Rect2D {
            offset: vk::Offset2D { x: 0, y: 0 },
            extent: vk::Extent2D {
                width: self.width,
                height: self.height,
            },
        };
        unsafe {
            d.cmd_set_viewport(cmd, 0, &[vp]);
            d.cmd_set_scissor(cmd, 0, &[sc]);
        }

        // Bind HDR forward pipeline
        unsafe {
            d.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, hdr_pl);
        }

        // Bind UBO descriptor set (set=0)
        if let Some(desc_set) = self.device.frame_descriptor_set(fi) {
            let sets = [desc_set];
            unsafe {
                d.cmd_bind_descriptor_sets(
                    cmd,
                    vk::PipelineBindPoint::GRAPHICS,
                    hdr_pll,
                    0,
                    &sets,
                    &[],
                );
            }
        }

        // Bind the shadow/environment/light descriptor set with the exact
        // layout used by the HDR pipeline. Earlier passes may have bound set=1
        // through a different pipeline layout, which does not guarantee that
        // the binding remains compatible after set=0 is rebound above.
        if let Some(desc_set) = self.device.shadow_desc_set {
            let sets = [desc_set];
            unsafe {
                d.cmd_bind_descriptor_sets(
                    cmd,
                    vk::PipelineBindPoint::GRAPHICS,
                    hdr_pll,
                    1,
                    &sets,
                    &[],
                );
            }
        }

        // Draw calls with dynamic batching (drawables pre-sorted by (material, mesh))
        let mut last_material_id: Option<&str> = None;
        let mut last_mesh_id: Option<&str> = None;
        #[allow(unused_assignments)]
        let mut cached_vb = vk::Buffer::null();
        #[allow(unused_assignments)]
        let mut cached_ib = vk::Buffer::null();
        #[allow(unused_assignments)]
        let mut cached_idx_ty = vk::IndexType::UINT32;
        let mut cached_index_count = 0u32;
        for drawable in &input.drawables {
            let mesh_id = &drawable.mesh.id;
            let material_id = &drawable.material.id;

            // Look up mesh buffers; cache across consecutive same-mesh drawables
            if Some(mesh_id.as_str()) != last_mesh_id {
                if let Some(m) = self.meshes.get(mesh_id).cloned() {
                    let vk_vb = self
                        .device
                        .buffers
                        .get(m.vertex_buffer.index, m.vertex_buffer.generation)
                        .map(|e| e.buffer)
                        .unwrap_or(vk::Buffer::null());
                    let vk_ib = self
                        .device
                        .buffers
                        .get(m.index_buffer.index, m.index_buffer.generation)
                        .map(|e| e.buffer)
                        .unwrap_or(vk::Buffer::null());
                    if vk_vb == vk::Buffer::null() {
                        last_material_id = None;
                        last_mesh_id = None;
                        cached_index_count = 0;
                        continue;
                    }
                    cached_vb = vk_vb;
                    cached_ib = vk_ib;
                    cached_idx_ty = vulkan_index_type(m.index_format);
                    cached_index_count = m.index_count;
                    last_mesh_id = Some(mesh_id.as_str());
                    // Bind VB/IB
                    let vbs = [cached_vb];
                    let offsets = [0u64];
                    unsafe {
                        d.cmd_bind_vertex_buffers(cmd, 0, &vbs, &offsets);
                        d.cmd_bind_index_buffer(cmd, cached_ib, 0, cached_idx_ty);
                    }
                } else {
                    tracing::trace!(
                        target: "scene_renderer",
                        mesh = mesh_id,
                        "skipping un-cached mesh in HDR forward pass"
                    );
                    last_material_id = None;
                    last_mesh_id = None;
                    continue;
                }
            }
            // (when same mesh, VB/IB are still bound — skip rebind)

            // Skip material descriptor rebind when same as last drawable
            if Some(material_id.as_str()) != last_material_id {
                let material = self.material_binding_for_drawable(input, &drawable.material);
                let material_ubo = Self::parse_material_ubo(&material.uniforms.bytes);
                let ubo_bytes: &[u8] = unsafe {
                    std::slice::from_raw_parts(
                        &material_ubo as *const _ as *const u8,
                        std::mem::size_of::<MaterialUBO>(),
                    )
                };
                let (mat_desc_set, _mat_buf) =
                    self.get_or_create_material_desc_set(material_id, ubo_bytes)?;
                self.bind_material_texture_if_changed(material_id, &material, mat_desc_set)?;
                let sets = [mat_desc_set];
                unsafe {
                    d.cmd_bind_descriptor_sets(
                        cmd,
                        vk::PipelineBindPoint::GRAPHICS,
                        hdr_pll,
                        2,
                        &sets,
                        &[],
                    );
                }
                last_material_id = Some(material_id.as_str());
            }

            // Push constants: world transform (128 B)
            let world = &drawable.world_transform;
            let mut pc_bytes = [0u8; 128];
            for (i, v) in world.iter().enumerate() {
                let bytes = v.to_ne_bytes();
                let offset = i * 4;
                if offset + 4 <= 128 {
                    pc_bytes[offset..offset + 4].copy_from_slice(&bytes);
                }
            }
            unsafe {
                d.cmd_push_constants(cmd, hdr_pll, vk::ShaderStageFlags::VERTEX, 0, &pc_bytes);
            }

            // Draw indexed
            unsafe {
                d.cmd_draw_indexed(cmd, cached_index_count, 1, 0, 0, 0);
            }

            stats.draw_calls += 1;
            stats.triangles += cached_index_count as u64 / 3;
        }

        // Skinned items (less batching opportunity due to unique per-item bone data)
        let mut last_skinned_mesh: Option<&str> = None;
        #[allow(unused_assignments)]
        let mut skinned_cached_vb = vk::Buffer::null();
        #[allow(unused_assignments)]
        let mut skinned_cached_ib = vk::Buffer::null();
        #[allow(unused_assignments)]
        let mut skinned_cached_idx_ty = vk::IndexType::UINT32;
        let mut skinned_cached_index_count = 0u32;
        for skinned in &input.skinned_items {
            let mesh_id = &skinned.mesh.id;
            let material_id = &skinned.material.id;

            // Cache VB/IB, skip on missing mesh
            if Some(mesh_id.as_str()) != last_skinned_mesh {
                match self.meshes.get(mesh_id).cloned() {
                    Some(m) => {
                        let vk_vb = self
                            .device
                            .buffers
                            .get(m.vertex_buffer.index, m.vertex_buffer.generation)
                            .map(|e| e.buffer)
                            .unwrap_or(vk::Buffer::null());
                        let vk_ib = self
                            .device
                            .buffers
                            .get(m.index_buffer.index, m.index_buffer.generation)
                            .map(|e| e.buffer)
                            .unwrap_or(vk::Buffer::null());
                        if vk_vb == vk::Buffer::null() {
                            last_skinned_mesh = None;
                            continue;
                        }
                        skinned_cached_vb = vk_vb;
                        skinned_cached_ib = vk_ib;
                        skinned_cached_index_count = m.index_count;
                        skinned_cached_idx_ty = vulkan_index_type(m.index_format);
                        last_skinned_mesh = Some(mesh_id.as_str());
                        let vbs = [skinned_cached_vb];
                        let offsets = [0u64];
                        unsafe {
                            d.cmd_bind_vertex_buffers(cmd, 0, &vbs, &offsets);
                            d.cmd_bind_index_buffer(
                                cmd,
                                skinned_cached_ib,
                                0,
                                skinned_cached_idx_ty,
                            );
                        }
                    }
                    None => {
                        tracing::trace!(
                            target: "scene_renderer",
                            mesh = mesh_id,
                            "skipping un-cached skinned mesh in HDR forward pass"
                        );
                        last_skinned_mesh = None;
                        continue;
                    }
                }
            }

            // Per-item: material descriptor, bone buffer, skinned descriptor set
            let material = self.material_binding_for_drawable(input, &skinned.material);
            let material_ubo = Self::parse_material_ubo(&material.uniforms.bytes);
            let ubo_bytes: &[u8] = unsafe {
                std::slice::from_raw_parts(
                    &material_ubo as *const _ as *const u8,
                    std::mem::size_of::<MaterialUBO>(),
                )
            };
            let (mat_desc_set, mat_buf) =
                self.get_or_create_material_desc_set(material_id, ubo_bytes)?;

            let skeleton_id = &skinned.skeleton.id;
            let bone_buf = self.get_or_create_bone_buffer(skeleton_id, &skinned.bone_palette)?;

            let skinned_desc_set = self.get_or_create_skinned_desc_set(
                material_id,
                skeleton_id,
                mat_desc_set,
                mat_buf,
                bone_buf,
            )?;

            let skinned_cache_key = format!("{material_id}:{skeleton_id}");
            self.bind_skinned_texture_if_changed(&skinned_cache_key, &material, skinned_desc_set)?;
            let sets = [skinned_desc_set];
            unsafe {
                d.cmd_bind_descriptor_sets(
                    cmd,
                    vk::PipelineBindPoint::GRAPHICS,
                    hdr_pll,
                    2,
                    &sets,
                    &[],
                );
            }

            let mut pc_bytes = Vec::with_capacity(128);
            for value in &skinned.world_transform {
                pc_bytes.extend_from_slice(&value.to_ne_bytes());
            }
            pc_bytes.resize(128, 0);
            unsafe {
                d.cmd_push_constants(cmd, hdr_pll, vk::ShaderStageFlags::VERTEX, 0, &pc_bytes);
                d.cmd_draw_indexed(cmd, skinned_cached_index_count, 1, 0, 0, 0);
            }

            stats.draw_calls += 1;
            stats.triangles += skinned_cached_index_count as u64 / 3;
        }

        // Scene extraction owns frustum culling. RenderFrameInput contains the
        // visible working set, so issuing a second indirect pass here would
        // draw every visible static object twice.

        // End HDR render pass
        unsafe {
            d.cmd_end_render_pass(cmd);
        }

        // Barrier: HDR color attachment -> shader read-only
        if let Some(hdr_img) = self.device.hdr_color_image {
            let barrier = vk::ImageMemoryBarrier::default()
                .image(hdr_img)
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
            unsafe {
                d.cmd_pipeline_barrier(
                    cmd,
                    vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                    vk::PipelineStageFlags::FRAGMENT_SHADER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &[barrier],
                );
            }
        }

        apply_extraction_stats(stats, input);
        Ok(())
    }

    /// Execute the directional shadow (CSM) pass.
    pub(crate) fn execute_shadow_pass(
        &mut self,
        input: &RenderFrameInput,
        stats: &mut FrameStats,
    ) -> Result<(), Vec<Diagnostic>> {
        let Some(shadow_light) = input.lights.iter().find(|light| {
            light.kind == LightKind::Directional
                && matches!(light.shadow_mode, ShadowMode::Hard | ShadowMode::Soft)
        }) else {
            // No directional light requested a shadow map this frame. Do not
            // manufacture a fixed light or issue stale/fake shadow draws.
            apply_extraction_stats(stats, input);
            return Ok(());
        };

        let light_direction = VulkanDevice::normalize_shadow_light_direction(glam::Vec3::from(
            shadow_light.direction,
        ))
        .map_err(|error| {
            vec![Diagnostic::new(
                "RV0286",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("invalid directional shadow light: {error}"),
            )]
        })?;

        let view = input.views.first().ok_or_else(|| {
            vec![Diagnostic::new(
                "RV0287",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "directional shadow pass requires a RenderView",
            )]
        })?;
        let view_mat = Mat4::from_cols_array(&view.view_matrix);
        let proj_mat = Mat4::from_cols_array(&view.projection_matrix);
        let (near, far) = VulkanDevice::derive_rh_zo_clip_planes(&proj_mat).map_err(|error| {
            vec![Diagnostic::new(
                "RV0288",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("cannot derive directional-shadow clip planes: {error}"),
            )]
        })?;
        let (cascade_splits, light_vps) =
            VulkanDevice::compute_cascade_data(&view_mat, &proj_mat, near, far, light_direction)
                .map_err(|error| {
                    vec![Diagnostic::new(
                        "RV0289",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        format!("cannot compute directional-shadow cascades: {error}"),
                    )]
                })?;

        let rp = self.device.shadow_rp.unwrap_or(vk::RenderPass::null());
        let pll = self
            .device
            .shadow_pipeline_layout
            .unwrap_or(vk::PipelineLayout::null());
        let pl = self.device.shadow_pipeline.unwrap_or(vk::Pipeline::null());
        if rp == vk::RenderPass::null()
            || pll == vk::PipelineLayout::null()
            || pl == vk::Pipeline::null()
        {
            return Err(vec![Diagnostic::new(
                "RV0226",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "directional shadow pass resources are incomplete",
            )]);
        }

        const SHADOW_SIZE: u32 = 2048;
        const CASCADE_COUNT: usize = 3;

        let splits_bytes: &[u8] =
            unsafe { std::slice::from_raw_parts(&cascade_splits as *const _ as *const u8, 16) };
        self.device.write_ubo_current(splits_bytes, 176);

        for (i, lvp) in light_vps.iter().enumerate() {
            let arr: [[f32; 4]; 4] = lvp.to_cols_array_2d();
            let vp_bytes: &[u8] =
                unsafe { std::slice::from_raw_parts(&arr as *const _ as *const u8, 64) };
            self.device
                .write_ubo_current(vp_bytes, 192 + (i as u64 * 64));
        }

        let d = &self.device.logical_device.device;
        let fi = self.device.current_frame;
        let cmd = self.device.frame_sync[fi].command_buffer;

        let clear_value = vk::ClearValue {
            depth_stencil: vk::ClearDepthStencilValue {
                depth: 1.0,
                stencil: 0,
            },
        };
        let clear_values = [clear_value];

        #[allow(clippy::needless_range_loop)]
        for cascade in 0..CASCADE_COUNT {
            let fb = match self.device.shadow_fbs.get(cascade).copied() {
                Some(fb) => fb,
                None => continue,
            };

            let rpbi = vk::RenderPassBeginInfo::default()
                .render_pass(rp)
                .framebuffer(fb)
                .render_area(vk::Rect2D {
                    offset: vk::Offset2D { x: 0, y: 0 },
                    extent: vk::Extent2D {
                        width: SHADOW_SIZE,
                        height: SHADOW_SIZE,
                    },
                })
                .clear_values(&clear_values);
            unsafe {
                d.cmd_begin_render_pass(cmd, &rpbi, vk::SubpassContents::INLINE);
            }

            let vp = vk::Viewport {
                x: 0.0,
                y: 0.0,
                width: SHADOW_SIZE as f32,
                height: SHADOW_SIZE as f32,
                min_depth: 0.0,
                max_depth: 1.0,
            };
            unsafe {
                d.cmd_set_viewport(cmd, 0, &[vp]);
                d.cmd_set_scissor(
                    cmd,
                    0,
                    &[vk::Rect2D {
                        offset: vk::Offset2D { x: 0, y: 0 },
                        extent: vk::Extent2D {
                            width: SHADOW_SIZE,
                            height: SHADOW_SIZE,
                        },
                    }],
                );
                d.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, pl);
            }

            let light_vp = light_vps[cascade];

            // Shadow draws with batching (drawables pre-sorted by mesh)
            let mut last_shadow_mesh: Option<&str> = None;
            #[allow(unused_assignments)]
            let mut shadow_cached_vb = vk::Buffer::null();
            #[allow(unused_assignments)]
            let mut shadow_cached_ib = vk::Buffer::null();
            let mut shadow_cached_index_count = 0u32;
            for drawable in &input.drawables {
                if !drawable.cast_shadows {
                    last_shadow_mesh = None;
                    continue;
                }

                let mesh_id = &drawable.mesh.id;

                if Some(mesh_id.as_str()) != last_shadow_mesh {
                    match self.meshes.get(mesh_id).cloned() {
                        Some(m) => {
                            let vk_vb = self
                                .device
                                .buffers
                                .get(m.vertex_buffer.index, m.vertex_buffer.generation)
                                .map(|e| e.buffer)
                                .unwrap_or(vk::Buffer::null());
                            let vk_ib = self
                                .device
                                .buffers
                                .get(m.index_buffer.index, m.index_buffer.generation)
                                .map(|e| e.buffer)
                                .unwrap_or(vk::Buffer::null());
                            if vk_vb == vk::Buffer::null() || vk_ib == vk::Buffer::null() {
                                last_shadow_mesh = None;
                                continue;
                            }
                            shadow_cached_vb = vk_vb;
                            shadow_cached_ib = vk_ib;
                            shadow_cached_index_count = m.index_count;
                            let shadow_index_type = vulkan_index_type(m.index_format);
                            last_shadow_mesh = Some(mesh_id.as_str());
                            // Bind VB/IB
                            let vbs = [shadow_cached_vb];
                            let offsets = [0u64];
                            unsafe {
                                d.cmd_bind_vertex_buffers(cmd, 0, &vbs, &offsets);
                                d.cmd_bind_index_buffer(
                                    cmd,
                                    shadow_cached_ib,
                                    0,
                                    shadow_index_type,
                                );
                            }
                        }
                        None => {
                            tracing::trace!(
                                target: "scene_renderer",
                                mesh = mesh_id,
                                "skipping un-cached mesh in shadow pass"
                            );
                            last_shadow_mesh = None;
                            continue;
                        }
                    }
                }

                let world = Mat4::from_cols_array(&drawable.world_transform);
                let mvp = light_vp * world;
                unsafe {
                    let mvp_bytes: &[u8] = std::slice::from_raw_parts(
                        &mvp as *const _ as *const u8,
                        std::mem::size_of::<Mat4>(),
                    );
                    d.cmd_push_constants(cmd, pll, vk::ShaderStageFlags::VERTEX, 0, mvp_bytes);
                    d.cmd_draw_indexed(cmd, shadow_cached_index_count, 1, 0, 0, 0);
                }

                stats.draw_calls += 1;
                stats.triangles += shadow_cached_index_count as u64 / 3;
            }

            unsafe {
                d.cmd_end_render_pass(cmd);
            }
        }

        // Global barrier: cascade layers -> shader readable
        if let Some(sm) = self.device.shadow_map {
            let barrier = vk::ImageMemoryBarrier::default()
                .image(sm)
                .subresource_range(vk::ImageSubresourceRange {
                    aspect_mask: vk::ImageAspectFlags::DEPTH,
                    base_mip_level: 0,
                    level_count: 1,
                    base_array_layer: 0,
                    layer_count: CASCADE_COUNT as u32,
                })
                .src_access_mask(vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE)
                .dst_access_mask(vk::AccessFlags::SHADER_READ)
                .old_layout(vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL)
                .new_layout(vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL);
            unsafe {
                d.cmd_pipeline_barrier(
                    cmd,
                    vk::PipelineStageFlags::LATE_FRAGMENT_TESTS,
                    vk::PipelineStageFlags::FRAGMENT_SHADER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &[barrier],
                );
            }
        }

        apply_extraction_stats(stats, input);
        Ok(())
    }

    /// Execute the tone-mapping pass (HDR -> LDR to swapchain).
    pub(crate) fn execute_tonemap_pass(
        &mut self,
        input: &RenderFrameInput,
        stats: &mut FrameStats,
    ) -> Result<(), Vec<Diagnostic>> {
        let Some(ref mut enc) = self.cur_enc else {
            return Err(vec![Diagnostic::new(
                "RV0227",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "tone-map pass requires an active frame encoder",
            )]);
        };

        let d = &self.device.logical_device.device;
        let fi = self.device.current_frame;
        let cmd = self.device.frame_sync[fi].command_buffer;

        let tone_rp = self.device.tone_rp.unwrap_or(vk::RenderPass::null());
        let tone_pl = self.device.tone_pipeline.unwrap_or(vk::Pipeline::null());
        let tone_pll = self
            .device
            .tone_pipeline_layout
            .unwrap_or(vk::PipelineLayout::null());
        let tone_ds = self
            .device
            .tone_desc_set
            .unwrap_or(vk::DescriptorSet::null());
        if tone_rp == vk::RenderPass::null()
            || tone_pl == vk::Pipeline::null()
            || tone_pll == vk::PipelineLayout::null()
            || tone_ds == vk::DescriptorSet::null()
        {
            return Err(vec![Diagnostic::new(
                "RV0228",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "tone-map pass resources are incomplete",
            )]);
        }

        let tone_fb = self
            .device
            .tone_framebuffers
            .get(self.cur_fb_index as usize)
            .copied()
            .unwrap_or(vk::Framebuffer::null());
        if tone_fb == vk::Framebuffer::null() {
            return Err(vec![Diagnostic::new(
                "RV0229",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "tone-map framebuffer is missing for the acquired swapchain image",
            )]);
        }

        let rpbi = vk::RenderPassBeginInfo::default()
            .render_pass(tone_rp)
            .framebuffer(tone_fb)
            .render_area(vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: vk::Extent2D {
                    width: self.width,
                    height: self.height,
                },
            });
        unsafe {
            d.cmd_begin_render_pass(cmd, &rpbi, vk::SubpassContents::INLINE);
        }

        enc.set_viewport(0.0, 0.0, self.width as f32, self.height as f32, 0.0, 1.0);
        enc.set_scissor(0, 0, self.width, self.height);

        unsafe {
            d.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, tone_pl);
        }

        if tone_ds != vk::DescriptorSet::null() {
            let sets = [tone_ds];
            unsafe {
                d.cmd_bind_descriptor_sets(
                    cmd,
                    vk::PipelineBindPoint::GRAPHICS,
                    tone_pll,
                    0,
                    &sets,
                    &[],
                );
            }
        }

        let identity: [u8; 128] = [0; 128];
        unsafe {
            d.cmd_push_constants(cmd, tone_pll, vk::ShaderStageFlags::VERTEX, 0, &identity);
        }

        enc.draw(3, 1, 0, 0);
        enc.end_render_pass();

        let _ = input;
        let _ = stats;
        Ok(())
    }

    // ── UI overlay rendering ─────────────────────────────────────────────

    /// Render UI batches after the main 3D scene, within the same render pass.
    fn render_ui_overlay(
        &mut self,
        encoder: &mut dyn CommandEncoder,
        ui_pl: PipelineHandle,
        ui_pll: PipelineLayoutHandle,
        batches: &[UiBatch],
        draw_calls: &mut u32,
    ) -> Result<(), Vec<Diagnostic>> {
        if batches.is_empty() {
            return Ok(());
        }

        // Count total vertices (non-indexed: 6 vertices per quad)
        let mut total_verts = 0usize;
        for batch in batches {
            total_verts += (batch.indices.len() / 6) * 6;
        }
        if total_verts == 0 {
            return Ok(());
        }

        // Build interleaved vertex data: [pos2, uv2, color4] = 32 bytes per vertex
        let stride: u64 = 32;
        let vb_size = total_verts as u64 * stride;

        // Expand indexed quads to non-indexed triangles
        let mut vert_bytes: Vec<u8> = Vec::with_capacity(total_verts * stride as usize);
        for batch in batches {
            let verts = &batch.vertices;
            for chunk in batch.indices.chunks(6) {
                for &idx in chunk {
                    let v = &verts[idx as usize];
                    vert_bytes.extend_from_slice(&v.position[0].to_ne_bytes());
                    vert_bytes.extend_from_slice(&v.position[1].to_ne_bytes());
                    vert_bytes.extend_from_slice(&v.uv[0].to_ne_bytes());
                    vert_bytes.extend_from_slice(&v.uv[1].to_ne_bytes());
                    let r = v.color[0] as f32 / 255.0;
                    let g = v.color[1] as f32 / 255.0;
                    let b = v.color[2] as f32 / 255.0;
                    let a = v.color[3] as f32 / 255.0;
                    vert_bytes.extend_from_slice(&r.to_ne_bytes());
                    vert_bytes.extend_from_slice(&g.to_ne_bytes());
                    vert_bytes.extend_from_slice(&b.to_ne_bytes());
                    vert_bytes.extend_from_slice(&a.to_ne_bytes());
                }
            }
        }

        // Ensure vertex buffer is large enough
        if self.ui_vb_capacity < vb_size {
            if let Some(old) = self.ui_vb.take() {
                self.device.destroy_buffer(old);
            }
            let vb_desc = BufferDescriptor {
                size_bytes: vb_size,
                usage_flags: render_core::BufferUsage(0),
                memory_hint: MemoryHint::CpuToGpu,
                debug_label: Some("ui-overlay-vb".into()),
            };
            let vb = self.device.create_buffer(&vb_desc).map_err(|e| {
                vec![Diagnostic::new(
                    "RV0216",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("create_ui_vb: {e:?}"),
                )]
            })?;
            self.ui_vb = Some(vb);
            self.ui_vb_capacity = vb_size;
        }

        // Write vertex data
        if let Some(vb) = self.ui_vb {
            self.device.write_buffer(vb, &vert_bytes, 0).map_err(|e| {
                vec![Diagnostic::new(
                    "RV0217",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("write_ui_vb: {e:?}"),
                )]
            })?;

            encoder.bind_pipeline(ui_pl);

            let mut pc = Vec::with_capacity(8);
            pc.extend_from_slice(&(self.width as f32).to_ne_bytes());
            pc.extend_from_slice(&(self.height as f32).to_ne_bytes());
            encoder.push_constants(ui_pll, 0x01, 0, &pc);

            encoder.bind_vertex_buffers(&[vb], &[0]);
            encoder.draw(total_verts as u32, 1, 0, 0);
            *draw_calls += 1;
        }

        Ok(())
    }

    fn recover_failed_device_frame(&mut self) -> Result<(), Vec<Diagnostic>> {
        self.device
            .abort_current_frame_recording()
            .map_err(|error| {
                vec![Diagnostic::new(
                    "RV0244",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("failed to recover Vulkan frame state: {error:?}"),
                )]
            })?;
        self.device.destroy_scene_framebuffers(&self.framebuffers);
        self.framebuffers.clear();
        self.device.resize(self.width, self.height);
        Ok(())
    }
}

// ============================================================================
// Retained single-pass implementation
// ============================================================================

impl SceneRenderer {
    // ------------------------------------------------------------------
    // Single-pass legacy path
    // ------------------------------------------------------------------

    #[allow(dead_code)]
    fn render_frame_legacy(
        &mut self,
        input: &RenderFrameInput,
    ) -> Result<FrameStats, Vec<Diagnostic>> {
        let msaa = self.device.msaa_samples(input.render_options.msaa_samples);
        let (sc_h, ii, mut encoder) = self.begin_frame_impl(input, msaa)?;

        // Begin a render pass covering the full viewport.
        if let Some(rp) = self.rp {
            // Real framebuffer from per-swapchain-image handles.
            let fb = self
                .framebuffers
                .get(self.cur_fb_index as usize)
                .copied()
                .unwrap_or(FramebufferHandle::new(0, 0));
            encoder.begin_render_pass(
                rp,
                fb,
                (0, 0, self.width, self.height),
                [0.02, 0.02, 0.06, 1.0],
                Some(1.0),
            );
        }

        encoder.set_viewport(0.0, 0.0, self.width as f32, self.height as f32, 0.0, 1.0);
        encoder.set_scissor(0, 0, self.width, self.height);

        if let Some(pll) = self.pll {
            encoder.bind_descriptor_sets(pll, 0, &[], &[]);
        }

        let mut draw_calls: u32 = 0;
        let mut triangles: u64 = 0;

        for drawable in &input.drawables {
            let mesh_id = &drawable.mesh.id;
            let mesh = match self.get_or_create_mesh(mesh_id) {
                Ok(m) => m,
                Err(diags) => {
                    tracing::warn!(
                        target: "scene_renderer",
                        mesh = mesh_id,
                        "skipping drawable, mesh creation failed"
                    );
                    for d in &diags {
                        tracing::warn!(target: "scene_renderer", code = d.code, message = d.message);
                    }
                    continue;
                }
            };

            let sample_count = input.render_options.msaa_samples;
            let pipeline = self.pipeline_for_drawable(input, drawable, sample_count)?;
            encoder.bind_pipeline(pipeline);

            // --- Material UBO (set=2) ---
            let material_id = &drawable.material.id;
            let material = self.material_binding_for_drawable(input, &drawable.material);
            let material_ubo = Self::parse_material_ubo(&material.uniforms.bytes);
            let ubo_bytes: &[u8] = unsafe {
                std::slice::from_raw_parts(
                    &material_ubo as *const _ as *const u8,
                    std::mem::size_of::<MaterialUBO>(),
                )
            };
            let (mat_desc_set, _mat_buf) =
                self.get_or_create_material_desc_set(material_id, ubo_bytes)?;
            if let Some(pll) = self.pll {
                let pll_vk = self
                    .device
                    .pipeline_layouts
                    .get(pll.index, pll.generation)
                    .map(|e| e.layout)
                    .unwrap_or(vk::PipelineLayout::null());
                if pll_vk != vk::PipelineLayout::null() {
                    let d = &self.device.logical_device.device;
                    let fi = self.device.current_frame;
                    let cmd = self.device.frame_sync[fi].command_buffer;
                    let sets = [mat_desc_set];
                    // SAFETY: command buffer is in recording state; descriptor
                    // set and pipeline layout are valid.
                    unsafe {
                        d.cmd_bind_descriptor_sets(
                            cmd,
                            vk::PipelineBindPoint::GRAPHICS,
                            pll_vk,
                            2,
                            &sets,
                            &[],
                        );
                    }
                }
            }

            self.bind_material_texture_if_changed(material_id, &material, mat_desc_set)?;

            // Push the world transform as push constants (placeholder MVP).
            if let Some(pll) = self.pll {
                let world = &drawable.world_transform; // [f32; 16]
                let mut pc_bytes = Vec::with_capacity(128);
                for v in world {
                    pc_bytes.extend_from_slice(&v.to_ne_bytes());
                }
                pc_bytes.resize(128, 0u8);
                encoder.push_constants(pll, 0x01, 0, &pc_bytes);
            }

            encoder.bind_vertex_buffers(&[mesh.vertex_buffer], &[0]);
            encoder.bind_index_buffer(mesh.index_buffer, 0, mesh.index_format);
            encoder.draw_indexed(mesh.index_count, 1, 0, 0, 0);

            draw_calls += 1;
            triangles += mesh.index_count as u64 / 3;
        }

        // ── Skinned items ──────────────────────────────────────────────
        let sample_count = input.render_options.msaa_samples;
        for skinned in &input.skinned_items {
            let mesh_id = &skinned.mesh.id;
            let mesh = match self.get_or_create_mesh(mesh_id) {
                Ok(m) => m,
                Err(diags) => {
                    tracing::warn!(
                        target: "scene_renderer",
                        mesh = mesh_id,
                        "skipping skinned drawable, mesh creation failed"
                    );
                    for d in &diags {
                        tracing::warn!(target: "scene_renderer", code = d.code, message = d.message);
                    }
                    continue;
                }
            };

            let pipeline = self.pipeline_for_skinned_drawable(input, skinned, sample_count)?;
            encoder.bind_pipeline(pipeline);

            // --- Material UBO (set=2, binding=0) ---
            let material_id = &skinned.material.id;
            let material = self.material_binding_for_drawable(input, &skinned.material);
            let material_ubo = Self::parse_material_ubo(&material.uniforms.bytes);
            let ubo_bytes: &[u8] = unsafe {
                std::slice::from_raw_parts(
                    &material_ubo as *const _ as *const u8,
                    std::mem::size_of::<MaterialUBO>(),
                )
            };
            let (mat_desc_set, mat_buf) =
                self.get_or_create_material_desc_set(material_id, ubo_bytes)?;

            // --- Bone palette UBO (set=2, binding=2) ---
            let skeleton_id = &skinned.skeleton.id;
            let bone_buf = self.get_or_create_bone_buffer(skeleton_id, &skinned.bone_palette)?;

            // --- Combined descriptor set (material + bone) ---
            let skinned_desc_set = self.get_or_create_skinned_desc_set(
                material_id,
                skeleton_id,
                mat_desc_set,
                mat_buf,
                bone_buf,
            )?;

            if let Some(pll) = self.pll {
                let pll_vk = self
                    .device
                    .pipeline_layouts
                    .get(pll.index, pll.generation)
                    .map(|e| e.layout)
                    .unwrap_or(vk::PipelineLayout::null());
                if pll_vk != vk::PipelineLayout::null() {
                    let d = &self.device.logical_device.device;
                    let fi = self.device.current_frame;
                    let cmd = self.device.frame_sync[fi].command_buffer;
                    let sets = [skinned_desc_set];
                    // SAFETY: command buffer is in recording state; descriptor
                    // set and pipeline layout are valid.
                    unsafe {
                        d.cmd_bind_descriptor_sets(
                            cmd,
                            vk::PipelineBindPoint::GRAPHICS,
                            pll_vk,
                            2,
                            &sets,
                            &[],
                        );
                    }
                }
            }

            let skinned_cache_key = format!("{material_id}:{skeleton_id}");
            self.bind_skinned_texture_if_changed(&skinned_cache_key, &material, skinned_desc_set)?;

            // The skinned shader applies this drawable world matrix after the
            // bone palette's model-space deformation.
            if let Some(pll) = self.pll {
                let mut pc_bytes = Vec::with_capacity(128);
                for value in &skinned.world_transform {
                    pc_bytes.extend_from_slice(&value.to_ne_bytes());
                }
                pc_bytes.resize(128, 0);
                encoder.push_constants(pll, 0x01, 0, &pc_bytes);
            }

            encoder.bind_vertex_buffers(&[mesh.vertex_buffer], &[0]);
            encoder.bind_index_buffer(mesh.index_buffer, 0, mesh.index_format);
            encoder.draw_indexed(mesh.index_count, 1, 0, 0, 0);

            draw_calls += 1;
            triangles += mesh.index_count as u64 / 3;
        }

        // ── UI overlay ────────────────────────────────────────────────
        if let (Some(ui_pl), Some(ui_pll)) = (self.ui_pl, self.ui_pll) {
            if !input.ui_batches.is_empty() {
                self.render_ui_overlay(
                    &mut *encoder,
                    ui_pl,
                    ui_pll,
                    &input.ui_batches,
                    &mut draw_calls,
                )?;
            }
        }

        encoder.end_render_pass();

        let stats = self.device.end_frame(sc_h, encoder, ii).map_err(|e| {
            vec![Diagnostic::new(
                "RV0209",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("end_frame: {e:?}"),
            )]
        })?;

        let extraction = extraction_stats(input);
        Ok(FrameStats {
            visible_drawables: extraction.visible_drawables,
            visible_lights: extraction.visible_lights,
            culled_drawables: extraction.culled_drawables,
            culled_lights: extraction.culled_lights,
            draw_calls,
            triangles,
            gpu_frame_ms: stats.gpu_frame_ms,
        })
    }
}

// ============================================================================
// BackendRenderer implementation
// ============================================================================

impl BackendRenderer for SceneRenderer {
    fn render_frame(&mut self, input: &RenderFrameInput) -> Result<FrameStats, Vec<Diagnostic>> {
        let diagnostics = engine_renderer::validate_frame_input(input);
        if diagnostics.iter().any(|diagnostic| {
            matches!(
                diagnostic.severity,
                DiagnosticSeverity::Error | DiagnosticSeverity::Fatal
            )
        }) {
            return Err(diagnostics);
        }

        let graph = engine_renderer::render_graph2::RenderGraph::build_with_config(
            input,
            &input.render_options.pass_graph_config,
        );
        if let Some(name) = graph.passes.iter().find_map(|pass| match &pass.kind {
            engine_renderer::render_graph2::PassKind::Custom(name)
                if self.pass_registry.find(name).is_none() =>
            {
                Some(*name)
            }
            _ => None,
        }) {
            return Err(vec![Diagnostic::new(
                "RV0291",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("render graph references unregistered custom pass '{name}'"),
            )]);
        }
        let compiled = graph.compile_v2().map_err(|error| {
            vec![Diagnostic::new(
                "RV0284",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("render graph compile failed: {error}"),
            )]
        })?;
        let abort_after_error = |renderer: &mut SceneRenderer, mut diagnostics: Vec<Diagnostic>| {
            if let Err(mut abort_diagnostics) = renderer.abort_frame() {
                diagnostics.append(&mut abort_diagnostics);
            }
            diagnostics
        };

        let mut stats = FrameStats::default();
        self.begin_frame(input)?;
        for (compiled_index, &pass_index) in compiled.pass_order.iter().enumerate() {
            let Some(pass) = graph.passes.get(pass_index) else {
                let diagnostics = vec![Diagnostic::new(
                    "RV0285",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("compiled graph referenced missing pass index {pass_index}"),
                )];
                return Err(abort_after_error(self, diagnostics));
            };
            let barriers = compiled
                .barriers_per_pass
                .get(compiled_index)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            if let Some(legacy_pass) = pass.to_legacy() {
                if let Err(diagnostics) = self.apply_pass_barriers(input, &legacy_pass, barriers) {
                    return Err(abort_after_error(self, diagnostics));
                }
                if let Err(diagnostics) = self.execute_pass(input, &legacy_pass, &mut stats) {
                    return Err(abort_after_error(self, diagnostics));
                }
            }
        }
        if let Err(diagnostics) = self.end_frame(&mut stats) {
            return Err(abort_after_error(self, diagnostics));
        }
        Ok(stats)
    }

    // ------------------------------------------------------------------
    // Multi-pass graph path
    // ------------------------------------------------------------------

    fn apply_pass_barriers(
        &mut self,
        _input: &RenderFrameInput,
        _pass: &render_graph::PassNode,
        barriers: &[engine_renderer::render_graph::CompiledBarrier],
    ) -> Result<(), Vec<Diagnostic>> {
        let fi = self.device.current_frame;
        self.device.apply_render_graph_barriers(fi, barriers);
        Ok(())
    }

    fn begin_frame(&mut self, input: &RenderFrameInput) -> Result<(), Vec<Diagnostic>> {
        if self.cur_enc.is_some() || self.cur_sc.is_some() || self.cur_ii.is_some() {
            return Err(vec![Diagnostic::new(
                "RV0269",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "begin_frame called while another frame is active",
            )]);
        }
        if input.views.len() > 1 {
            return Err(vec![Diagnostic::new(
                "RV0290",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!(
                    "Vulkan backend currently supports at most one render view, received {}",
                    input.views.len()
                ),
            )]);
        }
        let msaa = self.device.msaa_samples(input.render_options.msaa_samples);
        let (sc_h, ii, enc) = match self.begin_frame_impl(input, msaa) {
            Ok(frame) => frame,
            Err(mut diagnostics) => {
                if let Err(mut recovery_diagnostics) = self.recover_failed_device_frame() {
                    diagnostics.append(&mut recovery_diagnostics);
                }
                return Err(diagnostics);
            }
        };
        self.cur_sc = Some(sc_h);
        self.cur_ii = Some(ii);
        self.cur_enc = Some(enc);
        Ok(())
    }

    fn execute_pass(
        &mut self,
        input: &RenderFrameInput,
        pass: &render_graph::PassNode,
        stats: &mut FrameStats,
    ) -> Result<(), Vec<Diagnostic>> {
        if self.cur_enc.is_none() {
            return Err(vec![Diagnostic::new(
                "RV0224",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "execute_pass called without an active frame encoder",
            )]);
        }

        // Built-in passes own backend-specific Vulkan resources and must be
        // dispatched directly. In particular, the tone-map pass performs the
        // render-pass transition from UNDEFINED to PRESENT_SRC_KHR for the
        // acquired swapchain image. Custom passes continue to use the
        // pluggable registry.
        match pass.kind {
            render_graph::PassKind::OpaquePbrForward => {
                self.execute_hdr_forward_pass(input, stats)?;
            }
            render_graph::PassKind::DirectionalShadow => {
                self.execute_shadow_pass(input, stats)?;
            }
            render_graph::PassKind::ToneMap => {
                self.execute_tonemap_pass(input, stats)?;
            }
            render_graph::PassKind::Present => {}
            render_graph::PassKind::Custom(name) => {
                if let Some(rp) = self.pass_registry.find_mut(name) {
                    if rp.is_enabled(input) {
                        let enc = self.cur_enc.as_mut().expect("encoder checked above");
                        rp.execute(input, &mut **enc, stats)?;
                    }
                } else {
                    return Err(vec![Diagnostic::new(
                        "RV0291",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        format!("render graph references unregistered custom pass '{name}'"),
                    )]);
                }
            }
        }

        Ok(())
    }

    fn end_frame(&mut self, stats: &mut FrameStats) -> Result<(), Vec<Diagnostic>> {
        match (self.cur_sc.take(), self.cur_ii.take(), self.cur_enc.take()) {
            (Some(sc_h), Some(ii), Some(enc)) => {
                // SAFETY: the encoder was created by `begin_frame` and is still
                // valid; `end_frame` takes ownership and submits the command
                // buffer that has been recorded into during `execute_pass`.
                let s = match self.device.end_frame(sc_h, enc, ii) {
                    Ok(stats) => stats,
                    Err(error) => {
                        let mut diagnostics = vec![Diagnostic::new(
                            "RV0209",
                            DiagnosticSeverity::Error,
                            "scene_renderer",
                            format!("end_frame: {error:?}"),
                        )];
                        if let Err(mut recovery_diagnostics) = self.recover_failed_device_frame() {
                            diagnostics.append(&mut recovery_diagnostics);
                        }
                        return Err(diagnostics);
                    }
                };
                // Built-in Vulkan passes issue several draws directly because
                // they need backend-specific descriptor and render-pass state.
                // The generic encoder only accounts for the draws recorded
                // through its own methods (for example the tone-map pass), so
                // replacing the pass totals here would erase the scene work.
                stats.draw_calls = stats.draw_calls.saturating_add(s.draw_calls);
                stats.triangles = stats.triangles.saturating_add(s.triangles);
                stats.gpu_frame_ms = s.gpu_frame_ms;
            }
            (None, None, None) => {
                return Err(vec![Diagnostic::new(
                    "RV0267",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    "end_frame called without an active frame",
                )]);
            }
            _ => {
                let mut diagnostics = vec![Diagnostic::new(
                    "RV0268",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    "Vulkan frame state is internally inconsistent",
                )];
                if let Err(mut recovery_diagnostics) = self.recover_failed_device_frame() {
                    diagnostics.append(&mut recovery_diagnostics);
                }
                return Err(diagnostics);
            }
        }
        Ok(())
    }

    fn upload_mesh(&mut self, upload: MeshUpload) -> Result<UploadReceipt, Vec<Diagnostic>> {
        let mesh_id = upload.mesh_id.id.clone();
        if let Some(existing) = self.meshes.get(&mesh_id) {
            if existing.content_hash == upload.content_hash {
                return Ok(UploadReceipt::new(existing.revision));
            }
        }
        let revision = self
            .meshes
            .get(&mesh_id)
            .map_or(1, |existing| existing.revision.saturating_add(1));

        let vb_usage = render_core::BufferUsage(
            render_core::BufferUsage::VERTEX.0 | render_core::BufferUsage::COPY_DST.0,
        );
        let vb_desc = render_core::BufferDescriptor {
            size_bytes: upload.vertex_bytes.len() as u64,
            usage_flags: vb_usage,
            memory_hint: render_core::MemoryHint::CpuToGpu,
            debug_label: Some(format!("mesh-{mesh_id}-vertices")),
        };
        let vb = self.device.create_buffer(&vb_desc).map_err(|e| {
            vec![Diagnostic::new(
                "RV0203",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("upload_mesh create_buffer(vertices): {e:?}"),
            )]
        })?;
        if let Err(error) = self.device.write_buffer(vb, &upload.vertex_bytes, 0) {
            self.device.destroy_buffer(vb);
            return Err(vec![Diagnostic::new(
                "RV0204",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("upload_mesh write_buffer(vertices): {error:?}"),
            )]);
        }

        let ib_usage = render_core::BufferUsage(
            render_core::BufferUsage::INDEX.0 | render_core::BufferUsage::COPY_DST.0,
        );
        let ib_desc = render_core::BufferDescriptor {
            size_bytes: upload.index_bytes.len() as u64,
            usage_flags: ib_usage,
            memory_hint: render_core::MemoryHint::CpuToGpu,
            debug_label: Some(format!("mesh-{mesh_id}-indices")),
        };
        let ib = match self.device.create_buffer(&ib_desc) {
            Ok(buffer) => buffer,
            Err(error) => {
                self.device.destroy_buffer(vb);
                return Err(vec![Diagnostic::new(
                    "RV0205",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("upload_mesh create_buffer(indices): {error:?}"),
                )]);
            }
        };
        if let Err(error) = self.device.write_buffer(ib, &upload.index_bytes, 0) {
            self.device.destroy_buffer(ib);
            self.device.destroy_buffer(vb);
            return Err(vec![Diagnostic::new(
                "RV0206",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("upload_mesh write_buffer(indices): {error:?}"),
            )]);
        }

        let index_format = match upload.index_format {
            engine_renderer::IndexFormat::U16 => IndexFormat::U16,
            engine_renderer::IndexFormat::U32 => IndexFormat::U32,
        };
        let mesh = GpuMesh {
            vertex_buffer: vb,
            index_buffer: ib,
            index_count: upload.index_count,
            index_format,
            content_hash: upload.content_hash,
            revision,
        };

        if self.meshes.contains_key(&mesh_id) {
            if let Err(error) = self.device.wait_idle_checked() {
                self.device.destroy_buffer(vb);
                self.device.destroy_buffer(ib);
                return Err(vec![Diagnostic::new(
                    "RV0235",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("cannot replace in-flight mesh '{mesh_id}': {error:?}"),
                )]);
            }
        }
        if let Some(old) = self.meshes.insert(mesh_id, mesh) {
            self.device.destroy_buffer(old.vertex_buffer);
            self.device.destroy_buffer(old.index_buffer);
        }
        Ok(UploadReceipt::new(revision))
    }

    fn upload_texture(&mut self, upload: TextureUpload) -> Result<UploadReceipt, Vec<Diagnostic>> {
        use crate::device_impl::reload::{
            SampledTextureAddressMode, SampledTextureColorSpace, SampledTextureDescriptor,
            SampledTextureFilter, SampledTextureSamplerDescriptor,
        };
        use crate::device_impl::texture::FALLBACK_MATERIAL_TEXTURE_ID;

        let texture_id = upload.texture_id.id.clone();
        if texture_id == FALLBACK_MATERIAL_TEXTURE_ID {
            return Err(vec![Diagnostic::new(
                "RV0236",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "the renderer fallback texture ID is reserved",
            )]);
        }
        if let Some(existing) = self.texture_uploads.get(&texture_id) {
            if existing.content_hash == upload.content_hash {
                return Ok(UploadReceipt::new(existing.revision));
            }
        }
        let revision = self
            .texture_uploads
            .get(&texture_id)
            .map_or(1, |existing| existing.revision.saturating_add(1));
        let mut mip_bytes = Vec::new();
        for mip in &upload.mip_levels {
            mip_bytes.extend_from_slice(&mip.bytes);
        }
        let map_filter = |filter| match filter {
            SamplerFilter::Nearest => SampledTextureFilter::Nearest,
            SamplerFilter::Linear => SampledTextureFilter::Linear,
        };
        let map_address = |address| match address {
            SamplerAddressMode::Repeat => SampledTextureAddressMode::Repeat,
            SamplerAddressMode::ClampToEdge => SampledTextureAddressMode::ClampToEdge,
            SamplerAddressMode::MirroredRepeat => SampledTextureAddressMode::MirroredRepeat,
        };
        let descriptor = SampledTextureDescriptor::rgba8(
            upload.width,
            upload.height,
            u8::try_from(upload.mip_levels.len()).map_err(|_| {
                vec![Diagnostic::new(
                    "RV0237",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    "texture mip count exceeds the Vulkan upload contract",
                )]
            })?,
            &mip_bytes,
            match upload.color_space {
                engine_renderer::ColorSpace::Linear => SampledTextureColorSpace::Linear,
                engine_renderer::ColorSpace::Srgb => SampledTextureColorSpace::Srgb,
            },
            SampledTextureSamplerDescriptor {
                min_filter: map_filter(upload.sampler.min_filter),
                mag_filter: map_filter(upload.sampler.mag_filter),
                mip_filter: map_filter(upload.sampler.mip_filter),
                address_u: map_address(upload.sampler.address_u),
                address_v: map_address(upload.sampler.address_v),
                address_w: map_address(upload.sampler.address_w),
            },
        );
        let new_texture = self
            .device
            .create_sampled_texture_resource(descriptor)
            .map_err(|error| {
                vec![Diagnostic::new(
                    "RV0238",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("texture upload failed for '{texture_id}': {error:?}"),
                )]
            })?;

        if self.device.textures.contains_key(&texture_id) {
            if let Err(error) = self.device.wait_idle_checked() {
                self.device.destroy_gpu_texture(new_texture);
                return Err(vec![Diagnostic::new(
                    "RV0239",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("cannot replace in-flight texture '{texture_id}': {error:?}"),
                )]);
            }
        }
        let descriptor_sets: Vec<vk::DescriptorSet> = self
            .material_cache
            .values()
            .filter(|entry| entry.bound_texture_id == texture_id)
            .map(|entry| entry.desc_set)
            .chain(
                self.skinned_desc_cache
                    .values()
                    .filter(|entry| entry.bound_texture_id == texture_id)
                    .map(|entry| entry.desc_set),
            )
            .collect();
        let mut old_texture = self.device.textures.insert(texture_id.clone(), new_texture);
        let mut rebind_diagnostics = Vec::new();
        for descriptor_set in descriptor_sets.iter().copied() {
            match self
                .device
                .bind_material_texture(&texture_id, descriptor_set)
            {
                Ok(true) => {}
                Ok(false) => rebind_diagnostics.push(Diagnostic::new(
                    "RV0276",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!(
                        "replacement texture '{texture_id}' disappeared before descriptor update"
                    ),
                )),
                Err(error) => rebind_diagnostics.push(Diagnostic::new(
                    "RV0277",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("failed to rebind replacement texture '{texture_id}': {error:?}"),
                )),
            }
        }
        if !rebind_diagnostics.is_empty() {
            let failed_texture = self.device.textures.remove(&texture_id);
            if let Some(previous_texture) = old_texture.take() {
                self.device
                    .textures
                    .insert(texture_id.clone(), previous_texture);
                for descriptor_set in descriptor_sets {
                    match self
                        .device
                        .bind_material_texture(&texture_id, descriptor_set)
                    {
                        Ok(true) => {}
                        Ok(false) => rebind_diagnostics.push(Diagnostic::new(
                            "RV0278",
                            DiagnosticSeverity::Fatal,
                            "scene_renderer",
                            format!(
                                "failed to restore texture '{texture_id}' after replacement rollback"
                            ),
                        )),
                        Err(error) => rebind_diagnostics.push(Diagnostic::new(
                            "RV0279",
                            DiagnosticSeverity::Fatal,
                            "scene_renderer",
                            format!(
                                "failed to restore texture '{texture_id}' descriptor after replacement rollback: {error:?}"
                            ),
                        )),
                    }
                }
            }
            if let Some(failed_texture) = failed_texture {
                self.device.destroy_gpu_texture(failed_texture);
            }
            return Err(rebind_diagnostics);
        }
        if let Some(old_texture) = old_texture {
            self.device.destroy_gpu_texture(old_texture);
        }
        self.texture_uploads.insert(
            texture_id,
            UploadedResourceState {
                content_hash: upload.content_hash,
                revision,
            },
        );
        Ok(UploadReceipt::new(revision))
    }

    fn upload_material(
        &mut self,
        upload: MaterialUpload,
    ) -> Result<UploadReceipt, Vec<Diagnostic>> {
        if let Some(texture) = &upload.base_color_texture {
            if !self.device.textures.contains_key(&texture.id) {
                return Err(vec![Diagnostic::new(
                    "RV0240",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!(
                        "material '{}' references texture '{}' before a successful upload",
                        upload.material_id.id, texture.id
                    ),
                )]);
            }
        }
        let material_id = upload.material_id.id.clone();
        if let Some(existing) = self.uploaded_materials.get(&material_id) {
            if existing.content_hash == upload.content_hash {
                return Ok(UploadReceipt::new(existing.revision));
            }
        }
        let revision = self
            .uploaded_materials
            .get(&material_id)
            .map_or(1, |existing| existing.revision.saturating_add(1));
        self.uploaded_materials.insert(
            material_id,
            UploadedMaterialState {
                binding: uploaded_material_binding(&upload),
                content_hash: upload.content_hash,
                revision,
            },
        );
        Ok(UploadReceipt::new(revision))
    }

    fn remove_resource(&mut self, removal: ResourceRemoval) -> Result<(), Vec<Diagnostic>> {
        let resource_id = removal.resource_id.id;
        match removal.kind {
            ResourceKind::Mesh => {
                if self.meshes.contains_key(&resource_id) {
                    self.device.wait_idle_checked().map_err(|error| {
                        vec![Diagnostic::new(
                            "RV0241",
                            DiagnosticSeverity::Error,
                            "scene_renderer",
                            format!("cannot remove in-flight mesh '{resource_id}': {error:?}"),
                        )]
                    })?;
                }
                if let Some(mesh) = self.meshes.remove(&resource_id) {
                    self.device.destroy_buffer(mesh.vertex_buffer);
                    self.device.destroy_buffer(mesh.index_buffer);
                }
            }
            ResourceKind::Texture => {
                use crate::device_impl::texture::FALLBACK_MATERIAL_TEXTURE_ID;
                if resource_id == FALLBACK_MATERIAL_TEXTURE_ID {
                    return Err(vec![Diagnostic::new(
                        "RV0242",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        "the renderer fallback texture cannot be removed",
                    )]);
                }
                if let Some(dependent) = self.uploaded_materials.values().find(|material| {
                    material
                        .binding
                        .textures
                        .iter()
                        .any(|slot| slot.texture.id == resource_id)
                }) {
                    return Err(vec![Diagnostic::new(
                        "RV0270",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        format!(
                            "texture '{resource_id}' is still referenced by material '{}'",
                            dependent.binding.material_id.id
                        ),
                    )]);
                }
                self.device.wait_idle_checked().map_err(|error| {
                    vec![Diagnostic::new(
                        "RV0243",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        format!("cannot remove in-flight texture '{resource_id}': {error:?}"),
                    )]
                })?;
                let material_keys: Vec<String> = self
                    .material_cache
                    .iter()
                    .filter(|(_, entry)| entry.bound_texture_id == resource_id)
                    .map(|(key, _)| key.clone())
                    .collect();
                let skinned_keys: Vec<String> = self
                    .skinned_desc_cache
                    .iter()
                    .filter(|(_, entry)| entry.bound_texture_id == resource_id)
                    .map(|(key, _)| key.clone())
                    .collect();
                for key in &material_keys {
                    let descriptor_set = self.material_cache[key].desc_set;
                    let bound = self
                        .device
                        .bind_material_texture(FALLBACK_MATERIAL_TEXTURE_ID, descriptor_set)
                        .map_err(|error| {
                            vec![Diagnostic::new(
                                "RV0280",
                                DiagnosticSeverity::Error,
                                "scene_renderer",
                                format!(
                                    "failed to detach texture '{resource_id}' from material '{key}': {error:?}"
                                ),
                            )]
                        })?;
                    if !bound {
                        return Err(vec![Diagnostic::new(
                            "RV0281",
                            DiagnosticSeverity::Fatal,
                            "scene_renderer",
                            "fallback texture disappeared during resource removal",
                        )]);
                    }
                }
                for key in &skinned_keys {
                    let descriptor_set = self.skinned_desc_cache[key].desc_set;
                    let bound = self
                        .device
                        .bind_material_texture(FALLBACK_MATERIAL_TEXTURE_ID, descriptor_set)
                        .map_err(|error| {
                            vec![Diagnostic::new(
                                "RV0282",
                                DiagnosticSeverity::Error,
                                "scene_renderer",
                                format!(
                                    "failed to detach texture '{resource_id}' from skinned material '{key}': {error:?}"
                                ),
                            )]
                        })?;
                    if !bound {
                        return Err(vec![Diagnostic::new(
                            "RV0283",
                            DiagnosticSeverity::Fatal,
                            "scene_renderer",
                            "fallback texture disappeared during skinned resource removal",
                        )]);
                    }
                }
                for key in material_keys {
                    if let Some(entry) = self.material_cache.get_mut(&key) {
                        entry.bound_texture_id = FALLBACK_MATERIAL_TEXTURE_ID.to_owned();
                    }
                }
                for key in skinned_keys {
                    if let Some(entry) = self.skinned_desc_cache.get_mut(&key) {
                        entry.bound_texture_id = FALLBACK_MATERIAL_TEXTURE_ID.to_owned();
                    }
                }
                if let Some(texture) = self.device.textures.remove(&resource_id) {
                    self.device.destroy_gpu_texture(texture);
                }
                self.texture_uploads.remove(&resource_id);
            }
            ResourceKind::Material => {
                self.evict_material_by_id(&resource_id)?;
                self.uploaded_materials.remove(&resource_id);
            }
        }
        Ok(())
    }

    fn abort_frame(&mut self) -> Result<(), Vec<Diagnostic>> {
        if self.cur_enc.is_none() && self.cur_sc.is_none() && self.cur_ii.is_none() {
            return Ok(());
        }
        self.cur_enc.take();
        self.cur_sc.take();
        self.cur_ii.take();
        self.recover_failed_device_frame()
    }

    fn resize(&mut self, width: u32, height: u32) -> Result<(), Vec<Diagnostic>> {
        if width == 0 || height == 0 {
            return Err(vec![Diagnostic::new(
                "RV0245",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "Vulkan surface dimensions must be non-zero",
            )]);
        }
        if self.cur_enc.is_some() {
            return Err(vec![Diagnostic::new(
                "RV0246",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "cannot resize while a frame is being recorded",
            )]);
        }
        self.device.wait_idle_checked().map_err(|error| {
            vec![Diagnostic::new(
                "RV0247",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("failed to wait for Vulkan resize: {error:?}"),
            )]
        })?;
        self.device.destroy_scene_framebuffers(&self.framebuffers);
        self.framebuffers.clear();
        self.width = width;
        self.height = height;
        self.device.resize(width, height);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_renderer::hash_vertex_layout;

    struct MockDevice {
        next_index: u32,
        create_calls: u32,
        destroyed: Vec<PipelineHandle>,
        created_descs: Vec<PipelineDescriptor>,
    }

    impl MockDevice {
        fn new() -> Self {
            Self {
                next_index: 1,
                create_calls: 0,
                destroyed: Vec::new(),
                created_descs: Vec::new(),
            }
        }
    }

    impl Device for MockDevice {
        fn adapter_info(&self) -> &render_core::AdapterInfo {
            unimplemented!("not needed in tests")
        }

        fn create_pipeline(
            &mut self,
            desc: &PipelineDescriptor,
        ) -> Result<PipelineHandle, render_core::RhiError> {
            self.create_calls += 1;
            self.created_descs.push(desc.clone());
            let handle = PipelineHandle::new(self.next_index, 1);
            self.next_index += 1;
            Ok(handle)
        }

        fn destroy_pipeline(&mut self, handle: PipelineHandle) {
            self.destroyed.push(handle);
        }
    }

    #[test]
    fn shadow_index_binding_preserves_uploaded_index_width() {
        assert_eq!(vulkan_index_type(IndexFormat::U16), vk::IndexType::UINT16);
        assert_eq!(vulkan_index_type(IndexFormat::U32), vk::IndexType::UINT32);
    }

    #[test]
    fn fallback_extraction_stats_count_static_and_skinned_drawables() {
        let mut input = RenderFrameInput::empty(1);
        input.drawables.push(RenderableItem {
            entity: None,
            mesh: AssetId::new("mesh_static"),
            material: AssetId::new("material_static"),
            world_transform: [0.0; 16],
            bounds: engine_renderer::AxisAlignedBox::UNIT,
            render_layer: "default".to_string(),
            cast_shadows: true,
            sort_key: 0,
        });
        input.skinned_items.push(SkinnedItem {
            entity: None,
            mesh: AssetId::new("mesh_skinned"),
            material: AssetId::new("material_skinned"),
            skeleton: AssetId::new("skeleton"),
            bone_palette: Vec::new(),
            bone_palette_layout: engine_renderer::BonePaletteLayout::Full4x4 { count: 0 },
            world_transform: [0.0; 16],
            bounds: engine_renderer::AxisAlignedBox::UNIT,
            render_layer: "default".to_string(),
            cast_shadows: true,
            sort_key: 1,
        });

        assert_eq!(extraction_stats(&input).visible_drawables, 2);
    }

    #[test]
    fn structured_extraction_stats_are_preserved() {
        let mut input = RenderFrameInput::empty(2);
        input.extraction_stats = Some(engine_renderer::ExtractionStats {
            visible_drawables: 3,
            culled_drawables: 5,
            visible_lights: 2,
            culled_lights: 7,
        });
        let mut frame = FrameStats::default();

        apply_extraction_stats(&mut frame, &input);

        assert_eq!(frame.visible_drawables, 3);
        assert_eq!(frame.culled_drawables, 5);
        assert_eq!(frame.visible_lights, 2);
        assert_eq!(frame.culled_lights, 7);
    }

    #[test]
    fn scene_forward_pipeline_resolution_uses_explicit_render_pass() {
        let resolver = MaterialResolver::new(4);
        let pll = PipelineLayoutHandle::new(3, 1);
        let rp = RenderPassHandle::new(7, 2);
        let material = fallback_material_binding(&AssetId::new("mat_default"));

        let (key, desc) = resolver.resolve(
            &material,
            &scene_forward_pipeline_context(pll, rp, 1),
            PipelineVariantKey::NONE,
        );

        assert_eq!(key.shader_asset_id, SCENE_FORWARD_PIPELINE_ID);
        assert_eq!(
            key.vertex_layout_hash,
            hash_vertex_layout(&desc.vertex_layout)
        );
        assert_eq!(key.variant_key, PipelineVariantKey::NONE);
        assert_eq!(desc.pipeline_layout, Some(pll));
        assert_eq!(desc.render_pass, Some(rp));
    }

    #[test]
    fn scene_forward_pipeline_cache_hit_reuses_handle() {
        let mut resolver = MaterialResolver::new(4);
        let mut device = MockDevice::new();
        let pll = PipelineLayoutHandle::new(1, 1);
        let rp = RenderPassHandle::new(2, 1);
        let material = fallback_material_binding(&AssetId::new("mat_shared"));

        let first = get_or_create_scene_forward_pipeline(
            &mut resolver,
            &mut device,
            &material,
            pll,
            rp,
            PipelineVariantKey::NONE,
            1,
        )
        .expect("first pipeline create should succeed");
        let second = get_or_create_scene_forward_pipeline(
            &mut resolver,
            &mut device,
            &material,
            pll,
            rp,
            PipelineVariantKey::NONE,
            1,
        )
        .expect("cache hit should succeed");

        assert_eq!(first, second);
        assert_eq!(device.create_calls, 1);
        assert!(device.destroyed.is_empty());
        assert_eq!(device.created_descs.len(), 1);
        assert_eq!(device.created_descs[0].render_pass, Some(rp));
        assert_eq!(resolver.library().len(), 1);
    }

    #[test]
    fn scene_forward_pipeline_cache_eviction_destroys_old_handle() {
        let mut resolver = MaterialResolver::new(1);
        let mut device = MockDevice::new();
        let pll = PipelineLayoutHandle::new(1, 1);
        let rp = RenderPassHandle::new(2, 1);
        let material = fallback_material_binding(&AssetId::new("mat_shared"));

        let first = get_or_create_scene_forward_pipeline(
            &mut resolver,
            &mut device,
            &material,
            pll,
            rp,
            PipelineVariantKey::NONE,
            1,
        )
        .expect("first pipeline create should succeed");
        let second = get_or_create_scene_forward_pipeline(
            &mut resolver,
            &mut device,
            &material,
            pll,
            rp,
            PipelineVariantKey::SKINNED,
            1,
        )
        .expect("second pipeline create should succeed");

        assert_ne!(first, second);
        assert_eq!(device.create_calls, 2);
        assert_eq!(device.destroyed, vec![first]);
        assert_eq!(resolver.library().len(), 1);
    }
}
