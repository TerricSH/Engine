//! Vulkan implementation of [`BackendRenderer`].
//!
//! Consumes [`RenderFrameInput`] and renders each drawable through a
//! forward-shaded pipeline with lighting.
//!
//! # First-pass limitations
//!
//! * Meshes that are not yet cached fall back to a hard-coded coloured quad.
//! * The framebuffer attachment is a placeholder (null image-view) so the
//!   render pass begin is structurally present but may not produce visible
//!   output on all drivers.  The frame lifecycle (acquire �?record �?submit
//!   �?present) is otherwise complete.
//! * Material handling currently covers pipeline variants plus basic raster and
//!   blend state, but textures and uniform bindings are not yet consumed.
//! * No texture loading, skeletal animation, or multi-pass optimisation.

use std::collections::{BTreeMap, HashMap};

use ash::vk;
use glam::Mat4;
use glam::Vec4 as GlamVec4;

use engine_renderer::{
    render_graph, AssetId, AxisAlignedBox, BackendRenderer, Diagnostic, DiagnosticSeverity,
    FrameStats, LightItem, LightKind, MaterialBinding, MaterialPipelineContext, MaterialResolver,
    ParamBlock, PassRegistry, RenderFrameInput, RenderableItem, SkinnedItem, Transparency,
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
use crate::shaders_embedded::{FORWARD_FRAG_SPV, FORWARD_VERT_SPV, SKINNED_VERT_SPV};

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
}

// ============================================================================
// Fallback mesh data  �? a coloured quad
// ============================================================================

/// Single vertex for the fallback mesh.
///
/// Layout: position (float32x3) + color (float32x4) + padding (float32)
/// Total stride = 32 bytes, matching the shadow pipeline's vertex-input
/// stride so the same vertex buffer can be used for both forward and
/// shadow rendering.
#[repr(C)]
struct FallbackVertex {
    position: [f32; 3],
    color: [f32; 4],
    _pad: f32,
}

const FALLBACK_VERTICES: [FallbackVertex; 4] = [
    FallbackVertex {
        position: [-0.5, -0.5, 0.0],
        color: [1.0, 0.0, 0.0, 1.0],
        _pad: 0.0,
    },
    FallbackVertex {
        position: [0.5, -0.5, 0.0],
        color: [0.0, 1.0, 0.0, 1.0],
        _pad: 0.0,
    },
    FallbackVertex {
        position: [0.5, 0.5, 0.0],
        color: [0.0, 0.0, 1.0, 1.0],
        _pad: 0.0,
    },
    FallbackVertex {
        position: [-0.5, 0.5, 0.0],
        color: [1.0, 1.0, 0.0, 1.0],
        _pad: 0.0,
    },
];

fn fallback_vertex_bytes() -> Vec<u8> {
    let mut bytes = Vec::with_capacity(FALLBACK_VERTICES.len() * 32);
    for v in &FALLBACK_VERTICES {
        for f in v
            .position
            .iter()
            .copied()
            .chain(v.color.iter().copied())
            .chain(std::iter::once(v._pad))
        {
            bytes.extend_from_slice(&f.to_ne_bytes());
        }
    }
    bytes
}

