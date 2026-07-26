//! Vulkan implementation of [`BackendRenderer`].
//!
//! Consumes [`RenderFrameInput`] and renders each drawable through a
//! forward-shaded pipeline with lighting.
//!
//! Resources are uploaded through the typed renderer contract before a frame
//! references them. The render graph records shadow, HDR forward, tone-map and
//! present passes with explicit abort handling when any pass fails.
//!
//! The portable material contract supports opaque, alpha-masked, alpha-blended,
//! and double-sided PBR base-color materials through explicit pipeline
//! variants.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use ash::vk;
use glam::{Mat4, Vec3};

use engine_renderer::{
    render_graph2, AssetId, BackendRenderer, Diagnostic, DiagnosticSeverity, FrameStats, LightItem,
    LightKind, MaterialBinding, MaterialUpload, MeshUpload, MeshVertexFormat, ParamBlock,
    PassRegistry, RenderFrameInput, RenderPass, ResourceKind, ResourceRemoval, SamplerAddressMode,
    SamplerFilter, ShadowMode, TextureSlot, TextureUpload, UiBatch, UploadReceipt,
};
use render_core::{
    self, BufferDescriptor, BufferHandle, CommandEncoder, Device, FramebufferHandle, IndexFormat,
    MemoryHint, PipelineLayoutDescriptor, PipelineLayoutHandle, PushConstantRange,
    RenderPassDescriptor, RenderPassHandle, ShaderFormat, ShaderModuleDescriptor,
    ShaderModuleHandle, ShaderStage, SwapchainDescriptor, SwapchainHandle, TextureFormat,
};

#[cfg(test)]
use render_core::PipelineDescriptor;

