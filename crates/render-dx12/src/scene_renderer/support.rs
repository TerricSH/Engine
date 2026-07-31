use super::*;

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
#[derive(Clone, Debug)]
pub struct Dx12GpuMesh {
    pub vertex_buffer: BufferHandle,
    pub index_buffer: BufferHandle,
    pub index_count: u32,
    pub index_format: RhiIndexFormat,
    pub vertex_format: RendererMeshVertexFormat,
    pub vertex_count: u32,
    pub vertex_bytes: Vec<u8>,
    pub content_hash: [u8; 32],
    pub revision: u64,
}
#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
#[derive(Clone, Debug)]
pub(super) struct Dx12MaterialState {
    pub(super) constants: [u8; 32],
    pub(super) emissive_constants: [u8; 16],
    pub(super) advanced_constants: [u8; 16],
    pub(super) texture_ids: [Option<String>; 5],
    pub(super) transparency: Transparency,
    pub(super) double_sided: bool,
    pub(super) content_hash: [u8; 32],
    pub(super) revision: u64,
}
#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
pub(super) const MATERIAL_TEXTURE_BINDINGS: [u32; 5] = [1, 3, 4, 5, 6];

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
#[derive(Clone, Debug)]
pub(super) struct Dx12TextureState {
    pub(super) handle: TextureHandle,
    pub(super) content_hash: [u8; 32],
    pub(super) revision: u64,
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
#[derive(Clone, Debug)]
pub(super) struct Dx12EnvironmentState {
    pub(super) handle: TextureHandle,
    pub(super) mip_count: u32,
    pub(super) content_hash: [u8; 32],
    pub(super) revision: u64,
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
#[derive(Clone, Debug)]
pub(super) struct Dx12BoneBuffer {
    pub(super) handle: BufferHandle,
    pub(super) bytes: Vec<u8>,
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
#[derive(Clone, Debug)]
pub(super) struct Dx12DynamicVertexBuffer {
    pub(super) handle: BufferHandle,
    pub(super) bytes: Vec<u8>,
    pub(super) capacity: usize,
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
#[derive(Clone, Debug)]
pub(super) struct Dx12MorphTargetSet {
    pub(super) vertex_count: u32,
    pub(super) targets: Vec<engine_renderer::MorphTarget>,
    pub(super) content_hash: [u8; 32],
    pub(super) revision: u64,
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
#[derive(Clone, Copy, Debug)]
pub(crate) struct Dx12ShadowFrameData {
    pub(crate) light_view_projection: Mat4,
    pub(crate) light_direction_to_surface: glam::Vec3,
    pub(crate) soft: bool,
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
pub(super) struct Dx12FrameState {
    pub(super) image_index: u32,
    pub(super) encoder: Box<dyn CommandEncoder>,
    pub(super) draw_calls: u32,
    pub(super) triangles: u64,
    pub(super) visible_drawables: u32,
    pub(super) culled_drawables: u32,
    pub(super) visible_lights: u32,
    pub(super) culled_lights: u32,
    pub(super) hdr_pass_active: bool,
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
pub(super) fn prepare_dx12_ui(
    batches: &[engine_renderer::UiBatch],
    width: u32,
    height: u32,
) -> Result<PreparedUiOverlay, String> {
    prepare_ui_overlay(batches, width, height).map_err(|error| error.to_string())
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
pub(super) fn validate_dx12_frame_contract(
    input: &RenderFrameInput,
) -> Result<(), Vec<Diagnostic>> {
    const OUTPUT_MODES: &[engine_renderer::PassGraphOutputMode] =
        &[engine_renderer::PassGraphOutputMode::HdrThenToneMap];
    const MSAA_SAMPLES: &[u8] = &[1];
    const CLEAR_FLAGS: &[engine_renderer::ClearFlags] = &[
        engine_renderer::ClearFlags::ColorAndDepth,
        engine_renderer::ClearFlags::Skybox,
    ];
    let capabilities = BackendFrameCapabilities {
        allowed_output_modes: OUTPUT_MODES,
        allowed_msaa_samples: MSAA_SAMPLES,
        require_view: true,
        require_matching_view_msaa: true,
        require_matching_viewports: true,
        allowed_clear_flags: CLEAR_FLAGS,
    };
    match validate_backend_frame_contract(input, capabilities) {
        Ok(()) => Ok(()),
        Err(FrameContractViolation::UnsupportedOutputMode) => Err(vec![Diagnostic::new(
            "DX1247",
            DiagnosticSeverity::Error,
            "scene_renderer",
            "DX12 scene rendering uses the portable HdrThenToneMap graph; DirectToSwapchain bypasses required HDR composition",
        )]),
        Err(FrameContractViolation::MissingView) => Err(vec![Diagnostic::new(
            "DX1244",
            DiagnosticSeverity::Error,
            "scene_renderer",
            "the DX12 backend requires at least one render view per frame",
        )]),
        Err(FrameContractViolation::UnsupportedMsaa) => Err(vec![Diagnostic::new(
            "DX1249",
            DiagnosticSeverity::Error,
            "scene_renderer",
            "DX12 SceneRenderer does not yet implement multisample resolve; use 1x MSAA",
        )]),
        Err(FrameContractViolation::InvalidViewport) => Err(vec![Diagnostic::new(
            "DX1250",
            DiagnosticSeverity::Error,
            "scene_renderer",
            "DX12 render views require matching, valid normalized viewport rectangles",
        )]),
        Err(FrameContractViolation::UnsupportedClearMode) => Err(vec![Diagnostic::new(
            "DX1251",
            DiagnosticSeverity::Error,
            "scene_renderer",
            "DX12 SceneRenderer supports ColorAndDepth and Skybox clear modes",
        )]),
    }
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
pub struct Dx12SceneRenderer {
    pub(super) device: Dx12Device,
    pub(super) meshes: HashMap<String, Dx12GpuMesh>,
    pub(super) materials: HashMap<String, Dx12MaterialState>,
    pub(super) textures: HashMap<String, Dx12TextureState>,
    pub(super) environments: HashMap<String, Dx12EnvironmentState>,
    pub(super) fallback_environment: Option<TextureHandle>,
    pub(super) fallback_ui_texture: Option<TextureHandle>,
    pub(super) bone_buffers: HashMap<String, Dx12BoneBuffer>,
    pub(super) vertex_draw_buffer: Option<Dx12DynamicVertexBuffer>,
    pub(super) morphed_vertex_buffers: HashMap<String, Dx12DynamicVertexBuffer>,
    pub(super) morph_target_sets: HashMap<String, Dx12MorphTargetSet>,
    pub(super) particle_instance_buffers: HashMap<String, Dx12DynamicVertexBuffer>,
    pub(super) gpu_particle_parameter_buffers: HashMap<String, Dx12DynamicVertexBuffer>,
    pub(super) gpu_particle_dummy_buffer: Option<Dx12DynamicVertexBuffer>,
    pub(super) clustered_light_buffer: Option<Dx12DynamicVertexBuffer>,
    pub(super) clustered_grid_buffer: Option<Dx12DynamicVertexBuffer>,
    pub(super) clustered_index_buffer: Option<Dx12DynamicVertexBuffer>,
    pub(super) ui_vertex_buffer: Option<Dx12DynamicVertexBuffer>,
    // Revisions survive removal so recreating the same logical resource never
    // moves its receipt backwards.
    pub(super) mesh_revisions: HashMap<String, u64>,
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) swapchain: SwapchainHandle,
    pub(super) pipeline_layout: Option<PipelineLayoutHandle>,
    pub(super) pipeline: Option<PipelineHandle>,
    pub(super) double_sided_pipeline: Option<PipelineHandle>,
    pub(super) blend_pipeline: Option<PipelineHandle>,
    pub(super) blend_double_sided_pipeline: Option<PipelineHandle>,
    pub(super) oit_pipeline: Option<PipelineHandle>,
    pub(super) oit_double_sided_pipeline: Option<PipelineHandle>,
    pub(super) additive_pipeline: Option<PipelineHandle>,
    pub(super) additive_double_sided_pipeline: Option<PipelineHandle>,
    pub(super) skinned_pipeline: Option<PipelineHandle>,
    pub(super) skinned_double_sided_pipeline: Option<PipelineHandle>,
    pub(super) skinned_blend_pipeline: Option<PipelineHandle>,
    pub(super) skinned_blend_double_sided_pipeline: Option<PipelineHandle>,
    pub(super) skinned_oit_pipeline: Option<PipelineHandle>,
    pub(super) skinned_oit_double_sided_pipeline: Option<PipelineHandle>,
    pub(super) skinned_additive_pipeline: Option<PipelineHandle>,
    pub(super) skinned_additive_double_sided_pipeline: Option<PipelineHandle>,
    pub(super) particle_pipeline: Option<PipelineHandle>,
    pub(super) particle_additive_pipeline: Option<PipelineHandle>,
    pub(super) particle_oit_pipeline: Option<PipelineHandle>,
    pub(super) gpu_particle_pipeline: Option<PipelineHandle>,
    pub(super) gpu_particle_additive_pipeline: Option<PipelineHandle>,
    pub(super) gpu_particle_oit_pipeline: Option<PipelineHandle>,
    pub(super) skybox_pipeline: Option<PipelineHandle>,
    pub(super) hdr_texture: Option<TextureHandle>,
    pub(super) oit_accum_texture: Option<TextureHandle>,
    pub(super) oit_optical_depth_texture: Option<TextureHandle>,
    pub(super) hdr_depth_texture: Option<TextureHandle>,
    pub(super) hdr_render_pass: Option<RenderPassHandle>,
    pub(super) hdr_framebuffer: Option<FramebufferHandle>,
    pub(super) tone_map_pipeline_layout: Option<PipelineLayoutHandle>,
    pub(super) tone_map_pipeline: Option<PipelineHandle>,
    pub(super) ui_pipeline_layout: Option<PipelineLayoutHandle>,
    pub(super) ui_pipeline: Option<PipelineHandle>,
    pub(super) shadow_texture: Option<TextureHandle>,
    pub(super) shadow_render_pass: Option<RenderPassHandle>,
    pub(super) shadow_framebuffer: Option<FramebufferHandle>,
    pub(super) shadow_pipeline_layout: Option<PipelineLayoutHandle>,
    pub(super) shadow_pipeline: Option<PipelineHandle>,
    pub(super) skinned_shadow_pipeline: Option<PipelineHandle>,
    pub(super) shadow_frame_data: Option<Dx12ShadowFrameData>,
    pub(super) active_frame: Option<Dx12FrameState>,
    /// Any failure after handing a command list to `end_frame` makes allocator
    /// reuse ambiguous. Refuse subsequent frames until the backend is rebuilt.
    pub(super) fatal_frame_error: Option<String>,
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
pub(super) fn surface_variant_index(
    transparency: &Transparency,
    double_sided: bool,
    weighted_oit: bool,
) -> usize {
    usize::from(double_sided)
        + match transparency {
            Transparency::Blend if weighted_oit => 6,
            Transparency::Blend => 2,
            Transparency::Additive => 4,
            Transparency::Opaque | Transparency::Masked { .. } => 0,
        }
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
pub(super) fn create_surface_pipeline_variants(
    device: &mut Dx12Device,
    base: &PipelineDescriptor,
    label: &str,
) -> Result<[PipelineHandle; 8], render_core::RhiError> {
    let mut pipelines = Vec::with_capacity(8);
    for (double_sided, blend_mode) in [
        (false, None),
        (true, None),
        (false, Some("alpha")),
        (true, Some("alpha")),
        (false, Some("additive")),
        (true, Some("additive")),
        (false, Some("weighted_oit")),
        (true, Some("weighted_oit")),
    ] {
        let mut descriptor = base.clone();
        descriptor.raster_state.cull_mode = Some(if double_sided { "none" } else { "back" }.into());
        descriptor.depth_state.write_enabled = blend_mode.is_none();
        descriptor.blend_state.mode = blend_mode.map(str::to_owned);
        descriptor.debug_label = Some(format!(
            "{label}-{}-{}",
            if double_sided {
                "double-sided"
            } else {
                "single-sided"
            },
            blend_mode.unwrap_or("opaque-mask")
        ));
        match device.create_pipeline(&descriptor) {
            Ok(pipeline) => pipelines.push(pipeline),
            Err(error) => {
                for pipeline in pipelines {
                    device.destroy_pipeline(pipeline);
                }
                return Err(error);
            }
        }
    }
    pipelines
        .try_into()
        .map_err(|_| render_core::RhiError::Backend {
            detail: "DX12 surface pipeline creation produced the wrong variant count".into(),
        })
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
pub(super) fn update_dynamic_storage_buffer(
    device: &mut Dx12Device,
    slot: &mut Option<Dx12DynamicVertexBuffer>,
    source: &[u8],
    label: &str,
) -> Result<BufferHandle, render_core::RhiError> {
    let mut bytes = source.to_vec();
    if bytes.is_empty() {
        bytes.resize(4, 0);
    } else {
        bytes.resize(bytes.len().next_multiple_of(4), 0);
    }
    if let Some(existing) = slot.as_mut() {
        if existing.capacity >= bytes.len() {
            if existing.bytes != bytes {
                device.write_buffer(existing.handle, &bytes, 0)?;
                existing.bytes = bytes;
            }
            return Ok(existing.handle);
        }
    }
    if let Some(old) = slot.take() {
        device.destroy_buffer(old.handle);
    }
    let capacity = bytes.len().next_power_of_two().max(4);
    let handle = device.create_buffer(&BufferDescriptor {
        size_bytes: capacity as u64,
        usage_flags: render_core::BufferUsage::STORAGE,
        memory_hint: MemoryHint::CpuToGpu,
        debug_label: Some(label.to_owned()),
    })?;
    if let Err(error) = device.write_buffer(handle, &bytes, 0) {
        device.destroy_buffer(handle);
        return Err(error);
    }
    *slot = Some(Dx12DynamicVertexBuffer {
        handle,
        bytes,
        capacity,
    });
    Ok(handle)
}