const FALLBACK_INDICES: [u16; 6] = [0, 1, 2, 2, 3, 0];

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
        // 32-byte stride: float32x3 + float32x4 + 4-byte padding.
        // The padding ensures the stride matches the shadow pipeline's
        // vertex-input stride so the same buffers can be used for both.
        stride_bytes: 32,
        attributes: vec![
            VertexAttribute {
                semantic: "position".into(),
                format: "float32x3".into(),
                offset_bytes: 0,
            },
            VertexAttribute {
                semantic: "color".into(),
                format: "float32x4".into(),
                offset_bytes: 12,
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
    buffer: vk::Buffer,
}

const MAX_MATERIALS: usize = 256;

/// Cache entry for a bone palette descriptor set + buffer.
#[allow(dead_code)]
struct BonePaletteCacheEntry {
    desc_set: vk::DescriptorSet,
    bone_buffer: vk::Buffer,
}

/// Cached bone UBO buffer (handle for writes + raw VkBuffer for descriptor binding).
struct CachedBoneBuffer {
    handle: BufferHandle,
    vk_buffer: vk::Buffer,
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
// Frustum-culling helpers
// ============================================================================

/// Test whether an [`AxisAlignedBox`] transformed by `world_transform`
/// intersects the given view-frustum planes.
///
/// Each frustum plane is `(nx, ny, nz, d)` where the half-space
/// `nx·x + ny·y + nz·z + d �?0` is considered "inside".
///
/// Returns `true` if the AABB is at least partially visible.
pub(crate) fn is_aabb_visible(
    bounds: &AxisAlignedBox,
    world_transform: &[f32; 16],
    frustum_planes: &[[f32; 4]; 6],
) -> bool {
    // 8 corners of the AABB in local space.
    let corners = [
        [bounds.min[0], bounds.min[1], bounds.min[2], 1.0],
        [bounds.max[0], bounds.min[1], bounds.min[2], 1.0],
        [bounds.min[0], bounds.max[1], bounds.min[2], 1.0],
        [bounds.max[0], bounds.max[1], bounds.min[2], 1.0],
        [bounds.min[0], bounds.min[1], bounds.max[2], 1.0],
        [bounds.max[0], bounds.min[1], bounds.max[2], 1.0],
        [bounds.min[0], bounds.max[1], bounds.max[2], 1.0],
        [bounds.max[0], bounds.max[1], bounds.max[2], 1.0],
    ];

    // Build transform matrix once.
    let m = Mat4::from_cols_array(world_transform);

    for plane in frustum_planes {
        let (nx, ny, nz, d) = (plane[0], plane[1], plane[2], plane[3]);
        let mut all_outside = true;

        for corner in &corners {
            let world_corner = m * GlamVec4::new(corner[0], corner[1], corner[2], corner[3]);
            let dist = nx * world_corner.x + ny * world_corner.y + nz * world_corner.z + d;
            if dist >= 0.0 {
                all_outside = false;
                break;
            }
        }

        if all_outside {
            return false; // Entire AABB is on the outside of this plane.
        }
    }

    true
}

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
            material_cache: HashMap::new(),
            material_cache_order: Vec::new(),
            bone_palette_buffers: HashMap::new(),
            bone_palette_buffers_order: Vec::new(),
            skinned_desc_cache: HashMap::new(),
            skinned_desc_cache_order: Vec::new(),
            rp: None,
            pll: None,
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

    /// Create the render pass and pipeline layout used by scene-forward draws.
    ///
    /// This is called once from [`begin_frame_impl`] when
    /// `self.initialized` is `false`.
    fn init_once(&mut self) -> Result<(), Vec<Diagnostic>> {
        if self.initialized {
            return Ok(());
        }

        // Point the device at the forward-shader SPIR-V blobs so that
        // `Device::create_pipeline` can find them.
        self.device
            .set_mvp_shaders(FORWARD_VERT_SPV, FORWARD_FRAG_SPV);
        // Register the skinned vertex shader for skinned-mesh pipelines.
        if !SKINNED_VERT_SPV.is_empty() {
            self.device.set_skinned_vertex_shader(SKINNED_VERT_SPV);
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

        self.initialized = true;
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
            .unwrap_or_else(|| fallback_material_binding(material_id))
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
            // Update the buffer contents with the latest bone data.
            let _ = self.device.write_buffer(cached.handle, &ubo_data, 0);
            return Ok(cached.vk_buffer);
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
        self.device.write_buffer(buf, &ubo_data, 0).map_err(|e| {
            vec![Diagnostic::new(
                "RV0219",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("write_buffer(bone UBO): {e:?}"),
            )]
        })?;

        // Resolve raw Vulkan buffer handle
        let vk_buf = self
            .device
            .buffers
            .get(buf.index, buf.generation)
            .map(|e| e.buffer)
            .unwrap_or(vk::Buffer::null());
        if vk_buf == vk::Buffer::null() {
            return Err(vec![Diagnostic::new(
                "RV0220",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "bone UBO buffer handle invalid",
            )]);
        }

        // Evict oldest if at capacity
        if self.bone_palette_buffers.len() >= MAX_BONE_PALETTES {
            if let Some(oldest_key) = self.bone_palette_buffers_order.first().cloned() {
                self.bone_palette_buffers_order.remove(0);
                self.bone_palette_buffers.remove(&oldest_key);
            }
        }

        self.bone_palette_buffers.insert(
            skeleton_id.to_string(),
            CachedBoneBuffer {
                handle: buf,
                vk_buffer: vk_buf,
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

        // Evict oldest if at capacity
        if self.skinned_desc_cache.len() >= MAX_BONE_PALETTES {
            if let Some(oldest_key) = self.skinned_desc_cache_order.first().cloned() {
                self.skinned_desc_cache_order.remove(0);
                self.skinned_desc_cache.remove(&oldest_key);
            }
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
        let read_f32 = |offset: usize| -> f32 {
            if offset + 4 <= bytes.len() {
                f32::from_ne_bytes(bytes[offset..offset + 4].try_into().unwrap())
            } else {
                0.0
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
            metallic: read_f32(16),
            roughness: read_f32(20),
            ao: read_f32(24),
            _padding: [0.0],
        }
    }

    /// Look up or create a material descriptor set + buffer for the given
    /// material.  Uses a LRU eviction policy capped at [`MAX_MATERIALS`].
    fn get_or_create_material_desc_set(
        &mut self,
        material_id: &str,
        ubo_data: &[u8],
    ) -> Result<(vk::DescriptorSet, vk::Buffer), Vec<Diagnostic>> {
        // Check cache first (and move to front for LRU)
        if let Some(entry) = self.material_cache.get(material_id) {
            // Promote in LRU order (simple move-to-front)
            if let Some(pos) = self
                .material_cache_order
                .iter()
                .position(|k| k == material_id)
            {
                self.material_cache_order.remove(pos);
                self.material_cache_order.push(material_id.to_string());
            }
            return Ok((entry.desc_set, entry.buffer));
        }

        // Evict oldest if at capacity
        if self.material_cache.len() >= MAX_MATERIALS {
            self.evict_oldest_material();
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
        self.device.write_buffer(buf, ubo_data, 0).map_err(|e| {
            vec![Diagnostic::new(
                "RV0215",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("write_buffer(material UBO): {e:?}"),
            )]
        })?;

        // Resolve raw Vulkan buffer handle for the descriptor set
        let vk_buf = self
            .device
            .buffers
            .get(buf.index, buf.generation)
            .map(|e| e.buffer)
            .unwrap_or(vk::Buffer::null());
        if vk_buf == vk::Buffer::null() {
            return Err(vec![Diagnostic::new(
                "RV0216",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "material UBO buffer handle invalid",
            )]);
        }

        // Allocate and update descriptor set via the device
        let desc_set = self
            .device
            .allocate_material_descriptor_set(vk_buf, 32)
            .map_err(|e| {
                vec![Diagnostic::new(
                    "RV0217",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("allocate_material_descriptor_set: {e:?}"),
                )]
            })?;

        let entry = MaterialCacheEntry {
            desc_set,
            buffer: vk_buf,
        };
        self.material_cache.insert(material_id.to_string(), entry);
        self.material_cache_order.push(material_id.to_string());

        Ok((desc_set, vk_buf))
    }

    /// Evict the oldest entry from the material cache (LRU).
    fn evict_oldest_material(&mut self) {
        if let Some(oldest_key) = self.material_cache_order.first().cloned() {
            self.material_cache_order.remove(0);
            self.material_cache.remove(&oldest_key);
            // NOTE: Vulkan buffers and descriptor sets are freed when the
            // descriptor pool is destroyed (no per-allocation free needed).
        }
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
        self.device
            .write_buffer(vb, &vertex_bytes, 0)
            .map_err(|e| {
                vec![Diagnostic::new(
                    "RV0204",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("write_buffer(vertices): {e:?}"),
                )]
            })?;

        // --- Index buffer ---
        let ib_desc = BufferDescriptor {
            size_bytes: index_bytes.len() as u64,
            usage_flags: render_core::BufferUsage(0),
            memory_hint: MemoryHint::CpuToGpu,
            debug_label: Some(format!("mesh-{mesh_id}-indices")),
        };
        let ib = self.device.create_buffer(&ib_desc).map_err(|e| {
            vec![Diagnostic::new(
                "RV0205",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("create_buffer(indices): {e:?}"),
            )]
        })?;
        self.device.write_buffer(ib, &index_bytes, 0).map_err(|e| {
            vec![Diagnostic::new(
                "RV0206",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("write_buffer(indices): {e:?}"),
            )]
        })?;

        let index_count = (index_bytes.len() / 2) as u32; // u16 indices
        let mesh = GpuMesh {
            vertex_buffer: vb,
            index_buffer: ib,
            index_count,
            index_format: IndexFormat::U16,
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
        msaa_samples: vk::SampleCountFlags,
    ) -> Result<(SwapchainHandle, u32, Box<dyn CommandEncoder>), Vec<Diagnostic>> {
        // Apply the requested MSAA sample count to the device before any
        // resource creation takes place (ensure_sc �?ensure_hdr_resources).
        self.device.hdr_msaa_samples = msaa_samples;
        self.init_once()?;

        // The device's descriptor infrastructure expects per-frame UBO data
        // to be written before `begin_frame`.
        self.device.write_default_ubo();

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

        let (ii, encoder) = self.device.begin_frame(sc_h).map_err(|e| {
            vec![Diagnostic::new(
                "RV0208",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("begin_frame: {e:?}"),
            )]
        })?;

        self.cur_fb_index = ii;

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
        if hdr_rp == vk::RenderPass::null() || hdr_pl == vk::Pipeline::null() {
            return Ok(());
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

        // Draw calls
        for drawable in &input.drawables {
            let mesh_id = &drawable.mesh.id;
            let mesh = match self.meshes.get(mesh_id).cloned() {
                Some(m) => m,
                None => {
                    tracing::trace!(
                        target: "scene_renderer",
                        mesh = mesh_id,
                        "skipping un-cached mesh in HDR forward pass"
                    );
                    continue;
                }
            };

            let material_id = &drawable.material.id;
            let material = self.material_binding_for_drawable(input, &drawable.material);
            let material_ubo = Self::parse_material_ubo(&material.uniforms.bytes);
            let ubo_bytes: &[u8] = unsafe {
                std::slice::from_raw_parts(
                    &material_ubo as *const _ as *const u8,
                    std::mem::size_of::<MaterialUBO>(),
                )
            };
            let (mat_desc_set, _mat_buf) = self
                .get_or_create_material_desc_set(material_id, ubo_bytes)
                .unwrap_or_else(|diags| {
                    for d in &diags {
                        tracing::warn!(target: "scene_renderer", code = d.code, message = d.message);
                    }
                    (vk::DescriptorSet::null(), vk::Buffer::null())
                });
            if mat_desc_set != vk::DescriptorSet::null() {
                for tex_slot in &material.textures {
                    let tex_id = &tex_slot.texture.id;
                    if self.device.textures.contains_key(tex_id) {
                        let _ = self.device.bind_material_texture(tex_id, mat_desc_set);
                        break;
                    }
                }
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
            }

            // Push constants: world transform (128 B)
            let world = &drawable.world_transform;
            let mut pc_bytes = Vec::with_capacity(128);
            for v in world {
                pc_bytes.extend_from_slice(&v.to_ne_bytes());
            }
            pc_bytes.resize(128, 0u8);
            unsafe {
                d.cmd_push_constants(cmd, hdr_pll, vk::ShaderStageFlags::VERTEX, 0, &pc_bytes);
            }

            // Bind vertex/index buffers
            let vk_vb = self
                .device
                .buffers
                .get(mesh.vertex_buffer.index, mesh.vertex_buffer.generation)
                .map(|e| e.buffer)
                .unwrap_or(vk::Buffer::null());
            let vk_ib = self
                .device
                .buffers
                .get(mesh.index_buffer.index, mesh.index_buffer.generation)
                .map(|e| e.buffer)
                .unwrap_or(vk::Buffer::null());
            if vk_vb != vk::Buffer::null() {
                let vbs = [vk_vb];
                let offsets = [0u64];
                let idx_ty = match mesh.index_format {
                    IndexFormat::U16 => vk::IndexType::UINT16,
                    IndexFormat::U32 => vk::IndexType::UINT32,
                };
                unsafe {
                    d.cmd_bind_vertex_buffers(cmd, 0, &vbs, &offsets);
                    d.cmd_bind_index_buffer(cmd, vk_ib, 0, idx_ty);
                    d.cmd_draw_indexed(cmd, mesh.index_count, 1, 0, 0, 0);
                }
            }

            stats.draw_calls += 1;
            stats.triangles += mesh.index_count as u64 / 3;
        }

        // Skinned items
        for skinned in &input.skinned_items {
            let mesh_id = &skinned.mesh.id;
            let mesh = match self.meshes.get(mesh_id).cloned() {
                Some(m) => m,
                None => {
                    tracing::trace!(
                        target: "scene_renderer",
                        mesh = mesh_id,
                        "skipping un-cached skinned mesh in HDR forward pass"
                    );
                    continue;
                }
            };

            let material_id = &skinned.material.id;
            let material = self.material_binding_for_drawable(input, &skinned.material);
            let material_ubo = Self::parse_material_ubo(&material.uniforms.bytes);
            let ubo_bytes: &[u8] = unsafe {
                std::slice::from_raw_parts(
                    &material_ubo as *const _ as *const u8,
                    std::mem::size_of::<MaterialUBO>(),
                )
            };
            let (mat_desc_set, mat_buf) = self
                .get_or_create_material_desc_set(material_id, ubo_bytes)
                .unwrap_or_else(|diags| {
                    for d in &diags {
                        tracing::warn!(target: "scene_renderer", code = d.code, message = d.message);
                    }
                    (vk::DescriptorSet::null(), vk::Buffer::null())
                });
            if mat_desc_set == vk::DescriptorSet::null() {
                continue;
            }

            let skeleton_id = &skinned.skeleton.id;
            let bone_buf = match self.get_or_create_bone_buffer(skeleton_id, &skinned.bone_palette)
            {
                Ok(b) => b,
                Err(diags) => {
                    for d in &diags {
                        tracing::warn!(target: "scene_renderer", code = d.code, message = d.message);
                    }
                    continue;
                }
            };

            let skinned_desc_set = self
                .get_or_create_skinned_desc_set(
                    material_id,
                    skeleton_id,
                    mat_desc_set,
                    mat_buf,
                    bone_buf,
                )
                .unwrap_or_else(|diags| {
                    for d in &diags {
                        tracing::warn!(target: "scene_renderer", code = d.code, message = d.message);
                    }
                    vk::DescriptorSet::null()
                });
            if skinned_desc_set == vk::DescriptorSet::null() {
                continue;
            }

            if skinned_desc_set != vk::DescriptorSet::null() {
                for tex_slot in &material.textures {
                    let tex_id = &tex_slot.texture.id;
                    if self.device.textures.contains_key(tex_id) {
                        let _ = self.device.bind_material_texture(tex_id, skinned_desc_set);
                        break;
                    }
                }
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
            }

            let pc_bytes = vec![0u8; 128];
            unsafe {
                d.cmd_push_constants(cmd, hdr_pll, vk::ShaderStageFlags::VERTEX, 0, &pc_bytes);
            }

            // Bind vertex/index buffers
            let vk_vb = self
                .device
                .buffers
                .get(mesh.vertex_buffer.index, mesh.vertex_buffer.generation)
                .map(|e| e.buffer)
                .unwrap_or(vk::Buffer::null());
            let vk_ib = self
                .device
                .buffers
                .get(mesh.index_buffer.index, mesh.index_buffer.generation)
                .map(|e| e.buffer)
                .unwrap_or(vk::Buffer::null());
            if vk_vb != vk::Buffer::null() {
                let vbs = [vk_vb];
                let offsets = [0u64];
                let idx_ty = match mesh.index_format {
                    IndexFormat::U16 => vk::IndexType::UINT16,
                    IndexFormat::U32 => vk::IndexType::UINT32,
                };
                unsafe {
                    d.cmd_bind_vertex_buffers(cmd, 0, &vbs, &offsets);
                    d.cmd_bind_index_buffer(cmd, vk_ib, 0, idx_ty);
                    d.cmd_draw_indexed(cmd, mesh.index_count, 1, 0, 0, 0);
                }
            }

            stats.draw_calls += 1;
            stats.triangles += mesh.index_count as u64 / 3;
        }

        // Frustum cull + indirect draw (Phase 5.1)
        if let Some(planes) = input.views.first().and_then(|v| v.frustum) {
            let fp: [[f32; 4]; 6] = planes;
            let mut visible_indices: Vec<usize> = Vec::new();
            for (i, drawable) in input.drawables.iter().enumerate() {
                if !self.meshes.contains_key(&drawable.mesh.id) {
                    continue;
                }
                if is_aabb_visible(&drawable.bounds, &drawable.world_transform, &fp) {
                    visible_indices.push(i);
                }
            }

            if !visible_indices.is_empty() {
                let mut indirect_bytes: Vec<u8> = Vec::with_capacity(visible_indices.len() * 20);

                for &idx in &visible_indices {
                    let drawable = &input.drawables[idx];
                    let mesh = self.meshes.get(&drawable.mesh.id).unwrap();
                    indirect_bytes.extend_from_slice(&mesh.index_count.to_ne_bytes());
                    indirect_bytes.extend_from_slice(&1u32.to_ne_bytes());
                    indirect_bytes.extend_from_slice(&0u32.to_ne_bytes());
                    indirect_bytes.extend_from_slice(&0i32.to_ne_bytes());
                    indirect_bytes.extend_from_slice(&0u32.to_ne_bytes());
                }

                self.device.write_indirect_draw_buffer(&indirect_bytes, 0);

                let indirect_buf = self
                    .device
                    .indirect_draw_buffer
                    .unwrap_or(vk::Buffer::null());

                if indirect_buf != vk::Buffer::null() {
                    unsafe {
                        d.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, hdr_pl);
                    }

                    for (cmd_idx, &visible_idx) in visible_indices.iter().enumerate() {
                        let drawable = &input.drawables[visible_idx];
                        let mesh = match self.meshes.get(&drawable.mesh.id).cloned() {
                            Some(m) => m,
                            None => continue,
                        };

                        let material =
                            self.material_binding_for_drawable(input, &drawable.material);
                        let material_ubo = Self::parse_material_ubo(&material.uniforms.bytes);
                        let ubo_bytes: &[u8] = unsafe {
                            std::slice::from_raw_parts(
                                &material_ubo as *const _ as *const u8,
                                std::mem::size_of::<MaterialUBO>(),
                            )
                        };
                        let (mat_desc_set, _mat_buf) = self
                            .get_or_create_material_desc_set(&drawable.material.id, ubo_bytes)
                            .unwrap_or_else(|diags| {
                                for d in &diags {
                                    tracing::warn!(
                                        target: "scene_renderer",
                                        code = d.code,
                                        message = d.message,
                                    );
                                }
                                (vk::DescriptorSet::null(), vk::Buffer::null())
                            });

                        if mat_desc_set != vk::DescriptorSet::null() {
                            for tex_slot in &material.textures {
                                let tex_id = &tex_slot.texture.id;
                                if self.device.textures.contains_key(tex_id) {
                                    let _ = self.device.bind_material_texture(tex_id, mat_desc_set);
                                    break;
                                }
                            }
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
                        }

                        let world = &drawable.world_transform;
                        let mut pc_bytes = Vec::with_capacity(128);
                        for v in world {
                            pc_bytes.extend_from_slice(&v.to_ne_bytes());
                        }
                        pc_bytes.resize(128, 0u8);
                        unsafe {
                            d.cmd_push_constants(
                                cmd,
                                hdr_pll,
                                vk::ShaderStageFlags::VERTEX,
                                0,
                                &pc_bytes,
                            );
                        }

                        let vk_vb = self
                            .device
                            .buffers
                            .get(mesh.vertex_buffer.index, mesh.vertex_buffer.generation)
                            .map(|e| e.buffer)
                            .unwrap_or(vk::Buffer::null());
                        let vk_ib = self
                            .device
                            .buffers
                            .get(mesh.index_buffer.index, mesh.index_buffer.generation)
                            .map(|e| e.buffer)
                            .unwrap_or(vk::Buffer::null());

                        if vk_vb != vk::Buffer::null() {
                            let vbs = [vk_vb];
                            let offsets = [0u64];
                            let idx_ty = match mesh.index_format {
                                IndexFormat::U16 => vk::IndexType::UINT16,
                                IndexFormat::U32 => vk::IndexType::UINT32,
                            };
                            unsafe {
                                d.cmd_bind_vertex_buffers(cmd, 0, &vbs, &offsets);
                                d.cmd_bind_index_buffer(cmd, vk_ib, 0, idx_ty);
                            }

                            let cmd_offset = cmd_idx as u64 * 20;
                            unsafe {
                                d.cmd_draw_indexed_indirect(cmd, indirect_buf, cmd_offset, 1, 20);
                            }
                        }
                    }

                    stats.visible_drawables = visible_indices.len() as u32;
                    stats.culled_drawables =
                        input.drawables.len() as u32 - visible_indices.len() as u32;
                }
            }
        }

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

        stats.visible_drawables = input.drawables.len() as u32;
        stats.visible_lights = input.lights.len() as u32;
        Ok(())
    }

    /// Execute the directional shadow (CSM) pass.
    pub(crate) fn execute_shadow_pass(
        &mut self,
        input: &RenderFrameInput,
        stats: &mut FrameStats,
    ) -> Result<(), Vec<Diagnostic>> {
        let rp = self.device.shadow_rp.unwrap_or(vk::RenderPass::null());
        let pll = self
            .device
            .shadow_pipeline_layout
            .unwrap_or(vk::PipelineLayout::null());
        let pl = self.device.shadow_pipeline.unwrap_or(vk::Pipeline::null());
        if rp == vk::RenderPass::null() || pl == vk::Pipeline::null() {
            return Ok(());
        }

        const SHADOW_SIZE: u32 = 2048;
        const CASCADE_COUNT: usize = 3;

        let (view_mat, proj_mat) = if let Some(view) = input.views.first() {
            (
                Mat4::from_cols_array(&view.view_matrix),
                Mat4::from_cols_array(&view.projection_matrix),
            )
        } else {
            (Mat4::IDENTITY, Mat4::IDENTITY)
        };

        let (cascade_splits, light_vps) =
            VulkanDevice::compute_cascade_data(&view_mat, &proj_mat, 0.1, 100.0);

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

            for drawable in &input.drawables {
                if !drawable.cast_shadows {
                    continue;
                }

                let mesh_id = &drawable.mesh.id;
                let mesh = match self.meshes.get(mesh_id).cloned() {
                    Some(m) => m,
                    None => {
                        tracing::trace!(
                            target: "scene_renderer",
                            mesh = mesh_id,
                            "skipping un-cached mesh in shadow pass"
                        );
                        continue;
                    }
                };

                let vk_vb = self
                    .device
                    .buffers
                    .get(mesh.vertex_buffer.index, mesh.vertex_buffer.generation)
                    .map(|e| e.buffer)
                    .unwrap_or(vk::Buffer::null());
                let vk_ib = self
                    .device
                    .buffers
                    .get(mesh.index_buffer.index, mesh.index_buffer.generation)
                    .map(|e| e.buffer)
                    .unwrap_or(vk::Buffer::null());
                if vk_vb == vk::Buffer::null() || vk_ib == vk::Buffer::null() {
                    continue;
                }

                let world = Mat4::from_cols_array(&drawable.world_transform);
                let mvp = light_vp * world;
                let mvp_bytes: &[u8] = unsafe {
                    std::slice::from_raw_parts(
                        &mvp as *const _ as *const u8,
                        std::mem::size_of::<Mat4>(),
                    )
                };
                unsafe {
                    d.cmd_push_constants(cmd, pll, vk::ShaderStageFlags::VERTEX, 0, mvp_bytes);
                    let vbs = [vk_vb];
                    let offsets = [0u64];
                    d.cmd_bind_vertex_buffers(cmd, 0, &vbs, &offsets);
                    d.cmd_bind_index_buffer(cmd, vk_ib, 0, vk::IndexType::UINT32);
                    d.cmd_draw_indexed(cmd, mesh.index_count, 1, 0, 0, 0);
                }

                stats.draw_calls += 1;
                stats.triangles += mesh.index_count as u64 / 3;
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

        stats.visible_drawables = input.drawables.len() as u32;
        stats.visible_lights = input.lights.len() as u32;
        Ok(())
    }

    /// Execute the tone-mapping pass (HDR -> LDR to swapchain).
    pub(crate) fn execute_tonemap_pass(
        &mut self,
        input: &RenderFrameInput,
        stats: &mut FrameStats,
    ) -> Result<(), Vec<Diagnostic>> {
        let Some(ref mut enc) = self.cur_enc else {
            return Ok(());
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
        if tone_rp == vk::RenderPass::null() || tone_pl == vk::Pipeline::null() {
            return Ok(());
        }

        let tone_fb = self
            .device
            .tone_framebuffers
            .get(self.cur_fb_index as usize)
            .copied()
            .unwrap_or(vk::Framebuffer::null());
        if tone_fb == vk::Framebuffer::null() {
            return Ok(());
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
}

// ============================================================================
// BackendRenderer implementation
// ============================================================================

impl BackendRenderer for SceneRenderer {
    // ------------------------------------------------------------------
    // Single-pass legacy path
    // ------------------------------------------------------------------

    fn render_frame(&mut self, input: &RenderFrameInput) -> Result<FrameStats, Vec<Diagnostic>> {
        let msaa = self.device.msaa_samples(input.render_options.msaa_samples);
        let (sc_h, ii, mut encoder) = self.begin_frame_impl(msaa)?;

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

            // ── Bind base color texture (set=2, binding=1) if cached ─────
            for tex_slot in &material.textures {
                let tex_id = &tex_slot.texture.id;
                if self.device.textures.contains_key(tex_id) {
                    let _ = self.device.bind_material_texture(tex_id, mat_desc_set);
                    break; // bind at most one texture per drawable for now
                }
            }

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

            // ── Bind base color texture (set=2, binding=1) if cached ─────
            for tex_slot in &material.textures {
                let tex_id = &tex_slot.texture.id;
                if self.device.textures.contains_key(tex_id) {
                    let _ = self.device.bind_material_texture(tex_id, skinned_desc_set);
                    break;
                }
            }

            // Push constants (128 B, not used by skinned.vert but required by layout)
            if let Some(pll) = self.pll {
                let pc_bytes = vec![0u8; 128];
                encoder.push_constants(pll, 0x01, 0, &pc_bytes);
            }

            encoder.bind_vertex_buffers(&[mesh.vertex_buffer], &[0]);
            encoder.bind_index_buffer(mesh.index_buffer, 0, mesh.index_format);
            encoder.draw_indexed(mesh.index_count, 1, 0, 0, 0);

            draw_calls += 1;
            triangles += mesh.index_count as u64 / 3;
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

        Ok(FrameStats {
            visible_drawables: input.drawables.len() as u32,
            visible_lights: input.lights.len() as u32,
            culled_drawables: 0,
            culled_lights: 0,
            draw_calls,
            triangles,
            gpu_frame_ms: stats.gpu_frame_ms,
        })
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
        let msaa = self.device.msaa_samples(input.render_options.msaa_samples);
        let (sc_h, ii, enc) = self.begin_frame_impl(msaa)?;
        self.cur_sc = Some(sc_h);
        self.cur_ii = Some(ii);
        self.cur_enc = Some(enc);
        Ok(())
    }

    #[allow(unexpected_cfgs, clippy::redundant_guards)]
    fn execute_pass(
        &mut self,
        input: &RenderFrameInput,
        pass: &render_graph::PassNode,
        stats: &mut FrameStats,
    ) -> Result<(), Vec<Diagnostic>> {
        if cfg!(feature = "legacy_dispatch") {
            if self.cur_enc.is_none() {
                return Ok(());
            }

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
                render_graph::PassKind::Custom(name) if name == "bloom" => {
                    if let Some(ref mut enc) = self.cur_enc {
                        let _ = self.pass_registry.find_mut("bloom").map(
                            |p: &mut dyn engine_renderer::RenderPass| {
                                p.execute(input, &mut **enc, stats)
                            },
                        );
                    }
                }
                render_graph::PassKind::Custom(name) if name == "ssao" => {
                    if let Some(ref mut enc) = self.cur_enc {
                        let _ = self.pass_registry.find_mut("ssao").map(
                            |p: &mut dyn engine_renderer::RenderPass| {
                                p.execute(input, &mut **enc, stats)
                            },
                        );
                    }
                }
                render_graph::PassKind::Custom(name) => {
                    tracing::warn!(target: "scene_renderer", pass = name, "unknown custom render pass");
                }
            }

            return Ok(());
        }

        // Default path: dispatch through the PassRegistry.
        if let Some(rp) = self.pass_registry.find_mut(pass.kind.name()) {
            let enc = self.cur_enc.as_mut().unwrap();
            rp.execute(input, &mut **enc, stats)
        } else {
            Ok(())
        }
    }

    fn end_frame(&mut self, stats: &mut FrameStats) -> Result<(), Vec<Diagnostic>> {
        if let (Some(sc_h), Some(ii)) = (self.cur_sc.take(), self.cur_ii.take()) {
            // SAFETY: the encoder was created by `begin_frame` and is still
            // valid; `end_frame` takes ownership and submits the command
            // buffer that has been recorded into during `execute_pass`.
            let enc = self.cur_enc.take().unwrap();
            let s = self.device.end_frame(sc_h, enc, ii).map_err(|e| {
                vec![Diagnostic::new(
                    "RV0209",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("end_frame: {e:?}"),
                )]
            })?;
            stats.draw_calls = s.draw_calls;
            stats.triangles = s.triangles;
            stats.gpu_frame_ms = s.gpu_frame_ms;
        }
        Ok(())
    }

    fn upload_mesh(
        &mut self,
        mesh_id: &str,
        vertex_bytes: &[u8],
        index_bytes: &[u8],
        index_count: u32,
        index_format_u16: bool,
    ) -> Result<(), Vec<Diagnostic>> {
        // Destroy old GPU buffers if re-uploading the same mesh ID.
        if let Some(old) = self.meshes.remove(mesh_id) {
            self.device.destroy_buffer(old.vertex_buffer);
            self.device.destroy_buffer(old.index_buffer);
        }

        // Combined vertex + transfer-dst usage for the vertex buffer.
        let vb_usage = render_core::BufferUsage(
            render_core::BufferUsage::VERTEX.0
                | render_core::BufferUsage::COPY_DST.0,
        );
        let vb_desc = render_core::BufferDescriptor {
            size_bytes: vertex_bytes.len() as u64,
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
        self.device
            .write_buffer(vb, vertex_bytes, 0)
            .map_err(|e| {
                vec![Diagnostic::new(
                    "RV0204",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("upload_mesh write_buffer(vertices): {e:?}"),
                )]
            })?;

        // Index buffer with INDEX usage.
        let ib_usage = render_core::BufferUsage(
            render_core::BufferUsage::INDEX.0 | render_core::BufferUsage::COPY_DST.0,
        );
        let ib_desc = render_core::BufferDescriptor {
            size_bytes: index_bytes.len() as u64,
            usage_flags: ib_usage,
            memory_hint: render_core::MemoryHint::CpuToGpu,
            debug_label: Some(format!("mesh-{mesh_id}-indices")),
        };
        let ib = self.device.create_buffer(&ib_desc).map_err(|e| {
            vec![Diagnostic::new(
                "RV0205",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("upload_mesh create_buffer(indices): {e:?}"),
            )]
        })?;
        self.device.write_buffer(ib, index_bytes, 0).map_err(|e| {
            vec![Diagnostic::new(
                "RV0206",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("upload_mesh write_buffer(indices): {e:?}"),
            )]
        })?;

        let index_format = if index_format_u16 {
            IndexFormat::U16
        } else {
            IndexFormat::U32
        };
        let mesh = GpuMesh {
            vertex_buffer: vb,
            index_buffer: ib,
            index_count,
            index_format,
        };
        self.meshes.insert(mesh_id.to_string(), mesh);
        Ok(())
    }

    fn resize(&mut self, width: u32, height: u32) -> Result<(), Vec<Diagnostic>> {
        self.width = width.max(1);
        self.height = height.max(1);
        self.device.resize(width.max(1), height.max(1));
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