use crate::device_impl::VulkanDevice;
use crate::shaders_embedded::{
    FORWARD_FRAG_SPV, FORWARD_VERT_SPV, SKINNED_VERT_SPV, SKYBOX_FRAG_SPV, SKYBOX_VERT_SPV,
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
    pub vertex_format: MeshVertexFormat,
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

#[inline]
fn vulkan_index_type(format: IndexFormat) -> vk::IndexType {
    match format {
        IndexFormat::U16 => vk::IndexType::UINT16,
        IndexFormat::U32 => vk::IndexType::UINT32,
    }
}

const SCENE_FORWARD_PIPELINE_ID: &str = "scene-forward";
const BUILTIN_PASS_KINDS: [&str; 4] = [
    "directional_shadow_pass",
    "opaque_pbr_forward_pass",
    "tone_map_pass",
    "present",
];

fn prepare_and_register_custom_pass(
    registry: &mut PassRegistry,
    device: &mut dyn Device,
    mut pass: Box<dyn RenderPass>,
) -> Result<(), Vec<Diagnostic>> {
    let kind = pass.kind();
    if kind.trim().is_empty() || kind.trim() != kind {
        return Err(vec![Diagnostic::new(
            "RV0298",
            DiagnosticSeverity::Error,
            "scene_renderer",
            format!("custom render pass kind '{kind}' must be a non-empty, trimmed identifier"),
        )]);
    }
    if BUILTIN_PASS_KINDS.contains(&kind) {
        return Err(vec![Diagnostic::new(
            "RV0298",
            DiagnosticSeverity::Error,
            "scene_renderer",
            format!("custom render pass kind '{kind}' is reserved for Vulkan's built-in pass"),
        )]);
    }
    if registry.find(kind).is_some() {
        return Err(vec![Diagnostic::new(
            "RV0299",
            DiagnosticSeverity::Error,
            "scene_renderer",
            format!("custom render pass kind '{kind}' is already registered"),
        )]);
    }

    // Preparation happens before the pass is made visible to graph execution.
    // If it fails, callers may fix the issue and retry without a half-registered
    // entry shadowing the new pass.
    pass.prepare(device)?;
    registry.register(pass);
    Ok(())
}

fn execute_registered_custom_pass(
    registry: &mut PassRegistry,
    name: &str,
    input: &RenderFrameInput,
    encoder: &mut dyn CommandEncoder,
    stats: &mut FrameStats,
) -> Result<(), Vec<Diagnostic>> {
    let Some(pass) = registry.find_mut(name) else {
        return Err(vec![Diagnostic::new(
            "RV0291",
            DiagnosticSeverity::Error,
            "scene_renderer",
            format!("render graph references unregistered custom pass '{name}'"),
        )]);
    };
    if pass.is_enabled(input) {
        pass.execute(input, encoder, stats)?;
    }
    Ok(())
}

fn apply_registered_custom_pass_declarations(
    registry: &PassRegistry,
    graph: &mut engine_renderer::render_graph2::RenderGraph,
) -> Result<(), Vec<Diagnostic>> {
    for node in &mut graph.passes {
        let engine_renderer::render_graph2::PassKind::Custom(name) = &node.kind else {
            continue;
        };
        let name = *name;
        let pass = registry.find(name).ok_or_else(|| {
            vec![Diagnostic::new(
                "RV0291",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("render graph references unregistered custom pass '{name}'"),
            )]
        })?;
        let declaration = pass.declare(node.view_id);
        let declared_kind_matches = matches!(
            declaration.kind,
            render_graph2::PassKind::Custom(declared) if declared == name
        );
        if !declared_kind_matches
            || declaration.view_id != node.view_id
            || declaration.name.trim().is_empty()
        {
            return Err(vec![Diagnostic::new(
                "RV0315",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!(
                    "custom pass '{name}' returned an inconsistent declaration for view {}",
                    node.view_id
                ),
            )]);
        }

        *node = declaration;
    }
    Ok(())
}

fn uploaded_material_binding(upload: &MaterialUpload) -> MaterialBinding {
    let texture_references = upload.texture_references();
    let texture_flags = texture_references
        .iter()
        .enumerate()
        .fold(0_u32, |flags, (index, texture)| {
            flags | u32::from(texture.is_some()) << index
        });
    let mut bytes = Vec::with_capacity(MATERIAL_UBO_SIZE);
    for value in upload.base_color {
        bytes.extend_from_slice(&value.to_ne_bytes());
    }
    for value in [
        upload.metallic,
        upload.roughness,
        upload.ambient_occlusion,
        match upload.transparency {
            engine_renderer::Transparency::Masked { cutoff } => cutoff,
            engine_renderer::Transparency::Opaque | engine_renderer::Transparency::Blend => -1.0,
        },
    ] {
        bytes.extend_from_slice(&value.to_ne_bytes());
    }
    for value in [
        upload.emissive[0],
        upload.emissive[1],
        upload.emissive[2],
        texture_flags as f32,
    ] {
        bytes.extend_from_slice(&value.to_ne_bytes());
    }
    let textures = texture_references
        .into_iter()
        .zip(MATERIAL_TEXTURE_BINDINGS)
        .enumerate()
        .filter_map(|(index, (texture, binding))| {
            texture.map(|texture| TextureSlot {
                binding,
                texture: texture.clone(),
                sampler: AssetId::new(format!("{}::sampler", texture.id)),
                color_space: if matches!(index, 0 | 4) {
                    engine_renderer::ColorSpace::Srgb
                } else {
                    engine_renderer::ColorSpace::Linear
                },
                mip_bias: 0.0,
            })
        })
        .collect();
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

/// CPU-side material UBO layout (48 bytes total).
///
/// Field layout (std140):
/// | offset | field       | type      | bytes |
/// |--------|-------------|-----------|-------|
/// |      0 | base_color  | vec4     |    16 |
/// |     16 | metallic    | float    |     4 |
/// |     20 | roughness   | float    |     4 |
/// |     24 | ao          | float    |     4 |
/// |     28 | alpha_cutoff| float    |     4 |
/// |     32 | emissive     | vec4     |    16 |
/// Total: 48 bytes.
#[repr(C)]
struct MaterialUBO {
    base_color: [f32; 4],
    metallic: f32,
    roughness: f32,
    ao: f32,
    alpha_cutoff: f32,
    emissive: [f32; 4],
}

const MATERIAL_UBO_SIZE: usize = std::mem::size_of::<MaterialUBO>();
const MATERIAL_TEXTURE_BINDINGS: [u32; 5] = [1, 3, 4, 5, 6];

/// Cache entry for a material descriptor set + UBO buffer.
struct MaterialCacheEntry {
    desc_set: vk::DescriptorSet,
    handle: BufferHandle,
    buffer: vk::Buffer,
    ubo_data: [u8; MATERIAL_UBO_SIZE],
    bound_texture_ids: [String; 5],
}

const MAX_MATERIALS: usize = 256;

/// Cache entry for a bone palette descriptor set.
struct BonePaletteCacheEntry {
    desc_set: vk::DescriptorSet,
    bound_texture_ids: [String; 5],
}

/// Cached bone UBO buffer (handle for writes + raw VkBuffer for descriptor binding).
struct CachedBoneBuffer {
    handle: BufferHandle,
    vk_buffer: vk::Buffer,
    ubo_data: Vec<u8>,
}

const MAX_BONE_PALETTES: usize = 64;

// ============================================================================
// Light GPU data packing
// ============================================================================

/// Pack a single [`LightItem`] into the 64-byte GPU Light struct format.
///
/// GPU layout (std430):
///   position[4]    锟?xyz = world position, w = type flag (0=dir, 1=point, 2=spot)
///   direction[4]   锟?xyz = normalized direction, w = unused
///   color[4]       锟?rgb = color, a = intensity
///   attenuation[4] 锟?x = range, y = linear, z = quadratic, w = spot_cutoff_cos
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

const UI_VERTEX_STRIDE: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct UiScissor {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PreparedUiDraw {
    first_vertex: u32,
    vertex_count: u32,
    texture_id: Option<String>,
    scissor: UiScissor,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct PreparedUiOverlay {
    vertex_bytes: Vec<u8>,
    draws: Vec<PreparedUiDraw>,
}

fn first_missing_ui_texture(
    batches: &[UiBatch],
    mut texture_exists: impl FnMut(&str) -> bool,
) -> Option<&str> {
    batches
        .iter()
        .filter_map(|batch| batch.texture.as_ref())
        .map(|asset_id| asset_id.id.as_str())
        .find(|texture_id| !texture_exists(texture_id))
}

fn validate_vulkan_output_mode(
    output_mode: engine_renderer::PassGraphOutputMode,
) -> Result<(), Vec<Diagnostic>> {
    if output_mode == engine_renderer::PassGraphOutputMode::DirectToSwapchain {
        return Err(vec![Diagnostic::new(
            "RV0310",
            DiagnosticSeverity::Error,
            "scene_renderer",
            "Vulkan SceneRenderer does not support DirectToSwapchain; use HdrThenToneMap",
        )]);
    }
    Ok(())
}

fn validate_vulkan_frame_contract(input: &RenderFrameInput) -> Result<(), Vec<Diagnostic>> {
    validate_vulkan_output_mode(input.render_options.pass_graph_config.output_mode)?;

    if input.render_options.msaa_samples != 1
        || input.views.iter().any(|view| view.msaa_samples != 1)
    {
        return Err(vec![Diagnostic::new(
            "RV0317",
            DiagnosticSeverity::Error,
            "scene_renderer",
            "Vulkan SceneRenderer does not yet implement multisample resolve; use 1x MSAA",
        )]);
    }
    if input.views.iter().any(|view| {
        !view.viewport.is_valid_normalized()
            || !view.viewport_rect_normalized.is_valid_normalized()
            || view.viewport != view.viewport_rect_normalized
    }) {
        return Err(vec![Diagnostic::new(
            "RV0318",
            DiagnosticSeverity::Error,
            "scene_renderer",
            "Vulkan SceneRenderer requires matching, valid normalized viewport rectangles",
        )]);
    }
    if input.views.iter().any(|view| {
        !matches!(
            view.clear_flags,
            engine_renderer::ClearFlags::ColorAndDepth | engine_renderer::ClearFlags::Skybox
        )
    }) {
        return Err(vec![Diagnostic::new(
            "RV0319",
            DiagnosticSeverity::Error,
            "scene_renderer",
            "Vulkan SceneRenderer currently supports only ColorAndDepth and Skybox clear modes",
        )]);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
struct VulkanViewportRect {
    viewport: vk::Viewport,
    scissor: vk::Rect2D,
}

fn vulkan_viewport_rect(
    rect: engine_renderer::Rect,
    surface_width: u32,
    surface_height: u32,
) -> Result<VulkanViewportRect, &'static str> {
    if !rect.is_valid_normalized() {
        return Err("viewport must be finite, positive, and contained in [0, 1]");
    }
    if surface_width == 0 || surface_height == 0 {
        return Err("viewport surface dimensions must be positive");
    }
    if surface_width > i32::MAX as u32 || surface_height > i32::MAX as u32 {
        return Err("viewport surface dimensions exceed Vulkan's signed offset range");
    }

    let surface_width_f = surface_width as f32;
    let surface_height_f = surface_height as f32;
    let x = rect.min[0] * surface_width_f;
    let y = rect.min[1] * surface_height_f;
    let right = rect.max[0] * surface_width_f;
    let bottom = rect.max[1] * surface_height_f;
    let scissor_left = x.floor().clamp(0.0, surface_width_f) as u32;
    let scissor_top = y.floor().clamp(0.0, surface_height_f) as u32;
    let scissor_right = right.ceil().clamp(0.0, surface_width_f) as u32;
    let scissor_bottom = bottom.ceil().clamp(0.0, surface_height_f) as u32;

    Ok(VulkanViewportRect {
        viewport: vk::Viewport {
            x,
            y,
            width: right - x,
            height: bottom - y,
            min_depth: 0.0,
            max_depth: 1.0,
        },
        scissor: vk::Rect2D {
            offset: vk::Offset2D {
                x: scissor_left as i32,
                y: scissor_top as i32,
            },
            extent: vk::Extent2D {
                width: scissor_right.saturating_sub(scissor_left),
                height: scissor_bottom.saturating_sub(scissor_top),
            },
        },
    })
}

const TONE_MAP_MODE_ACES: u32 = 0;
const TONE_MAP_MODE_REINHARD: u32 = 1;
const TONE_MAP_MODE_NONE: u32 = 2;

#[derive(Clone, Copy, Debug, PartialEq)]
struct ToneMapPushConstants {
    mode: u32,
    exposure: f32,
    output_is_srgb: u32,
    padding: u32,
}

impl ToneMapPushConstants {
    const SIZE: usize = 16;

    fn to_bytes(self) -> [u8; Self::SIZE] {
        let mut bytes = [0; Self::SIZE];
        bytes[0..4].copy_from_slice(&self.mode.to_ne_bytes());
        bytes[4..8].copy_from_slice(&self.exposure.to_ne_bytes());
        bytes[8..12].copy_from_slice(&self.output_is_srgb.to_ne_bytes());
        bytes[12..16].copy_from_slice(&self.padding.to_ne_bytes());
        bytes
    }
}

fn swapchain_format_is_srgb(format: vk::Format) -> bool {
    matches!(
        format,
        vk::Format::R8G8B8A8_SRGB | vk::Format::B8G8R8A8_SRGB | vk::Format::A8B8G8R8_SRGB_PACK32
    )
}

fn tone_map_push_constants(
    tone_mapping: engine_renderer::ToneMapping,
    exposure_ev100: Option<f32>,
    swapchain_format: vk::Format,
) -> Result<ToneMapPushConstants, String> {
    let mode = match tone_mapping {
        engine_renderer::ToneMapping::Aces => TONE_MAP_MODE_ACES,
        engine_renderer::ToneMapping::Reinhard => TONE_MAP_MODE_REINHARD,
        engine_renderer::ToneMapping::None => TONE_MAP_MODE_NONE,
    };

    // `exposure_ev100` is an absolute EV100 override, not an EV compensation.
    // A missing override is neutral exposure; an override uses exp2(-EV100).
    let exposure = match exposure_ev100 {
        None => 1.0,
        Some(ev100) if ev100.is_finite() => (-ev100).exp2(),
        Some(ev100) => {
            return Err(format!("exposure_ev100 must be finite, received {ev100}"));
        }
    };
    if !exposure.is_finite() {
        return Err(format!(
            "exposure_ev100 {exposure_ev100:?} produces a non-finite exposure multiplier"
        ));
    }

    Ok(ToneMapPushConstants {
        mode,
        exposure,
        output_is_srgb: u32::from(swapchain_format_is_srgb(swapchain_format)),
        padding: 0,
    })
}

fn ui_scissor(
    batch: usize,
    rect: engine_renderer::Rect,
    width: u32,
    height: u32,
) -> Result<Option<UiScissor>, String> {
    if rect
        .min
        .into_iter()
        .chain(rect.max)
        .any(|value| !value.is_finite())
    {
        return Err(format!("UI batch {batch} has a non-finite clip rectangle"));
    }
    if rect.max[0] < rect.min[0] || rect.max[1] < rect.min[1] {
        return Err(format!("UI batch {batch} has an inverted clip rectangle"));
    }

    let max_x = width.min(i32::MAX as u32) as f32;
    let max_y = height.min(i32::MAX as u32) as f32;
    let x0 = rect.min[0].floor().clamp(0.0, max_x) as i32;
    let y0 = rect.min[1].floor().clamp(0.0, max_y) as i32;
    let x1 = rect.max[0].ceil().clamp(0.0, max_x) as i32;
    let y1 = rect.max[1].ceil().clamp(0.0, max_y) as i32;
    if x1 <= x0 || y1 <= y0 {
        return Ok(None);
    }
    Ok(Some(UiScissor {
        x: x0,
        y: y0,
        width: (x1 - x0) as u32,
        height: (y1 - y0) as u32,
    }))
}

/// Expand each UI batch's indices into a shared non-indexed vertex stream,
/// retaining one draw record per input batch.  Draw records are deliberately
/// not sorted or merged: batch order, texture selection and clip state are
/// observable parts of the UI contract.
fn prepare_ui_overlay(
    batches: &[UiBatch],
    width: u32,
    height: u32,
) -> Result<PreparedUiOverlay, String> {
    let mut prepared = PreparedUiOverlay::default();
    for (batch_index, batch) in batches.iter().enumerate() {
        if batch.indices.len() % 3 != 0 {
            return Err(format!(
                "UI batch {batch_index} index count {} is not a triangle-list multiple",
                batch.indices.len()
            ));
        }
        let Some(scissor) = ui_scissor(batch_index, batch.clip_rect, width, height)? else {
            continue;
        };
        if batch.indices.is_empty() {
            continue;
        }
        let first_vertex = u32::try_from(prepared.vertex_bytes.len() / UI_VERTEX_STRIDE)
            .map_err(|_| "UI vertex offset exceeds u32".to_owned())?;
        for &index in &batch.indices {
            let vertex = batch.vertices.get(index as usize).ok_or_else(|| {
                format!(
                    "UI batch {batch_index} index {index} is outside {} vertices",
                    batch.vertices.len()
                )
            })?;
            if vertex
                .position
                .into_iter()
                .chain(vertex.uv)
                .any(|value| !value.is_finite())
            {
                return Err(format!(
                    "UI batch {batch_index} vertex {index} contains non-finite data"
                ));
            }
            prepared
                .vertex_bytes
                .extend_from_slice(&vertex.position[0].to_ne_bytes());
            prepared
                .vertex_bytes
                .extend_from_slice(&vertex.position[1].to_ne_bytes());
            prepared
                .vertex_bytes
                .extend_from_slice(&vertex.uv[0].to_ne_bytes());
            prepared
                .vertex_bytes
                .extend_from_slice(&vertex.uv[1].to_ne_bytes());
            for channel in vertex.color {
                prepared
                    .vertex_bytes
                    .extend_from_slice(&(f32::from(channel) / 255.0).to_ne_bytes());
            }
        }
        let vertex_count = u32::try_from(batch.indices.len())
            .map_err(|_| format!("UI batch {batch_index} has too many indices"))?;
        prepared.draws.push(PreparedUiDraw {
            first_vertex,
            vertex_count,
            texture_id: batch.texture.as_ref().map(|asset_id| asset_id.id.clone()),
            scissor,
        });
    }
    Ok(prepared)
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
    forward_shader_modules: Vec<ShaderModuleHandle>,
    skinned_shader_modules: Vec<ShaderModuleHandle>,

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
    pass_registry: PassRegistry,

    /// One CPU-visible UI vertex buffer per in-flight frame. Reusing a single
    /// buffer would race the other frame slot while the GPU is reading it.
    ui_vbs: [Option<BufferHandle>; 2],
    ui_vb_capacities: [u64; 2],

    /// Per-pass GPU timestamp state machine (ENG-04). Async read-back lands
    /// frames-in-flight frames after recording; unavailable/disabled states
    /// degrade to status reporting only.
    gpu_timestamps: crate::timestamps::GpuTimestampProfiler,
    /// Lazily created timestamp query pools (one per frame-in-flight slot).
    timestamp_pools: crate::timestamps::TimestampQueryPools,
    /// Engine configuration switch for GPU timestamps.
    gpu_timing_enabled: bool,
    /// Whether device timestamp support was already evaluated.
    gpu_timing_configured: bool,
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
            forward_shader_modules: Vec::new(),
            skinned_shader_modules: Vec::new(),
            ui_vbs: [None; 2],
            ui_vb_capacities: [0; 2],
            framebuffers: Vec::new(),
            cur_fb_index: 0,
            cur_sc: None,
            cur_ii: None,
            cur_enc: None,
            width: width.max(1),
            height: height.max(1),
            pass_registry: PassRegistry::new(),
            gpu_timestamps: crate::timestamps::GpuTimestampProfiler::new(),
            timestamp_pools: crate::timestamps::TimestampQueryPools::new(),
            gpu_timing_enabled: true,
            gpu_timing_configured: false,
        }
    }

    /// Register and prepare a custom render pass.
    ///
    /// Registration is allowed whenever no frame is active. Preparation is
    /// performed exactly once before the pass is inserted into the registry,
    /// so a failing pass cannot become visible to graph execution. Built-in
    /// pass names are reserved because Vulkan dispatches those passes directly.
    pub fn register_pass(&mut self, pass: Box<dyn RenderPass>) -> Result<(), Vec<Diagnostic>> {
        if self.cur_enc.is_some() || self.cur_sc.is_some() || self.cur_ii.is_some() {
            return Err(vec![Diagnostic::new(
                "RV0297",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "cannot register a custom render pass while a frame is active",
            )]);
        }
        prepare_and_register_custom_pass(&mut self.pass_registry, &mut self.device, pass)
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
    // Pipeline initialisation  (lazy 锟?called on the first frame)
    // ------------------------------------------------------------------

    fn configure_scene_shaders(&mut self) {
        self.device
            .set_forward_shaders(FORWARD_VERT_SPV, FORWARD_FRAG_SPV);
        self.device
            .set_skybox_shaders(SKYBOX_VERT_SPV, SKYBOX_FRAG_SPV);
        if !SKINNED_VERT_SPV.is_empty() {
            self.device.set_skinned_vertex_shader(SKINNED_VERT_SPV);
        }
    }

    fn create_scene_shader_modules(&mut self) -> Result<(), Vec<Diagnostic>> {
        if !self.forward_shader_modules.is_empty() && !self.skinned_shader_modules.is_empty() {
            return Ok(());
        }
        if FORWARD_VERT_SPV.is_empty() || FORWARD_FRAG_SPV.is_empty() || SKINNED_VERT_SPV.is_empty()
        {
            return Err(vec![Diagnostic::new(
                "RV0293",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "embedded forward or skinned SPIR-V is unavailable",
            )]);
        }

        let forward_vertex = self
            .device
            .create_shader_module(&ShaderModuleDescriptor {
                format: ShaderFormat::SpirV,
                stage: ShaderStage::Vertex,
                source_bytes: FORWARD_VERT_SPV.to_vec(),
                entry_points: vec!["main".into()],
                source_hash: [0x61; 32],
                debug_label: Some("scene-forward-vs".into()),
            })
            .map_err(|error| {
                vec![Diagnostic::new(
                    "RV0294",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("create forward vertex shader: {error:?}"),
                )]
            })?;

        let fragment = match self.device.create_shader_module(&ShaderModuleDescriptor {
            format: ShaderFormat::SpirV,
            stage: ShaderStage::Fragment,
            source_bytes: FORWARD_FRAG_SPV.to_vec(),
            entry_points: vec!["main".into()],
            source_hash: [0x62; 32],
            debug_label: Some("scene-forward-fs".into()),
        }) {
            Ok(shader) => shader,
            Err(error) => {
                self.device.destroy_shader_module(forward_vertex);
                return Err(vec![Diagnostic::new(
                    "RV0295",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("create forward fragment shader: {error:?}"),
                )]);
            }
        };

        let skinned_vertex = match self.device.create_shader_module(&ShaderModuleDescriptor {
            format: ShaderFormat::SpirV,
            stage: ShaderStage::Vertex,
            source_bytes: SKINNED_VERT_SPV.to_vec(),
            entry_points: vec!["main".into()],
            source_hash: [0x63; 32],
            debug_label: Some("scene-skinned-vs".into()),
        }) {
            Ok(shader) => shader,
            Err(error) => {
                self.device.destroy_shader_module(fragment);
                self.device.destroy_shader_module(forward_vertex);
                return Err(vec![Diagnostic::new(
                    "RV0296",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("create skinned vertex shader: {error:?}"),
                )]);
            }
        };

        self.forward_shader_modules = vec![forward_vertex, fragment];
        self.skinned_shader_modules = vec![skinned_vertex, fragment];
        Ok(())
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
            present_after: true,
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
                size: 128, // 4锟? f32 matrix (64 B) + spare uniform data
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

        self.create_scene_shader_modules()?;

        // 鈹€鈹€ Material descriptor infrastructure (set=2: UBO + texture) 鈹€
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

        // 鈹€鈹€ Shadow-mapping resources 鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€
        // Ensure the device has created shadow resources (idempotent).
        self.device.ensure_shadow().map_err(|e| {
            vec![Diagnostic::new(
                "RV0211",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("ensure_shadow: {e:?}"),
            )]
        })?;

        // 鈹€鈹€ Environment cubemap (IBL, set=1 binding=1) 鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€
        self.device.create_env_cubemap().map_err(|e| {
            vec![Diagnostic::new(
                "RV0212",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("create_env_cubemap: {e:?}"),
            )]
        })?;

        // 鈹€鈹€ Light SSBO (set=1 binding=2) 鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€
        self.device.create_light_ssbo().map_err(|e| {
            vec![Diagnostic::new(
                "RV0222",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("create_light_ssbo: {e:?}"),
            )]
        })?;

        // 鈹€鈹€ Indirect draw buffers (Phase 5.1) 鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€
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

        // 鈹€鈹€ Framebuffers (per swapchain image, color + depth) 鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€
        self.framebuffers = self.device.create_scene_framebuffers(rp).map_err(|e| {
            vec![Diagnostic::new(
                "RV0213",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("create_scene_framebuffers: {e:?}"),
            )]
        })?;

        // 鈹€鈹€ UI overlay pipeline 鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€
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
        self.framebuffers =
            self.device
                .create_scene_framebuffers(render_pass)
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
    ) -> Result<MaterialBinding, Vec<Diagnostic>> {
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
            .ok_or_else(|| {
                vec![Diagnostic::new(
                    "RV0232",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("material '{}' was not uploaded", material_id.id),
                )]
            })
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
        let validate_mesh = |mesh_id: &str, expected_format: MeshVertexFormat| {
            let Some(mesh) = self.meshes.get(mesh_id) else {
                return Err(vec![Diagnostic::new(
                    "RV0230",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("drawable references mesh '{mesh_id}' before a successful upload"),
                )]);
            };
            if mesh.vertex_format != expected_format {
                return Err(vec![Diagnostic::new(
                    "RV0292",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!(
                        "mesh '{mesh_id}' has {:?} vertices but this draw requires {:?}",
                        mesh.vertex_format, expected_format
                    ),
                )]);
            }
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
            Ok(())
        };
        for mesh_id in input.drawables.iter().map(|item| &item.mesh.id) {
            validate_mesh(mesh_id, MeshVertexFormat::Pbr32)?;
        }
        for mesh_id in input.skinned_items.iter().map(|item| &item.mesh.id) {
            validate_mesh(mesh_id, MeshVertexFormat::Skinned64)?;
        }
        for material_id in input
            .drawables
            .iter()
            .map(|item| &item.material)
            .chain(input.skinned_items.iter().map(|item| &item.material))
        {
            let material = self.material_binding_for_drawable(input, material_id)?;
            self.selected_material_texture_ids(&material)?;
        }
        Ok(())
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

        // Check bone buffer cache 锟?if found, update data and return.
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
            usage_flags: render_core::BufferUsage::UNIFORM,
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
    ///   bindings=1,3..=6: material textures (updated after allocation)
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
            .allocate_skinned_material_descriptor_set(
                mat_buffer,
                MATERIAL_UBO_SIZE as u64,
                bone_buffer,
                4096,
            )
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
                bound_texture_ids: std::array::from_fn(|_| {
                    crate::device_impl::texture::FALLBACK_MATERIAL_TEXTURE_ID.to_string()
                }),
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
    ///   [0..16)  base_color  锟?vec4 f32
    ///   [16..20) metallic    锟?f32
    ///   [20..24) roughness   锟?f32
    ///   [24..28) ao          锟?f32
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
        let read_vec4 = |offset: usize, fallback: [f32; 4]| -> [f32; 4] {
            if offset + 16 <= bytes.len() {
                [
                    f32::from_ne_bytes(bytes[offset..offset + 4].try_into().unwrap()),
                    f32::from_ne_bytes(bytes[offset + 4..offset + 8].try_into().unwrap()),
                    f32::from_ne_bytes(bytes[offset + 8..offset + 12].try_into().unwrap()),
                    f32::from_ne_bytes(bytes[offset + 12..offset + 16].try_into().unwrap()),
                ]
            } else {
                fallback
            }
        };
        MaterialUBO {
            base_color: read_vec4(0, [0.8, 0.6, 0.4, 1.0]),
            metallic: read_f32(16, 0.0).clamp(0.0, 1.0),
            roughness: read_f32(20, 1.0).clamp(0.04, 1.0),
            ao: read_f32(24, 1.0).clamp(0.0, 1.0),
            alpha_cutoff: read_f32(28, -1.0),
            emissive: read_vec4(32, [0.0; 4]),
        }
    }

    fn material_texture_flags(material: &MaterialBinding) -> f32 {
        MATERIAL_TEXTURE_BINDINGS
            .iter()
            .enumerate()
            .fold(0_u32, |flags, (index, binding)| {
                flags
                    | (u32::from(
                        material
                            .textures
                            .iter()
                            .any(|slot| slot.binding == *binding),
                    ) << index)
            }) as f32
    }

    fn selected_material_texture_ids(
        &self,
        material: &MaterialBinding,
    ) -> Result<[String; 5], Vec<Diagnostic>> {
        use crate::device_impl::texture::FALLBACK_MATERIAL_TEXTURE_ID;

        let mut selected = std::array::from_fn(|_| FALLBACK_MATERIAL_TEXTURE_ID.to_string());
        for (index, binding) in MATERIAL_TEXTURE_BINDINGS.into_iter().enumerate() {
            let Some(slot) = material
                .textures
                .iter()
                .find(|slot| slot.binding == binding)
            else {
                continue;
            };
            if !self.device.textures.contains_key(&slot.texture.id) {
                return Err(vec![Diagnostic::new(
                    "RV0260",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!(
                        "material '{}' references texture '{}' at binding {} before a successful upload",
                        material.material_id.id, slot.texture.id, binding
                    ),
                )]);
            }
            selected[index] = slot.texture.id.clone();
        }
        Ok(selected)
    }

    fn bind_material_texture_if_changed(
        &mut self,
        material_id: &str,
        material: &MaterialBinding,
        descriptor_set: vk::DescriptorSet,
    ) -> Result<(), Vec<Diagnostic>> {
        let selected = self.selected_material_texture_ids(material)?;
        let current = self
            .material_cache
            .get(material_id)
            .map(|entry| entry.bound_texture_ids.clone())
            .unwrap_or_else(|| std::array::from_fn(|_| String::new()));
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
        for (index, binding) in MATERIAL_TEXTURE_BINDINGS.into_iter().enumerate() {
            if current[index] == selected[index] {
                continue;
            }
            let bound = self
                .device
                .bind_material_texture_at(&selected[index], binding, descriptor_set)
                .map_err(|error| {
                    vec![Diagnostic::new(
                        "RV0262",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        format!(
                            "bind material texture '{}' at binding {binding}: {error:?}",
                            selected[index]
                        ),
                    )]
                })?;
            if !bound {
                return Err(vec![Diagnostic::new(
                    "RV0263",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!(
                        "material texture '{}' disappeared before descriptor update",
                        selected[index]
                    ),
                )]);
            }
        }
        if let Some(entry) = self.material_cache.get_mut(material_id) {
            entry.bound_texture_ids = selected;
        }
        Ok(())
    }

    fn bind_skinned_texture_if_changed(
        &mut self,
        cache_key: &str,
        material: &MaterialBinding,
        descriptor_set: vk::DescriptorSet,
    ) -> Result<(), Vec<Diagnostic>> {
        let selected = self.selected_material_texture_ids(material)?;
        let current = self
            .skinned_desc_cache
            .get(cache_key)
            .map(|entry| entry.bound_texture_ids.clone())
            .unwrap_or_else(|| std::array::from_fn(|_| String::new()));
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
        for (index, binding) in MATERIAL_TEXTURE_BINDINGS.into_iter().enumerate() {
            if current[index] == selected[index] {
                continue;
            }
            let bound = self
                .device
                .bind_material_texture_at(&selected[index], binding, descriptor_set)
                .map_err(|error| {
                    vec![Diagnostic::new(
                        "RV0265",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        format!(
                            "bind skinned texture '{}' at binding {binding}: {error:?}",
                            selected[index]
                        ),
                    )]
                })?;
            if !bound {
                return Err(vec![Diagnostic::new(
                    "RV0266",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!(
                        "skinned texture '{}' disappeared before descriptor update",
                        selected[index]
                    ),
                )]);
            }
        }
        if let Some(entry) = self.skinned_desc_cache.get_mut(cache_key) {
            entry.bound_texture_ids = selected;
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
        let ubo_array: [u8; MATERIAL_UBO_SIZE] = ubo_data.try_into().map_err(|_| {
            vec![Diagnostic::new(
                "RV0250",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("material UBO must be exactly {MATERIAL_UBO_SIZE} bytes"),
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

        // Create a small UBO buffer for MaterialUBO.
        let buf_desc = BufferDescriptor {
            size_bytes: MATERIAL_UBO_SIZE as u64,
            usage_flags: render_core::BufferUsage::UNIFORM,
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
        let desc_set = match self
            .device
            .allocate_material_descriptor_set(vk_buf, MATERIAL_UBO_SIZE as u64)
        {
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
            bound_texture_ids: std::array::from_fn(|_| {
                crate::device_impl::texture::FALLBACK_MATERIAL_TEXTURE_ID.to_string()
            }),
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
        if let Some(texture_id) = first_missing_ui_texture(&input.ui_batches, |id| {
            self.device.textures.contains_key(id)
        }) {
            return Err(vec![Diagnostic::new(
                "RV0308",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("UI batch references texture '{texture_id}' before a successful upload"),
            )]);
        }
        prepare_ui_overlay(&input.ui_batches, self.width, self.height).map_err(|message| {
            vec![Diagnostic::new(
                "RV0298",
                DiagnosticSeverity::Error,
                "scene_renderer",
                message,
            )]
        })?;

        // Apply the requested MSAA sample count to the device before any
        // resource creation takes place (ensure_sc 锟?ensure_hdr_resources).
        self.device.hdr_msaa_samples = msaa_samples;
        if !self.initialized {
            // Swapchain setup creates the HDR forward pipeline, so the scene
            // shaders must be registered before `create_swapchain`.
            self.configure_scene_shaders();
        }

        if self.device.swapchain_recreate_pending {
            self.device.wait_idle_checked().map_err(|error| {
                vec![Diagnostic::new(
                    "RV0314",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("wait for suboptimal swapchain replacement: {error}"),
                )]
            })?;
            self.device.destroy_scene_framebuffers(&self.framebuffers);
            self.framebuffers.clear();
            self.device.destroy_swapchain_resources();
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

        if !input.ui_batches.is_empty() {
            self.device.ensure_ui_overlay_resources().map_err(|error| {
                vec![Diagnostic::new(
                    "RV0309",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("initialize UI overlay resources: {error}"),
                )]
            })?;
        }

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

        let render_view = input.views.first().ok_or_else(|| {
            vec![Diagnostic::new(
                "RV0013",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "HDR forward pass requires a RenderView",
            )]
        })?;
        let scene_viewport = vulkan_viewport_rect(
            render_view.viewport_rect_normalized,
            self.width,
            self.height,
        )
        .map_err(|message| {
            vec![Diagnostic::new(
                "RV0318",
                DiagnosticSeverity::Error,
                "scene_renderer",
                message,
            )]
        })?;
        // Render-pass load operations clear only the scene's render area. The
        // rest of the HDR attachment is intentionally never sampled by the
        // sub-viewport tone-map draw.
        let clear_color = render_view.clear_color;
        let clear_values = [
            vk::ClearValue {
                color: vk::ClearColorValue {
                    float32: clear_color,
                },
            },
            vk::ClearValue {
                depth_stencil: vk::ClearDepthStencilValue {
                    depth: 1.0,
                    stencil: 0,
                },
            },
        ];
        let rpbi = vk::RenderPassBeginInfo::default()
            .render_pass(hdr_rp)
            .framebuffer(hdr_fb)
            .render_area(scene_viewport.scissor)
            .clear_values(&clear_values);
        // SAFETY: command buffer is in recording state; RP, FB valid.
        unsafe {
            d.cmd_begin_render_pass(cmd, &rpbi, vk::SubpassContents::INLINE);
        }

        unsafe {
            d.cmd_set_viewport(cmd, 0, &[scene_viewport.viewport]);
            d.cmd_set_scissor(cmd, 0, &[scene_viewport.scissor]);
        }

        // Draw the environment cubemap before opaque geometry. The skybox
        // pipeline does not write depth, so all scene geometry naturally
        // replaces it while untouched pixels retain the environment.
        if input
            .views
            .first()
            .is_some_and(|view| view.clear_flags == engine_renderer::ClearFlags::Skybox)
        {
            let skybox_pipeline = self.device.hdr_skybox_pipeline.ok_or_else(|| {
                vec![Diagnostic::new(
                    "RV0321",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    "HDR skybox pipeline is unavailable",
                )]
            })?;
            unsafe {
                d.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, skybox_pipeline);
            }
            if let Some(desc_set) = self.device.frame_descriptor_set(fi) {
                unsafe {
                    d.cmd_bind_descriptor_sets(
                        cmd,
                        vk::PipelineBindPoint::GRAPHICS,
                        hdr_pll,
                        0,
                        &[desc_set],
                        &[],
                    );
                }
            }
            if let Some(desc_set) = self.device.shadow_desc_set {
                unsafe {
                    d.cmd_bind_descriptor_sets(
                        cmd,
                        vk::PipelineBindPoint::GRAPHICS,
                        hdr_pll,
                        1,
                        &[desc_set],
                        &[],
                    );
                    d.cmd_draw(cmd, 36, 1, 0, 0);
                }
                stats.draw_calls += 1;
                stats.triangles += 12;
            }
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

        // Opaque/masked drawables retain extraction order for batching. Blended
        // drawables render afterwards, back-to-front, with depth writes disabled.
        let camera_position = Mat4::from_cols_array(&render_view.view_matrix)
            .inverse()
            .w_axis
            .truncate();
        let mut ordered_drawables = Vec::with_capacity(input.drawables.len());
        let mut blended_drawables = Vec::new();
        for drawable in &input.drawables {
            let material = self.material_binding_for_drawable(input, &drawable.material)?;
            if matches!(material.transparency, engine_renderer::Transparency::Blend) {
                let translation = Vec3::new(
                    drawable.world_transform[12],
                    drawable.world_transform[13],
                    drawable.world_transform[14],
                );
                blended_drawables
                    .push(((translation - camera_position).length_squared(), drawable));
            } else {
                ordered_drawables.push(drawable);
            }
        }
        blended_drawables.sort_by(|left, right| right.0.total_cmp(&left.0));
        ordered_drawables.extend(blended_drawables.into_iter().map(|(_, drawable)| drawable));

        // Draw calls with dynamic batching.
        let mut last_material_id: Option<&str> = None;
        let mut last_mesh_id: Option<&str> = None;
        let mut current_material_pipeline = hdr_pl;
        #[allow(unused_assignments)]
        let mut cached_vb = vk::Buffer::null();
        #[allow(unused_assignments)]
        let mut cached_ib = vk::Buffer::null();
        #[allow(unused_assignments)]
        let mut cached_idx_ty = vk::IndexType::UINT32;
        let mut cached_index_count = 0u32;
        for drawable in ordered_drawables {
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
            // When the mesh is unchanged, the vertex and index buffers remain bound.

            // Skip material descriptor rebind when same as last drawable
            if Some(material_id.as_str()) != last_material_id {
                let material = self.material_binding_for_drawable(input, &drawable.material)?;
                let next_pipeline = match (&material.transparency, material.double_sided) {
                    (engine_renderer::Transparency::Blend, true) => self
                        .device
                        .hdr_forward_blend_double_sided_pipeline
                        .unwrap_or(vk::Pipeline::null()),
                    (engine_renderer::Transparency::Blend, false) => self
                        .device
                        .hdr_forward_blend_pipeline
                        .unwrap_or(vk::Pipeline::null()),
                    (_, true) => self
                        .device
                        .hdr_forward_double_sided_pipeline
                        .unwrap_or(vk::Pipeline::null()),
                    (_, false) => hdr_pl,
                };
                if next_pipeline == vk::Pipeline::null() {
                    return Err(vec![Diagnostic::new(
                        "RV0322",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        format!(
                            "material '{}' requires an unavailable surface pipeline",
                            material.material_id.id
                        ),
                    )]);
                }
                if next_pipeline != current_material_pipeline {
                    unsafe {
                        d.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, next_pipeline);
                    }
                    current_material_pipeline = next_pipeline;
                }
                let mut material_ubo = Self::parse_material_ubo(&material.uniforms.bytes);
                material_ubo.emissive[3] = Self::material_texture_flags(&material);
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

        // Skinned items use the same surface variants and transparent ordering.
        let mut ordered_skinned = Vec::with_capacity(input.skinned_items.len());
        let mut blended_skinned = Vec::new();
        for item in &input.skinned_items {
            let material = self.material_binding_for_drawable(input, &item.material)?;
            if matches!(material.transparency, engine_renderer::Transparency::Blend) {
                let translation = Vec3::new(
                    item.world_transform[12],
                    item.world_transform[13],
                    item.world_transform[14],
                );
                blended_skinned.push(((translation - camera_position).length_squared(), item));
            } else {
                ordered_skinned.push(item);
            }
        }
        blended_skinned.sort_by(|left, right| right.0.total_cmp(&left.0));
        ordered_skinned.extend(blended_skinned.into_iter().map(|(_, item)| item));

        // Skinned items (less batching opportunity due to unique per-item bone data)
        let mut last_skinned_mesh: Option<&str> = None;
        #[allow(unused_assignments)]
        let mut skinned_cached_vb = vk::Buffer::null();
        #[allow(unused_assignments)]
        let mut skinned_cached_ib = vk::Buffer::null();
        #[allow(unused_assignments)]
        let mut skinned_cached_idx_ty = vk::IndexType::UINT32;
        let mut skinned_cached_index_count = 0u32;
        for skinned in ordered_skinned {
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
            let material = self.material_binding_for_drawable(input, &skinned.material)?;
            let next_pipeline = match (&material.transparency, material.double_sided) {
                (engine_renderer::Transparency::Blend, true) => self
                    .device
                    .hdr_forward_blend_double_sided_pipeline
                    .unwrap_or(vk::Pipeline::null()),
                (engine_renderer::Transparency::Blend, false) => self
                    .device
                    .hdr_forward_blend_pipeline
                    .unwrap_or(vk::Pipeline::null()),
                (_, true) => self
                    .device
                    .hdr_forward_double_sided_pipeline
                    .unwrap_or(vk::Pipeline::null()),
                (_, false) => hdr_pl,
            };
            if next_pipeline == vk::Pipeline::null() {
                return Err(vec![Diagnostic::new(
                    "RV0322",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!(
                        "material '{}' requires an unavailable surface pipeline",
                        material.material_id.id
                    ),
                )]);
            }
            if next_pipeline != current_material_pipeline {
                unsafe {
                    d.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, next_pipeline);
                }
                current_material_pipeline = next_pipeline;
            }
            let mut material_ubo = Self::parse_material_ubo(&material.uniforms.bytes);
            material_ubo.emissive[3] = Self::material_texture_flags(&material);
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
                if matches!(
                    self.material_binding_for_drawable(input, &drawable.material)?
                        .transparency,
                    engine_renderer::Transparency::Blend
                ) {
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
        if self.cur_enc.is_none() {
            return Err(vec![Diagnostic::new(
                "RV0227",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "tone-map pass requires an active frame encoder",
            )]);
        }

        let swapchain_format = self
            .device
            .swapchain
            .as_ref()
            .map(|swapchain| swapchain.format)
            .ok_or_else(|| {
                vec![Diagnostic::new(
                    "RV0228",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    "tone-map pass requires an active swapchain format",
                )]
            })?;
        let tone_map_push = tone_map_push_constants(
            input.render_options.tone_mapping,
            input.render_options.exposure_ev100,
            swapchain_format,
        )
        .map_err(|message| {
            vec![Diagnostic::new(
                "RV0320",
                DiagnosticSeverity::Error,
                "scene_renderer",
                message,
            )
            .path("render_options.exposure_ev100")]
        })?;
        let tone_map_push_bytes = tone_map_push.to_bytes();
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

        let render_view = input.views.first().ok_or_else(|| {
            vec![Diagnostic::new(
                "RV0013",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "tone-map pass requires a RenderView",
            )]
        })?;
        let scene_viewport = vulkan_viewport_rect(
            render_view.viewport_rect_normalized,
            self.width,
            self.height,
        )
        .map_err(|message| {
            vec![Diagnostic::new(
                "RV0318",
                DiagnosticSeverity::Error,
                "scene_renderer",
                message,
            )]
        })?;
        let swapchain_clear = [vk::ClearValue {
            color: vk::ClearColorValue {
                float32: [0.0, 0.0, 0.0, 1.0],
            },
        }];

        let rpbi = vk::RenderPassBeginInfo::default()
            .render_pass(tone_rp)
            .framebuffer(tone_fb)
            .render_area(vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: vk::Extent2D {
                    width: self.width,
                    height: self.height,
                },
            })
            .clear_values(&swapchain_clear);
        unsafe {
            d.cmd_begin_render_pass(cmd, &rpbi, vk::SubpassContents::INLINE);
        }

        unsafe {
            d.cmd_set_viewport(
                cmd,
                0,
                &[vk::Viewport {
                    x: 0.0,
                    y: 0.0,
                    width: self.width as f32,
                    height: self.height as f32,
                    min_depth: 0.0,
                    max_depth: 1.0,
                }],
            );
            // Keep the full-surface viewport so the full-screen triangle's UV
            // coordinates sample the matching pixels in the HDR attachment.
            // Scissoring only the scene region prevents it from covering the
            // editor chrome or an authored letterbox region.
            d.cmd_set_scissor(cmd, 0, &[scene_viewport.scissor]);
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

        unsafe {
            d.cmd_push_constants(
                cmd,
                tone_pll,
                vk::ShaderStageFlags::FRAGMENT,
                0,
                &tone_map_push_bytes,
            );
        }

        unsafe {
            d.cmd_draw(cmd, 3, 1, 0, 0);
            d.cmd_end_render_pass(cmd);
        }

        stats.draw_calls += 1;
        stats.triangles += 1;
        Ok(())
    }

    // 鈹€鈹€ UI overlay rendering 鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€

    /// Render UI in a dedicated load-op pass over the tone-mapped swapchain
    /// image. This is invoked by the graph's Present pass, so no later scene
    /// pass can overwrite the overlay.
    fn execute_ui_overlay_pass(
        &mut self,
        batches: &[UiBatch],
        stats: &mut FrameStats,
    ) -> Result<(), Vec<Diagnostic>> {
        if batches.is_empty() {
            return Ok(());
        }
        let prepared = prepare_ui_overlay(batches, self.width, self.height).map_err(|message| {
            vec![Diagnostic::new(
                "RV0298",
                DiagnosticSeverity::Error,
                "scene_renderer",
                message,
            )]
        })?;
        if prepared.draws.is_empty() {
            return Ok(());
        }

        let mut descriptor_sets = Vec::with_capacity(prepared.draws.len());
        for draw in &prepared.draws {
            let descriptor_set = self
                .device
                .ui_overlay_descriptor_set(draw.texture_id.as_deref())
                .map_err(|error| {
                    vec![Diagnostic::new(
                        "RV0299",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        format!("cannot bind UI texture: {error}"),
                    )]
                })?;
            descriptor_sets.push(descriptor_set);
        }

        let frame_index = self.device.current_frame;
        let required_bytes = u64::try_from(prepared.vertex_bytes.len()).map_err(|_| {
            vec![Diagnostic::new(
                "RV0300",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "UI vertex data exceeds the Vulkan buffer size contract",
            )]
        })?;
        if self.ui_vb_capacities[frame_index] < required_bytes {
            if let Some(old) = self.ui_vbs[frame_index].take() {
                self.device.destroy_buffer(old);
            }
            let vertex_buffer = self
                .device
                .create_buffer(&BufferDescriptor {
                    size_bytes: required_bytes,
                    usage_flags: render_core::BufferUsage::VERTEX,
                    memory_hint: MemoryHint::CpuToGpu,
                    debug_label: Some(format!("ui-overlay-vb-{frame_index}")),
                })
                .map_err(|error| {
                    vec![Diagnostic::new(
                        "RV0216",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        format!("create UI vertex buffer: {error:?}"),
                    )]
                })?;
            self.ui_vbs[frame_index] = Some(vertex_buffer);
            self.ui_vb_capacities[frame_index] = required_bytes;
        }
        let vertex_buffer = self.ui_vbs[frame_index].ok_or_else(|| {
            vec![Diagnostic::new(
                "RV0301",
                DiagnosticSeverity::Fatal,
                "scene_renderer",
                "UI vertex buffer was not retained after creation",
            )]
        })?;
        self.device
            .write_buffer(vertex_buffer, &prepared.vertex_bytes, 0)
            .map_err(|error| {
                vec![Diagnostic::new(
                    "RV0217",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("write UI vertex buffer: {error:?}"),
                )]
            })?;

        let raw_vertex_buffer = self
            .device
            .buffers
            .get(vertex_buffer.index, vertex_buffer.generation)
            .map(|entry| entry.buffer)
            .ok_or_else(|| {
                vec![Diagnostic::new(
                    "RV0302",
                    DiagnosticSeverity::Fatal,
                    "scene_renderer",
                    "UI vertex buffer handle became invalid before recording",
                )]
            })?;
        let render_pass = self.device.ui_overlay_rp.ok_or_else(|| {
            vec![Diagnostic::new(
                "RV0303",
                DiagnosticSeverity::Fatal,
                "scene_renderer",
                "UI overlay render pass is not initialized",
            )]
        })?;
        let pipeline = self.device.ui_overlay_pipeline.ok_or_else(|| {
            vec![Diagnostic::new(
                "RV0304",
                DiagnosticSeverity::Fatal,
                "scene_renderer",
                "UI overlay pipeline is not initialized",
            )]
        })?;
        let pipeline_layout = self.device.ui_overlay_pipeline_layout.ok_or_else(|| {
            vec![Diagnostic::new(
                "RV0305",
                DiagnosticSeverity::Fatal,
                "scene_renderer",
                "UI overlay pipeline layout is not initialized",
            )]
        })?;
        let framebuffer = self
            .device
            .ui_overlay_framebuffers
            .get(self.cur_fb_index as usize)
            .copied()
            .ok_or_else(|| {
                vec![Diagnostic::new(
                    "RV0306",
                    DiagnosticSeverity::Fatal,
                    "scene_renderer",
                    "UI overlay framebuffer is missing for the acquired swapchain image",
                )]
            })?;
        let command_buffer = self
            .device
            .frame_sync
            .get(frame_index)
            .map(|frame| frame.command_buffer)
            .ok_or_else(|| {
                vec![Diagnostic::new(
                    "RV0307",
                    DiagnosticSeverity::Fatal,
                    "scene_renderer",
                    "UI overlay command buffer is unavailable",
                )]
            })?;
        let d = &self.device.logical_device.device;
        let begin_info = vk::RenderPassBeginInfo::default()
            .render_pass(render_pass)
            .framebuffer(framebuffer)
            .render_area(vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: vk::Extent2D {
                    width: self.width,
                    height: self.height,
                },
            });
        unsafe {
            d.cmd_begin_render_pass(command_buffer, &begin_info, vk::SubpassContents::INLINE);
            d.cmd_set_viewport(
                command_buffer,
                0,
                &[vk::Viewport {
                    x: 0.0,
                    y: 0.0,
                    width: self.width as f32,
                    height: self.height as f32,
                    min_depth: 0.0,
                    max_depth: 1.0,
                }],
            );
            d.cmd_bind_pipeline(command_buffer, vk::PipelineBindPoint::GRAPHICS, pipeline);
            d.cmd_bind_vertex_buffers(command_buffer, 0, &[raw_vertex_buffer], &[0]);
            let mut screen_size = [0u8; 8];
            screen_size[..4].copy_from_slice(&(self.width as f32).to_ne_bytes());
            screen_size[4..].copy_from_slice(&(self.height as f32).to_ne_bytes());
            d.cmd_push_constants(
                command_buffer,
                pipeline_layout,
                vk::ShaderStageFlags::VERTEX,
                0,
                &screen_size,
            );
            for (draw, descriptor_set) in prepared.draws.iter().zip(descriptor_sets) {
                d.cmd_set_scissor(
                    command_buffer,
                    0,
                    &[vk::Rect2D {
                        offset: vk::Offset2D {
                            x: draw.scissor.x,
                            y: draw.scissor.y,
                        },
                        extent: vk::Extent2D {
                            width: draw.scissor.width,
                            height: draw.scissor.height,
                        },
                    }],
                );
                d.cmd_bind_descriptor_sets(
                    command_buffer,
                    vk::PipelineBindPoint::GRAPHICS,
                    pipeline_layout,
                    0,
                    &[descriptor_set],
                    &[],
                );
                d.cmd_draw(command_buffer, draw.vertex_count, 1, draw.first_vertex, 0);
                stats.draw_calls = stats.draw_calls.saturating_add(1);
                stats.triangles = stats
                    .triangles
                    .saturating_add(u64::from(draw.vertex_count / 3));
            }
            d.cmd_end_render_pass(command_buffer);
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // Per-pass GPU timestamps (ENG-04)
    // ------------------------------------------------------------------

    /// Evaluate device support once and configure the profiler.
    fn configure_gpu_timestamps(&mut self) {
        if self.gpu_timing_configured {
            return;
        }
        self.gpu_timing_configured = true;
        if !self.gpu_timing_enabled {
            self.gpu_timestamps
                .configure(crate::timestamps::TimestampSupport::Disabled, 0);
            return;
        }
        let limits = &self.device.adapter.properties.limits;
        let support = crate::timestamps::evaluate_support(
            true,
            limits.timestamp_compute_and_graphics == vk::TRUE,
            limits.timestamp_period,
        );
        let slots = self.device.frame_sync.len().max(1);
        self.gpu_timestamps.configure(support, slots);
    }

    /// Read back the slot's previous frame (its fence was just waited inside
    /// the device `begin_frame`), then reset the pool and start recording.
    fn gpu_timestamps_begin_frame(&mut self, frame_index: u64) {
        self.configure_gpu_timestamps();
        let fi = self.device.current_frame;
        let device = self.device.logical_device.device.clone();
        if self.gpu_timestamps.readback_len(fi).is_some() {
            let ticks = self.timestamp_pools.read(&device, fi);
            self.gpu_timestamps.deliver_readback(fi, ticks.as_deref());
        }
        let Some(_) = self.gpu_timestamps.begin_recording(fi, frame_index) else {
            return;
        };
        if self.timestamp_pools.ensure_created(&device).is_err() {
            self.gpu_timestamps
                .degrade("timestamp query pool creation failed");
            return;
        }
        let cmd = self.device.frame_sync[fi].command_buffer;
        self.timestamp_pools.cmd_reset(&device, cmd, fi);
    }

    /// Record the start timestamp for `pass_name`; returns the query to write.
    fn gpu_timestamp_pass_start(&mut self, pass_name: &str) -> Option<(u32, usize)> {
        let fi = self.device.current_frame;
        self.gpu_timestamps
            .stamp_start(pass_name)
            .map(|query| (query, fi))
    }

    /// Record the end timestamp paired with the most recent start.
    fn gpu_timestamp_pass_end(&mut self) -> Option<(u32, usize)> {
        let fi = self.device.current_frame;
        self.gpu_timestamps.stamp_end().map(|query| (query, fi))
    }

    /// Close the frame's recording after submission and publish backend GPU
    /// timing into the frame statistics.
    fn gpu_timestamps_end_frame(&mut self, stats: &mut FrameStats) {
        self.gpu_timestamps.finish_recording();
        stats.gpu_timing = self.gpu_timestamps.status();
        if let Some(batch) = self.gpu_timestamps.take_latest() {
            stats.gpu_pass_frame_index = Some(batch.frame_index);
            stats.gpu_pass_times = batch.passes;
        }
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
// BackendRenderer implementation
// ============================================================================

impl BackendRenderer for SceneRenderer {
    fn configure_render_graph(
        &mut self,
        _input: &RenderFrameInput,
        graph: &mut engine_renderer::render_graph2::RenderGraph,
    ) -> Result<(), Vec<Diagnostic>> {
        apply_registered_custom_pass_declarations(&self.pass_registry, graph)
    }

    fn apply_pass_barriers(
        &mut self,
        _input: &RenderFrameInput,
        _pass: &render_graph2::PassNode,
        barriers: &[engine_renderer::render_graph2::CompiledBarrier],
    ) -> Result<(), Vec<Diagnostic>> {
        let fi = self.device.current_frame;
        self.device
            .apply_render_graph_barriers(fi, barriers)
            .map_err(|message| {
                vec![Diagnostic::new(
                    "RV0316",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    message,
                )]
            })
    }

    fn begin_frame(&mut self, input: &RenderFrameInput) -> Result<(), Vec<Diagnostic>> {
        validate_vulkan_frame_contract(input)?;
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
        let msaa = vk::SampleCountFlags::TYPE_1;
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
        // The device begin-frame already waited for this slot's in-flight
        // fence, so the previous frame recorded on the slot can be read back
        // without stalling; then the slot's pool is reset for this frame.
        self.gpu_timestamps_begin_frame(input.frame_index);
        Ok(())
    }

    fn execute_pass(
        &mut self,
        input: &RenderFrameInput,
        pass: &render_graph2::PassNode,
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
        //
        // Every pass is bracketed by GPU timestamp writes (start at
        // TOP_OF_PIPE, end at BOTTOM_OF_PIPE) into the current slot's pool.
        let stamp_device = self.device.logical_device.device.clone();
        let stamp_cmd = self.device.frame_sync[self.device.current_frame].command_buffer;
        if let Some((query, slot)) = self.gpu_timestamp_pass_start(pass.name) {
            self.timestamp_pools.cmd_write(
                &stamp_device,
                stamp_cmd,
                query,
                slot,
                vk::PipelineStageFlags::TOP_OF_PIPE,
            );
        }
        let pass_result = match pass.kind {
            render_graph2::PassKind::OpaquePbrForward => {
                self.execute_hdr_forward_pass(input, stats)
            }
            render_graph2::PassKind::DirectionalShadow => self.execute_shadow_pass(input, stats),
            render_graph2::PassKind::ToneMap => self.execute_tonemap_pass(input, stats),
            render_graph2::PassKind::Present => {
                self.execute_ui_overlay_pass(&input.ui_batches, stats)
            }
            render_graph2::PassKind::Custom(name) => {
                let enc = self.cur_enc.as_mut().expect("encoder checked above");
                execute_registered_custom_pass(
                    &mut self.pass_registry,
                    name,
                    input,
                    &mut **enc,
                    stats,
                )
            }
        };
        if let Some((query, slot)) = self.gpu_timestamp_pass_end() {
            self.timestamp_pools.cmd_write(
                &stamp_device,
                stamp_cmd,
                query,
                slot,
                vk::PipelineStageFlags::BOTTOM_OF_PIPE,
            );
        }
        pass_result
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
                        self.gpu_timestamps.abort_slot(self.device.current_frame);
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
                // The frame was submitted: its timestamp queries are now
                // pending asynchronous read-back. Publish whatever batch the
                // begin-frame read-back produced into the statistics.
                self.gpu_timestamps_end_frame(stats);
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
            vertex_format: upload.vertex_format,
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
        let descriptor_targets: Vec<(vk::DescriptorSet, u32)> = self
            .material_cache
            .values()
            .flat_map(|entry| {
                entry
                    .bound_texture_ids
                    .iter()
                    .enumerate()
                    .filter(|(_, bound)| *bound == &texture_id)
                    .map(|(index, _)| (entry.desc_set, MATERIAL_TEXTURE_BINDINGS[index]))
            })
            .chain(self.skinned_desc_cache.values().flat_map(|entry| {
                entry
                    .bound_texture_ids
                    .iter()
                    .enumerate()
                    .filter(|(_, bound)| *bound == &texture_id)
                    .map(|(index, _)| (entry.desc_set, MATERIAL_TEXTURE_BINDINGS[index]))
            }))
            .collect();
        let mut old_texture = self.device.textures.insert(texture_id.clone(), new_texture);
        let mut rebind_diagnostics = Vec::new();
        if let Err(error) = self
            .device
            .refresh_ui_overlay_texture_descriptor(&texture_id)
        {
            rebind_diagnostics.push(Diagnostic::new(
                "RV0311",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("failed to rebind replacement UI texture '{texture_id}': {error}"),
            ));
        }
        for (descriptor_set, binding) in descriptor_targets.iter().copied() {
            match self
                .device
                .bind_material_texture_at(&texture_id, binding, descriptor_set)
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
                if let Err(error) = self
                    .device
                    .refresh_ui_overlay_texture_descriptor(&texture_id)
                {
                    rebind_diagnostics.push(Diagnostic::new(
                        "RV0312",
                        DiagnosticSeverity::Fatal,
                        "scene_renderer",
                        format!(
                            "failed to restore UI texture '{texture_id}' descriptor after replacement rollback: {error}"
                        ),
                    ));
                }
                for (descriptor_set, binding) in descriptor_targets {
                    match self
                        .device
                        .bind_material_texture_at(&texture_id, binding, descriptor_set)
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
        for texture in upload.texture_references().into_iter().flatten() {
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
                let material_keys: Vec<(String, usize)> = self
                    .material_cache
                    .iter()
                    .flat_map(|(key, entry)| {
                        entry
                            .bound_texture_ids
                            .iter()
                            .enumerate()
                            .filter(|(_, bound)| bound.as_str() == resource_id)
                            .map(|(index, _)| (key.clone(), index))
                    })
                    .collect();
                let skinned_keys: Vec<(String, usize)> = self
                    .skinned_desc_cache
                    .iter()
                    .flat_map(|(key, entry)| {
                        entry
                            .bound_texture_ids
                            .iter()
                            .enumerate()
                            .filter(|(_, bound)| bound.as_str() == resource_id)
                            .map(|(index, _)| (key.clone(), index))
                    })
                    .collect();
                for (key, index) in &material_keys {
                    let descriptor_set = self.material_cache[key].desc_set;
                    let bound = self
                        .device
                        .bind_material_texture_at(
                            FALLBACK_MATERIAL_TEXTURE_ID,
                            MATERIAL_TEXTURE_BINDINGS[*index],
                            descriptor_set,
                        )
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
                for (key, index) in &skinned_keys {
                    let descriptor_set = self.skinned_desc_cache[key].desc_set;
                    let bound = self
                        .device
                        .bind_material_texture_at(
                            FALLBACK_MATERIAL_TEXTURE_ID,
                            MATERIAL_TEXTURE_BINDINGS[*index],
                            descriptor_set,
                        )
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
                for (key, index) in material_keys {
                    if let Some(entry) = self.material_cache.get_mut(&key) {
                        entry.bound_texture_ids[index] = FALLBACK_MATERIAL_TEXTURE_ID.to_owned();
                    }
                }
                for (key, index) in skinned_keys {
                    if let Some(entry) = self.skinned_desc_cache.get_mut(&key) {
                        entry.bound_texture_ids[index] = FALLBACK_MATERIAL_TEXTURE_ID.to_owned();
                    }
                }
                self.device
                    .release_ui_overlay_texture_descriptor(&resource_id)
                    .map_err(|error| {
                        vec![Diagnostic::new(
                            "RV0313",
                            DiagnosticSeverity::Error,
                            "scene_renderer",
                            format!(
                                "failed to release UI descriptor for texture '{resource_id}': {error}"
                            ),
                        )]
                    })?;
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
        // The aborted frame's command buffer is reset without submission, so
        // its timestamp queries can never be read back; drop the slot state.
        self.gpu_timestamps.abort_slot(self.device.current_frame);
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

    fn set_gpu_timing_enabled(&mut self, enabled: bool) {
        self.gpu_timing_enabled = enabled;
        // Re-evaluate device support on the next frame so a runtime toggle
        // takes effect without recreating the renderer.
        self.gpu_timing_configured = false;
    }
}

impl Drop for SceneRenderer {
    fn drop(&mut self) {
        // Query pools are device-owned objects; wait for in-flight work so
        // pools are never destroyed while queries are still being written,
        // then destroy them before the logical device goes away.
        self.device.wait_idle();
        self.timestamp_pools
            .destroy(&self.device.logical_device.device);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_renderer::{RenderableItem, SkinnedItem};
    use render_core::PipelineHandle;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn surface_upload(
        transparency: engine_renderer::Transparency,
        double_sided: bool,
    ) -> MaterialUpload {
        MaterialUpload {
            material_id: AssetId::new("material.surface"),
            base_color: [0.2, 0.3, 0.4, 0.6],
            metallic: 0.1,
            roughness: 0.7,
            ambient_occlusion: 0.8,
            emissive: [0.05, 0.1, 0.15],
            base_color_texture: None,
            normal_texture: None,
            metallic_roughness_texture: None,
            occlusion_texture: None,
            emissive_texture: None,
            transparency,
            double_sided,
            content_hash: [7; 32],
        }
    }

    #[test]
    fn uploaded_material_binding_preserves_surface_state_and_mask_cutoff() {
        let binding = uploaded_material_binding(&surface_upload(
            engine_renderer::Transparency::Masked { cutoff: 0.37 },
            true,
        ));
        assert_eq!(
            binding.transparency,
            engine_renderer::Transparency::Masked { cutoff: 0.37 }
        );
        assert!(binding.double_sided);
        assert_eq!(
            SceneRenderer::parse_material_ubo(&binding.uniforms.bytes).alpha_cutoff,
            0.37
        );
        assert_eq!(
            SceneRenderer::parse_material_ubo(&binding.uniforms.bytes).emissive,
            [0.05, 0.1, 0.15, 0.0]
        );
        assert_eq!(binding.uniforms.bytes.len(), MATERIAL_UBO_SIZE);

        let blended =
            uploaded_material_binding(&surface_upload(engine_renderer::Transparency::Blend, false));
        assert_eq!(
            SceneRenderer::parse_material_ubo(&blended.uniforms.bytes).alpha_cutoff,
            -1.0
        );
    }

    #[test]
    fn uploaded_material_binding_preserves_all_pbr_texture_slots_and_flags() {
        let mut upload = surface_upload(engine_renderer::Transparency::Opaque, false);
        upload.base_color_texture = Some(AssetId::new("base"));
        upload.normal_texture = Some(AssetId::new("normal"));
        upload.metallic_roughness_texture = Some(AssetId::new("metallic-roughness"));
        upload.occlusion_texture = Some(AssetId::new("occlusion"));
        upload.emissive_texture = Some(AssetId::new("emissive"));
        let binding = uploaded_material_binding(&upload);
        assert_eq!(
            binding
                .textures
                .iter()
                .map(|slot| (slot.binding, slot.texture.id.as_str()))
                .collect::<Vec<_>>(),
            vec![
                (1, "base"),
                (3, "normal"),
                (4, "metallic-roughness"),
                (5, "occlusion"),
                (6, "emissive"),
            ]
        );
        assert_eq!(
            SceneRenderer::parse_material_ubo(&binding.uniforms.bytes).emissive[3],
            31.0
        );
        assert_eq!(SceneRenderer::material_texture_flags(&binding), 31.0);
    }

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

        fn destroy_buffer(&mut self, _buffer: BufferHandle) {}

        fn destroy_texture(&mut self, _texture: render_core::TextureHandle) {}

        fn destroy_shader_module(&mut self, _module: ShaderModuleHandle) {}

        fn destroy_render_pass(&mut self, _pass: RenderPassHandle) {}

        fn destroy_framebuffer(&mut self, _framebuffer: FramebufferHandle) {}

        fn destroy_pipeline_layout(&mut self, _layout: PipelineLayoutHandle) {}

        fn destroy_swapchain(&mut self, _swapchain: SwapchainHandle) {}

        fn destroy_surface(&mut self, _surface: render_core::SurfaceHandle) {}

        fn wait_idle(&self) {}
    }

    #[derive(Default)]
    struct MockEncoder;

    impl CommandEncoder for MockEncoder {
        fn begin_render_pass(
            &mut self,
            _render_pass: RenderPassHandle,
            _framebuffer: FramebufferHandle,
            _area: (u32, u32, u32, u32),
            _clear_color: [f32; 4],
            _clear_depth: Option<f32>,
        ) {
        }

        fn bind_pipeline(&mut self, _pipeline: PipelineHandle) {}

        fn bind_vertex_buffers(&mut self, _buffers: &[BufferHandle], _offsets: &[u64]) {}

        fn bind_index_buffer(
            &mut self,
            _buffer: BufferHandle,
            _offset: u64,
            _index_format: IndexFormat,
        ) {
        }

        fn bind_descriptor_sets(
            &mut self,
            _pipeline_layout: PipelineLayoutHandle,
            _first_set: u32,
            _sets: &[render_core::DescriptorSetHandle],
            _dynamic_offsets: &[u32],
        ) -> Result<(), render_core::RhiError> {
            Ok(())
        }

        fn set_viewport(
            &mut self,
            _x: f32,
            _y: f32,
            _w: f32,
            _h: f32,
            _min_depth: f32,
            _max_depth: f32,
        ) {
        }

        fn set_scissor(&mut self, _x: i32, _y: i32, _w: u32, _h: u32) {}

        fn draw(
            &mut self,
            _vertex_count: u32,
            _instance_count: u32,
            _first_vertex: u32,
            _first_instance: u32,
        ) {
        }

        fn draw_indexed(
            &mut self,
            _index_count: u32,
            _instance_count: u32,
            _first_index: u32,
            _vertex_offset: i32,
            _first_instance: u32,
        ) {
        }

        fn end_render_pass(&mut self) {}

        fn push_constants(
            &mut self,
            _pipeline_layout: PipelineLayoutHandle,
            _stage_flags: u32,
            _offset: u32,
            _data: &[u8],
        ) {
        }
    }

    struct CountingPass {
        kind: &'static str,
        prepare_count: Arc<AtomicUsize>,
        execute_count: Arc<AtomicUsize>,
        fail_prepare: bool,
        reads_depth: bool,
        writes_swapchain: bool,
    }

    impl CountingPass {
        fn new(
            kind: &'static str,
            prepare_count: Arc<AtomicUsize>,
            execute_count: Arc<AtomicUsize>,
        ) -> Self {
            Self {
                kind,
                prepare_count,
                execute_count,
                fail_prepare: false,
                reads_depth: false,
                writes_swapchain: false,
            }
        }

        fn with_declared_resources(mut self) -> Self {
            self.reads_depth = true;
            self.writes_swapchain = true;
            self
        }

        fn failing(
            kind: &'static str,
            prepare_count: Arc<AtomicUsize>,
            execute_count: Arc<AtomicUsize>,
        ) -> Self {
            Self {
                kind,
                prepare_count,
                execute_count,
                fail_prepare: true,
                reads_depth: false,
                writes_swapchain: false,
            }
        }
    }

    impl RenderPass for CountingPass {
        fn kind(&self) -> &'static str {
            self.kind
        }

        fn declare(&self, view_id: u32) -> render_graph2::PassNode {
            let inputs = self
                .reads_depth
                .then(|| render_graph2::PassAttachment {
                    name: "depth_stencil".into(),
                    format: Some("D32".into()),
                    clear: false,
                    load_op: "load".into(),
                    size_source: render_graph2::SizeSource::Swapchain,
                    access: render_graph2::ResourceAccess::Read,
                })
                .into_iter()
                .collect();
            let outputs = self
                .writes_swapchain
                .then(|| render_graph2::PassAttachment {
                    name: "swapchain".into(),
                    format: None,
                    clear: false,
                    load_op: "load".into(),
                    size_source: render_graph2::SizeSource::Swapchain,
                    access: render_graph2::ResourceAccess::Write,
                })
                .into_iter()
                .collect();
            render_graph2::PassNode {
                kind: render_graph2::PassKind::Custom(self.kind),
                name: self.kind,
                view_id,
                inputs,
                outputs,
                depth_stencil: None,
            }
        }

        fn prepare(&mut self, _device: &mut dyn Device) -> Result<(), Vec<Diagnostic>> {
            self.prepare_count.fetch_add(1, Ordering::SeqCst);
            if self.fail_prepare {
                Err(vec![Diagnostic::new(
                    "TEST_PREPARE",
                    DiagnosticSeverity::Error,
                    "test",
                    "custom pass preparation failed",
                )])
            } else {
                Ok(())
            }
        }

        fn execute(
            &mut self,
            _input: &RenderFrameInput,
            _encoder: &mut dyn CommandEncoder,
            stats: &mut FrameStats,
        ) -> Result<(), Vec<Diagnostic>> {
            self.execute_count.fetch_add(1, Ordering::SeqCst);
            stats.draw_calls = stats.draw_calls.saturating_add(1);
            Ok(())
        }
    }

    fn frame_with_custom_pass(kind: &str) -> RenderFrameInput {
        let mut input = RenderFrameInput::empty(7);
        input.views.push(engine_renderer::RenderView {
            view_id: 0,
            camera_entity: None,
            viewport: engine_renderer::Rect::FULL,
            viewport_rect_normalized: engine_renderer::Rect::FULL,
            view_matrix: engine_renderer::IDENTITY_MAT4,
            projection_matrix: engine_renderer::IDENTITY_MAT4,
            clear_flags: engine_renderer::ClearFlags::ColorAndDepth,
            clear_color: [0.0, 0.0, 0.0, 1.0],
            render_layer_mask: u32::MAX,
            msaa_samples: 1,
            compose: engine_renderer::ViewCompose::Base {
                clear: engine_renderer::ClearFlags::ColorAndDepth,
                clear_color: [0.0, 0.0, 0.0, 1.0],
            },
            stack_order: 0,
            frustum: None,
        });
        input.render_options.pass_graph_config = engine_renderer::PassGraphConfig {
            passes: vec![
                engine_renderer::PassConfigEntry {
                    kind: kind.to_string(),
                    enabled: true,
                },
                engine_renderer::PassConfigEntry {
                    kind: "Present".to_string(),
                    enabled: true,
                },
            ],
            enabled: true,
            output_mode: engine_renderer::PassGraphOutputMode::HdrThenToneMap,
        };
        input
    }

    #[test]
    fn configured_custom_pass_is_prepared_once_and_executed() {
        let prepare_count = Arc::new(AtomicUsize::new(0));
        let execute_count = Arc::new(AtomicUsize::new(0));
        let mut registry = PassRegistry::new();
        let mut device = MockDevice::new();
        prepare_and_register_custom_pass(
            &mut registry,
            &mut device,
            Box::new(CountingPass::new(
                "custom_post",
                Arc::clone(&prepare_count),
                Arc::clone(&execute_count),
            )),
        )
        .expect("custom pass registration");

        let input = frame_with_custom_pass("custom_post");
        let mut graph = engine_renderer::render_graph2::RenderGraph::build_with_config(
            &input,
            &input.render_options.pass_graph_config,
        );
        apply_registered_custom_pass_declarations(&registry, &mut graph)
            .expect("custom pass declaration");
        let compiled = graph.compile().expect("custom render graph compile");
        let mut encoder = MockEncoder;
        let mut stats = FrameStats::default();
        let mut executed_custom_node = false;
        for pass_index in compiled.pass_order {
            let pass = &graph.passes[pass_index];
            if let engine_renderer::render_graph2::PassKind::Custom(name) = pass.kind {
                execute_registered_custom_pass(
                    &mut registry,
                    name,
                    &input,
                    &mut encoder,
                    &mut stats,
                )
                .expect("registered custom pass execution");
                executed_custom_node = true;
            }
        }

        assert!(executed_custom_node);
        assert_eq!(prepare_count.load(Ordering::SeqCst), 1);
        assert_eq!(execute_count.load(Ordering::SeqCst), 1);
        assert_eq!(stats.draw_calls, 1);
    }

    #[test]
    fn registered_custom_pass_declaration_populates_graph_resources() {
        let mut registry = PassRegistry::new();
        let mut device = MockDevice::new();
        prepare_and_register_custom_pass(
            &mut registry,
            &mut device,
            Box::new(
                CountingPass::new(
                    "custom_composite",
                    Arc::new(AtomicUsize::new(0)),
                    Arc::new(AtomicUsize::new(0)),
                )
                .with_declared_resources(),
            ),
        )
        .expect("custom pass registration");

        let input = frame_with_custom_pass("custom_composite");
        let mut graph = engine_renderer::render_graph2::RenderGraph::build_with_config(
            &input,
            &input.render_options.pass_graph_config,
        );
        apply_registered_custom_pass_declarations(&registry, &mut graph)
            .expect("custom pass declaration");
        let custom = graph
            .passes
            .iter()
            .find(|node| {
                matches!(
                    &node.kind,
                    engine_renderer::render_graph2::PassKind::Custom(name)
                        if *name == "custom_composite"
                )
            })
            .expect("custom graph node");

        assert_eq!(custom.inputs.len(), 1);
        assert_eq!(custom.inputs[0].name, "depth_stencil");
        assert_eq!(
            custom.inputs[0].access,
            engine_renderer::render_graph2::ResourceAccess::Read
        );
        assert_eq!(custom.outputs.len(), 1);
        assert_eq!(custom.outputs[0].name, "swapchain");
        assert_eq!(
            custom.outputs[0].access,
            engine_renderer::render_graph2::ResourceAccess::Write
        );
    }

    #[test]
    fn duplicate_custom_pass_is_rejected_before_prepare() {
        let first_prepares = Arc::new(AtomicUsize::new(0));
        let duplicate_prepares = Arc::new(AtomicUsize::new(0));
        let execute_count = Arc::new(AtomicUsize::new(0));
        let mut registry = PassRegistry::new();
        let mut device = MockDevice::new();
        prepare_and_register_custom_pass(
            &mut registry,
            &mut device,
            Box::new(CountingPass::new(
                "custom_post",
                Arc::clone(&first_prepares),
                Arc::clone(&execute_count),
            )),
        )
        .expect("first registration");

        let diagnostics = prepare_and_register_custom_pass(
            &mut registry,
            &mut device,
            Box::new(CountingPass::new(
                "custom_post",
                Arc::clone(&duplicate_prepares),
                Arc::clone(&execute_count),
            )),
        )
        .expect_err("duplicate registration must fail");

        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "RV0299"));
        assert_eq!(first_prepares.load(Ordering::SeqCst), 1);
        assert_eq!(duplicate_prepares.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn failed_custom_pass_prepare_does_not_register_the_pass() {
        let prepare_count = Arc::new(AtomicUsize::new(0));
        let execute_count = Arc::new(AtomicUsize::new(0));
        let mut registry = PassRegistry::new();
        let mut device = MockDevice::new();

        let diagnostics = prepare_and_register_custom_pass(
            &mut registry,
            &mut device,
            Box::new(CountingPass::failing(
                "custom_post",
                Arc::clone(&prepare_count),
                execute_count,
            )),
        )
        .expect_err("prepare failure must fail registration");

        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "TEST_PREPARE"));
        assert_eq!(prepare_count.load(Ordering::SeqCst), 1);
        assert!(registry.find("custom_post").is_none());
    }

    #[test]
    fn configured_unregistered_custom_pass_still_fails_closed() {
        let input = frame_with_custom_pass("missing_post");
        let mut registry = PassRegistry::new();
        let mut encoder = MockEncoder;
        let mut stats = FrameStats::default();

        let diagnostics = execute_registered_custom_pass(
            &mut registry,
            "missing_post",
            &input,
            &mut encoder,
            &mut stats,
        )
        .expect_err("unregistered custom pass must fail");

        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "RV0291"));
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

    fn ui_batch(texture: Option<&str>, clip_rect: engine_renderer::Rect) -> UiBatch {
        UiBatch {
            canvas_id: "editor".into(),
            z_order: 0,
            clip_rect,
            texture: texture.map(AssetId::new),
            vertices: vec![
                engine_renderer::UiVertex {
                    position: [10.0, 20.0],
                    uv: [0.0, 0.0],
                    color: [255, 128, 0, 255],
                },
                engine_renderer::UiVertex {
                    position: [20.0, 20.0],
                    uv: [1.0, 0.0],
                    color: [255, 128, 0, 255],
                },
                engine_renderer::UiVertex {
                    position: [20.0, 30.0],
                    uv: [1.0, 1.0],
                    color: [255, 128, 0, 255],
                },
            ],
            indices: vec![0, 1, 2],
            material: AssetId::new("ui-material"),
        }
    }

    #[test]
    fn ui_preparation_keeps_batch_order_and_one_draw_per_batch() {
        let clip = engine_renderer::Rect {
            min: [0.0, 0.0],
            max: [200.0, 100.0],
        };
        let batches = vec![ui_batch(Some("first"), clip), ui_batch(None, clip)];

        let prepared = prepare_ui_overlay(&batches, 200, 100).unwrap();

        assert_eq!(prepared.draws.len(), 2);
        assert_eq!(prepared.draws[0].texture_id.as_deref(), Some("first"));
        assert_eq!(prepared.draws[1].texture_id, None);
        assert_eq!(prepared.draws[0].first_vertex, 0);
        assert_eq!(prepared.draws[1].first_vertex, 3);
        assert_eq!(prepared.vertex_bytes.len(), 6 * UI_VERTEX_STRIDE);
    }

    #[test]
    fn empty_ui_preparation_has_no_overlay_draws() {
        let prepared = prepare_ui_overlay(&[], 1280, 720).unwrap();

        assert!(prepared.draws.is_empty());
        assert!(prepared.vertex_bytes.is_empty());
    }

    #[test]
    fn ui_preparation_clamps_fractional_clip_to_the_swapchain() {
        let batch = ui_batch(
            None,
            engine_renderer::Rect {
                min: [-3.4, 8.2],
                max: [500.7, 120.1],
            },
        );

        let prepared = prepare_ui_overlay(&[batch], 320, 100).unwrap();

        assert_eq!(
            prepared.draws[0].scissor,
            UiScissor {
                x: 0,
                y: 8,
                width: 320,
                height: 92,
            }
        );
    }

    #[test]
    fn ui_preparation_rejects_an_out_of_bounds_index() {
        let mut batch = ui_batch(
            None,
            engine_renderer::Rect {
                min: [0.0, 0.0],
                max: [100.0, 100.0],
            },
        );
        batch.indices[2] = 99;

        let error = prepare_ui_overlay(&[batch], 100, 100).unwrap_err();

        assert!(error.contains("index 99"));
        assert!(error.contains("outside 3 vertices"));
    }

    #[test]
    fn missing_ui_texture_check_ignores_textureless_batches() {
        let clip = engine_renderer::Rect {
            min: [0.0, 0.0],
            max: [100.0, 100.0],
        };
        let batches = vec![ui_batch(None, clip), ui_batch(Some("missing"), clip)];

        assert_eq!(
            first_missing_ui_texture(&batches, |id| id == "known"),
            Some("missing")
        );
        assert_eq!(first_missing_ui_texture(&batches, |_| true), None);
    }

    #[test]
    fn ui_fragment_shader_multiplies_texture_and_vertex_color() {
        let source = include_str!("../shaders/ui_overlay.frag");
        assert!(source.contains("texture(ui_texture, out_uv) * out_color"));
    }

    #[test]
    fn ui_vertex_shader_preserves_top_left_editor_coordinates() {
        let source = include_str!("../shaders/ui_overlay.vert");
        assert!(source.contains("float y = (in_position.y / pc.screen_size.y) * 2.0 - 1.0;"));
        assert!(!source.contains("float y = -(in_position.y / pc.screen_size.y) * 2.0 + 1.0;"));
    }

    #[test]
    fn skybox_shaders_generate_a_cube_and_sample_the_environment() {
        let vertex = include_str!("../shaders/skybox.vert");
        let fragment = include_str!("../shaders/skybox.frag");
        assert!(vertex.contains("CUBE_POSITIONS[gl_VertexIndex]"));
        assert!(vertex.contains("vec4(direction, 0.0)"));
        assert!(fragment.contains("samplerCube u_environment_map"));
        assert!(fragment.contains("texture(u_environment_map"));
    }

    #[test]
    fn vulkan_scene_renderer_rejects_direct_to_swapchain() {
        let diagnostics =
            validate_vulkan_output_mode(engine_renderer::PassGraphOutputMode::DirectToSwapchain)
                .unwrap_err();

        assert_eq!(diagnostics[0].code, "RV0310");
        assert!(diagnostics[0].message.contains("DirectToSwapchain"));
        assert!(
            validate_vulkan_output_mode(engine_renderer::PassGraphOutputMode::HdrThenToneMap)
                .is_ok()
        );
    }

    #[test]
    fn vulkan_scene_renderer_fails_closed_for_unimplemented_view_options() {
        let mut input = frame_with_custom_pass("custom_post");
        assert!(validate_vulkan_frame_contract(&input).is_ok());

        input.render_options.msaa_samples = 4;
        assert_eq!(
            validate_vulkan_frame_contract(&input).unwrap_err()[0].code,
            "RV0317"
        );
        input.render_options.msaa_samples = 1;

        let embedded = engine_renderer::Rect {
            min: [0.25, 0.125],
            max: [0.75, 0.875],
        };
        input.views[0].viewport = embedded;
        input.views[0].viewport_rect_normalized = embedded;
        assert!(validate_vulkan_frame_contract(&input).is_ok());

        input.views[0].viewport_rect_normalized.max = [0.5, 1.0];
        assert_eq!(
            validate_vulkan_frame_contract(&input).unwrap_err()[0].code,
            "RV0318"
        );
        input.views[0].viewport = engine_renderer::Rect::FULL;
        input.views[0].viewport_rect_normalized = engine_renderer::Rect::FULL;

        input.views[0].clear_flags = engine_renderer::ClearFlags::Skybox;
        assert!(validate_vulkan_frame_contract(&input).is_ok());

        input.views[0].clear_flags = engine_renderer::ClearFlags::Nothing;
        assert_eq!(
            validate_vulkan_frame_contract(&input).unwrap_err()[0].code,
            "RV0319"
        );
    }

    #[test]
    fn normalized_scene_viewport_maps_to_matching_vulkan_viewport_and_scissor() {
        let mapped = vulkan_viewport_rect(
            engine_renderer::Rect {
                min: [0.25, 0.1],
                max: [0.75, 0.9],
            },
            1600,
            900,
        )
        .unwrap();
        assert_eq!(mapped.viewport.x, 400.0);
        assert_eq!(mapped.viewport.y, 90.0);
        assert_eq!(mapped.viewport.width, 800.0);
        assert_eq!(mapped.viewport.height, 720.0);
        assert_eq!(mapped.scissor.offset.x, 400);
        assert_eq!(mapped.scissor.offset.y, 90);
        assert_eq!(mapped.scissor.extent.width, 800);
        assert_eq!(mapped.scissor.extent.height, 720);

        let fractional = vulkan_viewport_rect(
            engine_renderer::Rect {
                min: [0.1, 0.1],
                max: [0.2, 0.2],
            },
            17,
            11,
        )
        .unwrap();
        assert_eq!(fractional.scissor.offset.x, 1);
        assert_eq!(fractional.scissor.offset.y, 1);
        assert_eq!(fractional.scissor.extent.width, 3);
        assert_eq!(fractional.scissor.extent.height, 2);

        assert!(vulkan_viewport_rect(engine_renderer::Rect::FULL, 0, 900).is_err());
    }

    #[test]
    fn tone_map_push_constants_cover_modes_exposure_and_target_encoding() {
        let aces = tone_map_push_constants(
            engine_renderer::ToneMapping::Aces,
            None,
            vk::Format::B8G8R8A8_SRGB,
        )
        .unwrap();
        assert_eq!(aces.mode, TONE_MAP_MODE_ACES);
        assert_eq!(aces.exposure, 1.0);
        assert_eq!(aces.output_is_srgb, 1);

        let reinhard = tone_map_push_constants(
            engine_renderer::ToneMapping::Reinhard,
            Some(2.0),
            vk::Format::B8G8R8A8_UNORM,
        )
        .unwrap();
        assert_eq!(reinhard.mode, TONE_MAP_MODE_REINHARD);
        assert_eq!(reinhard.exposure, 0.25);
        assert_eq!(reinhard.output_is_srgb, 0);

        let identity = tone_map_push_constants(
            engine_renderer::ToneMapping::None,
            Some(-1.0),
            vk::Format::R8G8B8A8_SRGB,
        )
        .unwrap();
        assert_eq!(identity.mode, TONE_MAP_MODE_NONE);
        assert_eq!(identity.exposure, 2.0);
        assert_eq!(identity.output_is_srgb, 1);

        let bytes = identity.to_bytes();
        assert_eq!(bytes.len(), ToneMapPushConstants::SIZE);
        assert_eq!(
            u32::from_ne_bytes(bytes[0..4].try_into().unwrap()),
            TONE_MAP_MODE_NONE
        );
        assert_eq!(f32::from_ne_bytes(bytes[4..8].try_into().unwrap()), 2.0);
        assert_eq!(u32::from_ne_bytes(bytes[8..12].try_into().unwrap()), 1);
        assert_eq!(u32::from_ne_bytes(bytes[12..16].try_into().unwrap()), 0);
    }

    #[test]
    fn tone_map_push_constants_reject_non_finite_exposure() {
        for exposure in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let error = tone_map_push_constants(
                engine_renderer::ToneMapping::Aces,
                Some(exposure),
                vk::Format::B8G8R8A8_SRGB,
            )
            .unwrap_err();
            assert!(error.contains("must be finite"));
        }

        let overflow = tone_map_push_constants(
            engine_renderer::ToneMapping::Aces,
            Some(-1000.0),
            vk::Format::B8G8R8A8_SRGB,
        )
        .unwrap_err();
        assert!(overflow.contains("non-finite exposure multiplier"));
    }

    #[test]
    fn tone_map_fragment_shader_declares_all_runtime_branches() {
        let source = include_str!("../shaders/tonemap.frag");
        assert!(source.contains("layout(push_constant)"));
        assert!(source.contains("aces_narkowicz"));
        assert!(source.contains("TONE_MAP_REINHARD"));
        assert!(source.contains("TONE_MAP_NONE"));
        assert!(source.contains("tone_map.output_is_srgb == 0u"));
    }
}
