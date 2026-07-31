use super::*;

/// GPU-side representation of a mesh: vertex buffer, index buffer and the
/// metadata needed to issue an indexed draw call.
#[derive(Clone, Debug)]
pub struct GpuMesh {
    pub vertex_buffer: BufferHandle,
    pub index_buffer: BufferHandle,
    pub index_count: u32,
    pub vertex_count: u32,
    pub index_format: IndexFormat,
    pub vertex_format: MeshVertexFormat,
    pub content_hash: [u8; 32],
    pub revision: u64,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct UploadedResourceState {
    pub(super) content_hash: [u8; 32],
    pub(super) revision: u64,
}

#[derive(Clone, Debug)]
pub(super) struct UploadedMaterialState {
    pub(super) binding: MaterialBinding,
    pub(super) content_hash: [u8; 32],
    pub(super) revision: u64,
}

#[inline]
pub(super) fn vulkan_index_type(format: IndexFormat) -> vk::IndexType {
    match format {
        IndexFormat::U16 => vk::IndexType::UINT16,
        IndexFormat::U32 => vk::IndexType::UINT32,
    }
}

pub(super) const SCENE_FORWARD_PIPELINE_ID: &str = "scene-forward";
pub(super) const BUILTIN_PASS_KINDS: [&str; 4] = [
    "directional_shadow_pass",
    "opaque_pbr_forward_pass",
    "tone_map_pass",
    "present",
];

pub(super) fn prepare_and_register_custom_pass(
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

pub(super) fn execute_registered_custom_pass(
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

pub(super) fn apply_registered_custom_pass_declarations(
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

pub(super) fn uploaded_material_binding(upload: &MaterialUpload) -> MaterialBinding {
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
            engine_renderer::Transparency::Opaque
            | engine_renderer::Transparency::Blend
            | engine_renderer::Transparency::Additive => -1.0,
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
    for vector in [
        [
            upload.advanced.clearcoat,
            upload.advanced.clearcoat_roughness,
            upload.advanced.subsurface,
            upload.advanced.anisotropy,
        ],
        [
            upload.advanced.subsurface_color[0],
            upload.advanced.subsurface_color[1],
            upload.advanced.subsurface_color[2],
            0.0,
        ],
        [
            upload.advanced.sheen_color[0],
            upload.advanced.sheen_color[1],
            upload.advanced.sheen_color[2],
            0.0,
        ],
        [
            upload.advanced.rim_color[0],
            upload.advanced.rim_color[1],
            upload.advanced.rim_color[2],
            upload.advanced.rim_power,
        ],
    ] {
        for value in vector {
            bytes.extend_from_slice(&value.to_ne_bytes());
        }
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

/// CPU-side material UBO layout (112 bytes total).
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
/// |     48 | advanced0    | vec4     |    16 |
/// |     64 | subsurface   | vec4     |    16 |
/// |     80 | sheen        | vec4     |    16 |
/// |     96 | rim          | vec4     |    16 |
/// Total: 112 bytes.
#[repr(C)]
pub(super) struct MaterialUBO {
    pub(super) base_color: [f32; 4],
    pub(super) metallic: f32,
    pub(super) roughness: f32,
    pub(super) ao: f32,
    pub(super) alpha_cutoff: f32,
    pub(super) emissive: [f32; 4],
    pub(super) advanced0: [f32; 4],
    pub(super) subsurface_color: [f32; 4],
    pub(super) sheen_color: [f32; 4],
    pub(super) rim_color_power: [f32; 4],
}

pub(super) const MATERIAL_UBO_SIZE: usize = std::mem::size_of::<MaterialUBO>();
pub(super) const MATERIAL_TEXTURE_BINDINGS: [u32; 5] = [1, 3, 4, 5, 6];

/// Cache entry for a material descriptor set + UBO buffer.
pub(super) struct MaterialCacheEntry {
    pub(super) desc_set: vk::DescriptorSet,
    pub(super) handle: BufferHandle,
    pub(super) buffer: vk::Buffer,
    pub(super) ubo_data: [u8; MATERIAL_UBO_SIZE],
    pub(super) bound_texture_ids: [String; 5],
}

pub(super) const MAX_MATERIALS: usize = 256;

/// Cache entry for a bone palette descriptor set.
pub(super) struct BonePaletteCacheEntry {
    pub(super) desc_set: vk::DescriptorSet,
    pub(super) bound_texture_ids: [String; 5],
}

/// Cached bone UBO buffer (handle for writes + raw VkBuffer for descriptor binding).
pub(super) struct CachedBoneBuffer {
    pub(super) handle: BufferHandle,
    pub(super) vk_buffer: vk::Buffer,
    pub(super) ubo_data: Vec<u8>,
}

pub(super) const MAX_BONE_PALETTES: usize = 64;

#[derive(Clone, Debug)]
pub(super) struct GpuMorphTargetSet {
    pub(super) handle: BufferHandle,
    pub(super) buffer: vk::Buffer,
    pub(super) vertex_count: u32,
    pub(super) target_count: u32,
    pub(super) content_hash: [u8; 32],
    pub(super) revision: u64,
}

pub(super) fn prepare_ui_overlay(
    batches: &[UiBatch],
    width: u32,
    height: u32,
) -> Result<PreparedUiOverlay, String> {
    prepare_shared_ui_overlay(batches, width, height).map_err(|error| error.to_string())
}

pub(super) fn validate_vulkan_output_mode(
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

pub(super) fn validate_vulkan_frame_contract(
    input: &RenderFrameInput,
) -> Result<(), Vec<Diagnostic>> {
    const OUTPUT_MODES: &[engine_renderer::PassGraphOutputMode] =
        &[engine_renderer::PassGraphOutputMode::HdrThenToneMap];
    const MSAA_SAMPLES: &[u8] = &[1, 2, 4, 8];
    const CLEAR_FLAGS: &[engine_renderer::ClearFlags] = &[
        engine_renderer::ClearFlags::ColorAndDepth,
        engine_renderer::ClearFlags::Skybox,
    ];
    let capabilities = BackendFrameCapabilities {
        allowed_output_modes: OUTPUT_MODES,
        allowed_msaa_samples: MSAA_SAMPLES,
        require_view: false,
        require_matching_view_msaa: true,
        require_matching_viewports: true,
        allowed_clear_flags: CLEAR_FLAGS,
    };
    match validate_backend_frame_contract(input, capabilities) {
        Ok(()) => Ok(()),
        Err(FrameContractViolation::UnsupportedOutputMode) => {
            validate_vulkan_output_mode(input.render_options.pass_graph_config.output_mode)
        }
        Err(FrameContractViolation::UnsupportedMsaa) => Err(vec![Diagnostic::new(
            "RV0317",
            DiagnosticSeverity::Error,
            "scene_renderer",
            "Vulkan MSAA must be 1, 2, 4, or 8 samples and match every render view",
        )]),
        Err(FrameContractViolation::InvalidViewport) => Err(vec![Diagnostic::new(
            "RV0318",
            DiagnosticSeverity::Error,
            "scene_renderer",
            "Vulkan SceneRenderer requires matching, valid normalized viewport rectangles",
        )]),
        Err(FrameContractViolation::UnsupportedClearMode) => Err(vec![Diagnostic::new(
            "RV0319",
            DiagnosticSeverity::Error,
            "scene_renderer",
            "Vulkan SceneRenderer currently supports only ColorAndDepth and Skybox clear modes",
        )]),
        Err(FrameContractViolation::MissingView) => unreachable!("Vulkan views are optional here"),
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct VulkanViewportRect {
    pub(super) viewport: vk::Viewport,
    pub(super) scissor: vk::Rect2D,
}

pub(super) fn vulkan_viewport_rect(
    rect: engine_renderer::Rect,
    surface_width: u32,
    surface_height: u32,
) -> Result<VulkanViewportRect, &'static str> {
    let prepared =
        prepare_normalized_viewport(rect, surface_width, surface_height).map_err(|error| {
            match error {
                engine_renderer::backend_shared::ViewportPlanError::InvalidNormalizedRect => {
                    "viewport must be finite, positive, and contained in [0, 1]"
                }
                engine_renderer::backend_shared::ViewportPlanError::ZeroSurface => {
                    "viewport surface dimensions must be positive"
                }
                engine_renderer::backend_shared::ViewportPlanError::SurfaceTooLarge => {
                    "viewport surface dimensions exceed Vulkan's signed offset range"
                }
            }
        })?;

    Ok(VulkanViewportRect {
        viewport: vk::Viewport {
            x: prepared.x,
            y: prepared.y,
            width: prepared.width,
            height: prepared.height,
            min_depth: 0.0,
            max_depth: 1.0,
        },
        scissor: vk::Rect2D {
            offset: vk::Offset2D {
                x: prepared.scissor.x,
                y: prepared.scissor.y,
            },
            extent: vk::Extent2D {
                width: prepared.scissor.width,
                height: prepared.scissor.height,
            },
        },
    })
}

pub(super) fn swapchain_format_is_srgb(format: vk::Format) -> bool {
    matches!(
        format,
        vk::Format::R8G8B8A8_SRGB | vk::Format::B8G8R8A8_SRGB | vk::Format::A8B8G8R8_SRGB_PACK32
    )
}

pub(super) fn tone_map_push_constants(
    tone_mapping: engine_renderer::ToneMapping,
    exposure_ev100: Option<f32>,
    post_process: engine_renderer::PostProcessSettings,
    swapchain_format: vk::Format,
    weighted_oit_resolve: bool,
) -> Result<ToneMapPushConstants, String> {
    prepare_tone_map_plan(
        tone_mapping,
        exposure_ev100,
        post_process,
        ToneMapPlanOptions {
            output_is_srgb: swapchain_format_is_srgb(swapchain_format),
            weighted_oit_resolve,
        },
    )
    .map_err(|error| error.to_string())
}

// ============================================================================
// SceneRenderer
// ============================================================================
