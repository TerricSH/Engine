//! DirectX 12 implementation of [`engine_renderer::BackendRenderer`].
//!
//! The portable upload path supports owned static/skinned PBR meshes, textures,
//! and opaque, alpha-masked, alpha-blended, or double-sided material variants.

// ============================================================================
// Windows + backend-dx12: full implementation
// ============================================================================

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
use std::collections::HashMap;

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
use engine_renderer::{
    render_graph2, BackendRenderer, Diagnostic, DiagnosticSeverity, EnvironmentMapUpload,
    FrameStats, IndexFormat as RendererIndexFormat, MaterialUpload, MeshUpload,
    MeshVertexFormat as RendererMeshVertexFormat, MorphTargetSetUpload, RenderFrameInput,
    ResourceKind, ResourceRemoval, ShadowMode, TextureUpload, Transparency, UploadReceipt,
};
#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
use glam::Mat4;
#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
use render_core::{
    BufferDescriptor, BufferHandle, CommandEncoder, Device, FramebufferHandle,
    IndexFormat as RhiIndexFormat, MemoryHint, PipelineDescriptor, PipelineHandle,
    PipelineLayoutHandle, RenderPassHandle, SwapchainHandle, TextureHandle,
};

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
use crate::device::Dx12Device;
#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
use crate::scene_data::*;

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
struct Dx12MaterialState {
    constants: [u8; 32],
    emissive_constants: [u8; 16],
    advanced_constants: [u8; 16],
    texture_ids: [Option<String>; 5],
    transparency: Transparency,
    double_sided: bool,
    content_hash: [u8; 32],
    revision: u64,
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
const MATERIAL_TEXTURE_BINDINGS: [u32; 5] = [1, 3, 4, 5, 6];

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
#[derive(Clone, Debug)]
struct Dx12TextureState {
    handle: TextureHandle,
    content_hash: [u8; 32],
    revision: u64,
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
#[derive(Clone, Debug)]
struct Dx12EnvironmentState {
    handle: TextureHandle,
    mip_count: u32,
    content_hash: [u8; 32],
    revision: u64,
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
#[derive(Clone, Debug)]
struct Dx12BoneBuffer {
    handle: BufferHandle,
    bytes: Vec<u8>,
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
#[derive(Clone, Debug)]
struct Dx12DynamicVertexBuffer {
    handle: BufferHandle,
    bytes: Vec<u8>,
    capacity: usize,
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
#[derive(Clone, Debug)]
struct Dx12MorphTargetSet {
    vertex_count: u32,
    targets: Vec<engine_renderer::MorphTarget>,
    content_hash: [u8; 32],
    revision: u64,
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
#[derive(Clone, Copy, Debug)]
pub(crate) struct Dx12ShadowFrameData {
    pub(crate) light_view_projection: Mat4,
    pub(crate) light_direction_to_surface: glam::Vec3,
    pub(crate) soft: bool,
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
struct Dx12FrameState {
    image_index: u32,
    encoder: Box<dyn CommandEncoder>,
    draw_calls: u32,
    triangles: u64,
    visible_drawables: u32,
    culled_drawables: u32,
    visible_lights: u32,
    culled_lights: u32,
    hdr_pass_active: bool,
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
#[derive(Clone, Debug, PartialEq, Eq)]
struct Dx12UiDraw {
    first_vertex: u32,
    vertex_count: u32,
    texture_id: Option<String>,
    scissor: (i32, i32, u32, u32),
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct PreparedDx12Ui {
    vertex_bytes: Vec<u8>,
    draws: Vec<Dx12UiDraw>,
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
fn prepare_dx12_ui(
    batches: &[engine_renderer::UiBatch],
    width: u32,
    height: u32,
) -> Result<PreparedDx12Ui, String> {
    let mut prepared = PreparedDx12Ui::default();
    for (batch_index, batch) in batches.iter().enumerate() {
        if batch.indices.len() % 3 != 0 {
            return Err(format!(
                "UI batch {batch_index} index count {} is not a triangle-list multiple",
                batch.indices.len()
            ));
        }
        let rect = batch.clip_rect;
        if rect
            .min
            .into_iter()
            .chain(rect.max)
            .any(|value| !value.is_finite())
            || rect.max[0] < rect.min[0]
            || rect.max[1] < rect.min[1]
        {
            return Err(format!(
                "UI batch {batch_index} has an invalid clip rectangle"
            ));
        }
        let max_x = width.min(i32::MAX as u32) as f32;
        let max_y = height.min(i32::MAX as u32) as f32;
        let x0 = rect.min[0].floor().clamp(0.0, max_x) as i32;
        let y0 = rect.min[1].floor().clamp(0.0, max_y) as i32;
        let x1 = rect.max[0].ceil().clamp(0.0, max_x) as i32;
        let y1 = rect.max[1].ceil().clamp(0.0, max_y) as i32;
        if x1 <= x0 || y1 <= y0 || batch.indices.is_empty() {
            continue;
        }
        let first_vertex = u32::try_from(prepared.vertex_bytes.len() / 32)
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
            for value in vertex.position.into_iter().chain(vertex.uv) {
                prepared
                    .vertex_bytes
                    .extend_from_slice(&value.to_ne_bytes());
            }
            for channel in vertex.color {
                prepared
                    .vertex_bytes
                    .extend_from_slice(&(f32::from(channel) / 255.0).to_ne_bytes());
            }
        }
        prepared.draws.push(Dx12UiDraw {
            first_vertex,
            vertex_count: batch
                .indices
                .len()
                .try_into()
                .map_err(|_| format!("UI batch {batch_index} has too many indices"))?,
            texture_id: batch.texture.as_ref().map(|texture| texture.id.clone()),
            scissor: (x0, y0, (x1 - x0) as u32, (y1 - y0) as u32),
        });
    }
    Ok(prepared)
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
fn validate_dx12_frame_contract(input: &RenderFrameInput) -> Result<(), Vec<Diagnostic>> {
    if input.render_options.pass_graph_config.output_mode
        != engine_renderer::PassGraphOutputMode::HdrThenToneMap
    {
        return Err(vec![Diagnostic::new(
            "DX1247",
            DiagnosticSeverity::Error,
            "scene_renderer",
            "DX12 scene rendering uses the portable HdrThenToneMap graph; DirectToSwapchain bypasses required HDR composition",
        )]);
    }
    if input.views.is_empty() {
        return Err(vec![Diagnostic::new(
            "DX1244",
            DiagnosticSeverity::Error,
            "scene_renderer",
            "the DX12 backend requires at least one render view per frame",
        )]);
    }
    if input.render_options.msaa_samples != 1
        || input.views.iter().any(|view| view.msaa_samples != 1)
    {
        return Err(vec![Diagnostic::new(
            "DX1249",
            DiagnosticSeverity::Error,
            "scene_renderer",
            "DX12 SceneRenderer does not yet implement multisample resolve; use 1x MSAA",
        )]);
    }
    if input.views.iter().any(|view| {
        !view.viewport.is_valid_normalized() || view.viewport != view.viewport_rect_normalized
    }) {
        return Err(vec![Diagnostic::new(
            "DX1250",
            DiagnosticSeverity::Error,
            "scene_renderer",
            "DX12 render views require matching, valid normalized viewport rectangles",
        )]);
    }
    if input.views.iter().any(|view| {
        !matches!(
            view.clear_flags,
            engine_renderer::ClearFlags::ColorAndDepth | engine_renderer::ClearFlags::Skybox
        )
    }) {
        return Err(vec![Diagnostic::new(
            "DX1251",
            DiagnosticSeverity::Error,
            "scene_renderer",
            "DX12 SceneRenderer supports ColorAndDepth and Skybox clear modes",
        )]);
    }
    Ok(())
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
pub struct Dx12SceneRenderer {
    device: Dx12Device,
    meshes: HashMap<String, Dx12GpuMesh>,
    materials: HashMap<String, Dx12MaterialState>,
    textures: HashMap<String, Dx12TextureState>,
    environments: HashMap<String, Dx12EnvironmentState>,
    fallback_environment: Option<TextureHandle>,
    fallback_ui_texture: Option<TextureHandle>,
    bone_buffers: HashMap<String, Dx12BoneBuffer>,
    morphed_vertex_buffers: HashMap<String, Dx12DynamicVertexBuffer>,
    morph_target_sets: HashMap<String, Dx12MorphTargetSet>,
    particle_instance_buffers: HashMap<String, Dx12DynamicVertexBuffer>,
    gpu_particle_parameter_buffers: HashMap<String, Dx12DynamicVertexBuffer>,
    gpu_particle_dummy_buffer: Option<Dx12DynamicVertexBuffer>,
    clustered_light_buffer: Option<Dx12DynamicVertexBuffer>,
    clustered_grid_buffer: Option<Dx12DynamicVertexBuffer>,
    clustered_index_buffer: Option<Dx12DynamicVertexBuffer>,
    ui_vertex_buffer: Option<Dx12DynamicVertexBuffer>,
    // Revisions survive removal so recreating the same logical resource never
    // moves its receipt backwards.
    mesh_revisions: HashMap<String, u64>,
    width: u32,
    height: u32,
    swapchain: SwapchainHandle,
    pipeline_layout: Option<PipelineLayoutHandle>,
    pipeline: Option<PipelineHandle>,
    double_sided_pipeline: Option<PipelineHandle>,
    blend_pipeline: Option<PipelineHandle>,
    blend_double_sided_pipeline: Option<PipelineHandle>,
    oit_pipeline: Option<PipelineHandle>,
    oit_double_sided_pipeline: Option<PipelineHandle>,
    additive_pipeline: Option<PipelineHandle>,
    additive_double_sided_pipeline: Option<PipelineHandle>,
    skinned_pipeline: Option<PipelineHandle>,
    skinned_double_sided_pipeline: Option<PipelineHandle>,
    skinned_blend_pipeline: Option<PipelineHandle>,
    skinned_blend_double_sided_pipeline: Option<PipelineHandle>,
    skinned_oit_pipeline: Option<PipelineHandle>,
    skinned_oit_double_sided_pipeline: Option<PipelineHandle>,
    skinned_additive_pipeline: Option<PipelineHandle>,
    skinned_additive_double_sided_pipeline: Option<PipelineHandle>,
    particle_pipeline: Option<PipelineHandle>,
    particle_additive_pipeline: Option<PipelineHandle>,
    particle_oit_pipeline: Option<PipelineHandle>,
    gpu_particle_pipeline: Option<PipelineHandle>,
    gpu_particle_additive_pipeline: Option<PipelineHandle>,
    gpu_particle_oit_pipeline: Option<PipelineHandle>,
    skybox_pipeline: Option<PipelineHandle>,
    hdr_texture: Option<TextureHandle>,
    oit_accum_texture: Option<TextureHandle>,
    oit_optical_depth_texture: Option<TextureHandle>,
    hdr_depth_texture: Option<TextureHandle>,
    hdr_render_pass: Option<RenderPassHandle>,
    hdr_framebuffer: Option<FramebufferHandle>,
    tone_map_pipeline_layout: Option<PipelineLayoutHandle>,
    tone_map_pipeline: Option<PipelineHandle>,
    ui_pipeline_layout: Option<PipelineLayoutHandle>,
    ui_pipeline: Option<PipelineHandle>,
    shadow_texture: Option<TextureHandle>,
    shadow_render_pass: Option<RenderPassHandle>,
    shadow_framebuffer: Option<FramebufferHandle>,
    shadow_pipeline_layout: Option<PipelineLayoutHandle>,
    shadow_pipeline: Option<PipelineHandle>,
    skinned_shadow_pipeline: Option<PipelineHandle>,
    shadow_frame_data: Option<Dx12ShadowFrameData>,
    active_frame: Option<Dx12FrameState>,
    /// Any failure after handing a command list to `end_frame` makes allocator
    /// reuse ambiguous. Refuse subsequent frames until the backend is rebuilt.
    fatal_frame_error: Option<String>,
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
fn surface_variant_index(
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
fn create_surface_pipeline_variants(
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
fn update_dynamic_storage_buffer(
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

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
impl Dx12SceneRenderer {
    pub fn new(device: Dx12Device, swapchain: SwapchainHandle, width: u32, height: u32) -> Self {
        Self {
            device,
            meshes: HashMap::new(),
            materials: HashMap::new(),
            textures: HashMap::new(),
            environments: HashMap::new(),
            fallback_environment: None,
            fallback_ui_texture: None,
            bone_buffers: HashMap::new(),
            morphed_vertex_buffers: HashMap::new(),
            morph_target_sets: HashMap::new(),
            particle_instance_buffers: HashMap::new(),
            gpu_particle_parameter_buffers: HashMap::new(),
            gpu_particle_dummy_buffer: None,
            clustered_light_buffer: None,
            clustered_grid_buffer: None,
            clustered_index_buffer: None,
            ui_vertex_buffer: None,
            mesh_revisions: HashMap::new(),
            width: width.max(1),
            height: height.max(1),
            swapchain,
            pipeline_layout: None,
            pipeline: None,
            double_sided_pipeline: None,
            blend_pipeline: None,
            blend_double_sided_pipeline: None,
            oit_pipeline: None,
            oit_double_sided_pipeline: None,
            additive_pipeline: None,
            additive_double_sided_pipeline: None,
            skinned_pipeline: None,
            skinned_double_sided_pipeline: None,
            skinned_blend_pipeline: None,
            skinned_blend_double_sided_pipeline: None,
            skinned_oit_pipeline: None,
            skinned_oit_double_sided_pipeline: None,
            skinned_additive_pipeline: None,
            skinned_additive_double_sided_pipeline: None,
            particle_pipeline: None,
            particle_additive_pipeline: None,
            particle_oit_pipeline: None,
            gpu_particle_pipeline: None,
            gpu_particle_additive_pipeline: None,
            gpu_particle_oit_pipeline: None,
            skybox_pipeline: None,
            hdr_texture: None,
            oit_accum_texture: None,
            oit_optical_depth_texture: None,
            hdr_depth_texture: None,
            hdr_render_pass: None,
            hdr_framebuffer: None,
            tone_map_pipeline_layout: None,
            tone_map_pipeline: None,
            ui_pipeline_layout: None,
            ui_pipeline: None,
            shadow_texture: None,
            shadow_render_pass: None,
            shadow_framebuffer: None,
            shadow_pipeline_layout: None,
            shadow_pipeline: None,
            skinned_shadow_pipeline: None,
            shadow_frame_data: None,
            active_frame: None,
            fatal_frame_error: None,
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), Vec<Diagnostic>> {
        self.resize_surface(width, height)
    }

    pub fn wait_idle(&self) {
        self.device.wait_idle();
    }

    fn resize_surface(&mut self, width: u32, height: u32) -> Result<(), Vec<Diagnostic>> {
        if width == 0 || height == 0 {
            return Err(vec![Diagnostic::new(
                "DX1240",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("resize dimensions must be non-zero, got {width}x{height}"),
            )]);
        }
        if self.active_frame.is_some() {
            return Err(vec![Diagnostic::new(
                "DX1242",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "cannot resize the DX12 surface while a frame is active",
            )]);
        }

        self.device
            .recreate_swapchain(self.swapchain, width, height)
            .map_err(|error| {
                vec![Diagnostic::new(
                    "DX1241",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("resize/recreate_swapchain failed: {error:?}"),
                )]
            })?;
        self.width = width;
        self.height = height;
        if let Some(framebuffer) = self.hdr_framebuffer.take() {
            self.device.destroy_framebuffer(framebuffer);
        }
        if let Some(texture) = self.hdr_texture.take() {
            self.device.destroy_texture(texture);
        }
        if let Some(texture) = self.oit_accum_texture.take() {
            self.device.destroy_texture(texture);
        }
        if let Some(texture) = self.oit_optical_depth_texture.take() {
            self.device.destroy_texture(texture);
        }
        if let Some(texture) = self.hdr_depth_texture.take() {
            self.device.destroy_texture(texture);
        }
        Ok(())
    }

    fn ensure_hdr_targets(&mut self) -> Result<(), render_core::RhiError> {
        use render_core::{
            FramebufferDescriptor, RenderPassDescriptor, TextureDescriptor, TextureFormat,
            TextureUsage,
        };

        let render_pass = match self.hdr_render_pass {
            Some(render_pass) => render_pass,
            None => {
                let render_pass = self.device.create_render_pass(&RenderPassDescriptor {
                    color_attachments: vec![
                        TextureFormat::Rgba16Float,
                        TextureFormat::Rgba16Float,
                        TextureFormat::Rgba16Float,
                    ],
                    depth_stencil_format: Some(TextureFormat::Depth32Float),
                    sample_count: 1,
                    present_after: false,
                    debug_label: Some("dx12-hdr-forward".into()),
                })?;
                self.hdr_render_pass = Some(render_pass);
                render_pass
            }
        };
        if self.hdr_framebuffer.is_some()
            && self.hdr_texture.is_some()
            && self.oit_accum_texture.is_some()
            && self.oit_optical_depth_texture.is_some()
            && self.hdr_depth_texture.is_some()
        {
            return Ok(());
        }

        let hdr_texture = self.device.create_texture(&TextureDescriptor {
            width: self.width,
            height: self.height,
            depth_or_layers: 1,
            mip_levels: 1,
            format: TextureFormat::Rgba16Float,
            usage_flags: TextureUsage(TextureUsage::COLOR_ATTACHMENT.0 | TextureUsage::SAMPLED.0),
            sample_count: 1,
            debug_label: Some("dx12-hdr-color".into()),
        })?;
        let hdr_depth = match self.device.create_texture(&TextureDescriptor {
            width: self.width,
            height: self.height,
            depth_or_layers: 1,
            mip_levels: 1,
            format: TextureFormat::Depth32Float,
            usage_flags: TextureUsage(TextureUsage::DEPTH_ATTACHMENT.0),
            sample_count: 1,
            debug_label: Some("dx12-hdr-depth".into()),
        }) {
            Ok(texture) => texture,
            Err(error) => {
                self.device.destroy_texture(hdr_texture);
                return Err(error);
            }
        };
        let target_width = self.width;
        let target_height = self.height;
        let make_oit_texture = |device: &mut Dx12Device, label: &str| {
            device.create_texture(&TextureDescriptor {
                width: target_width,
                height: target_height,
                depth_or_layers: 1,
                mip_levels: 1,
                format: TextureFormat::Rgba16Float,
                usage_flags: TextureUsage(
                    TextureUsage::COLOR_ATTACHMENT.0 | TextureUsage::SAMPLED.0,
                ),
                sample_count: 1,
                debug_label: Some(label.into()),
            })
        };
        let oit_accum = match make_oit_texture(&mut self.device, "dx12-oit-accumulation") {
            Ok(texture) => texture,
            Err(error) => {
                self.device.destroy_texture(hdr_depth);
                self.device.destroy_texture(hdr_texture);
                return Err(error);
            }
        };
        let oit_optical_depth = match make_oit_texture(&mut self.device, "dx12-oit-optical-depth") {
            Ok(texture) => texture,
            Err(error) => {
                self.device.destroy_texture(oit_accum);
                self.device.destroy_texture(hdr_depth);
                self.device.destroy_texture(hdr_texture);
                return Err(error);
            }
        };
        let framebuffer = match self.device.create_framebuffer(&FramebufferDescriptor {
            render_pass,
            color_attachments: vec![hdr_texture, oit_accum, oit_optical_depth],
            depth_stencil_attachment: Some(hdr_depth),
            width: self.width,
            height: self.height,
            debug_label: Some("dx12-hdr-forward".into()),
        }) {
            Ok(framebuffer) => framebuffer,
            Err(error) => {
                self.device.destroy_texture(oit_optical_depth);
                self.device.destroy_texture(oit_accum);
                self.device.destroy_texture(hdr_depth);
                self.device.destroy_texture(hdr_texture);
                return Err(error);
            }
        };
        self.hdr_texture = Some(hdr_texture);
        self.oit_accum_texture = Some(oit_accum);
        self.oit_optical_depth_texture = Some(oit_optical_depth);
        self.hdr_depth_texture = Some(hdr_depth);
        self.hdr_framebuffer = Some(framebuffer);
        Ok(())
    }

    fn ensure_environment_fallback(&mut self) -> Result<(), render_core::RhiError> {
        if self.fallback_environment.is_some() {
            return Ok(());
        }
        // Keep a type-correct TextureCube bound when a scene has no HDRI.
        // Its contribution is disabled through environment intensity.
        let one = 0x3c00_u16.to_le_bytes();
        let pixel = [
            one[0], one[1], one[0], one[1], one[0], one[1], one[0], one[1],
        ];
        let mip = engine_renderer::EnvironmentCubeMip {
            face_size: 1,
            faces: vec![pixel.to_vec(); 6],
        };
        self.fallback_environment = Some(self.device.upload_sampled_rgba16f_cube(&[mip])?);
        Ok(())
    }

    fn ensure_ui_fallback(&mut self) -> Result<(), render_core::RhiError> {
        if self.fallback_ui_texture.is_some() {
            return Ok(());
        }
        self.fallback_ui_texture = Some(self.device.upload_sampled_rgba8(
            1,
            1,
            engine_renderer::ColorSpace::Linear,
            &[engine_renderer::TextureMipLevel {
                width: 1,
                height: 1,
                bytes: vec![255; 4],
            }],
            engine_renderer::SamplerDescriptor::default(),
        )?);
        Ok(())
    }

    /// Create the minimal static PBR32 forward PSO used by this backend.
    fn ensure_pipeline(&mut self) {
        use render_core::{
            BindGroupLayoutBinding, BindGroupLayoutDescriptor, PipelineDescriptor,
            PipelineLayoutDescriptor, PushConstantRange, ShaderFormat, ShaderModuleDescriptor,
            ShaderStage, TextureDescriptor, TextureUsage, VertexAttribute, VertexLayout,
        };

        if let Err(error) = self.ensure_hdr_targets() {
            tracing::error!(target: "scene_renderer", ?error, "create DX12 HDR targets failed");
            return;
        }
        if let Err(error) = self.ensure_environment_fallback() {
            tracing::error!(target: "scene_renderer", ?error, "create DX12 fallback environment failed");
            return;
        }
        if let Err(error) = self.ensure_ui_fallback() {
            tracing::error!(target: "scene_renderer", ?error, "create DX12 UI fallback failed");
            return;
        }

        if self.pipeline.is_some()
            && self.double_sided_pipeline.is_some()
            && self.blend_pipeline.is_some()
            && self.blend_double_sided_pipeline.is_some()
            && self.oit_pipeline.is_some()
            && self.oit_double_sided_pipeline.is_some()
            && self.additive_pipeline.is_some()
            && self.additive_double_sided_pipeline.is_some()
            && self.skinned_pipeline.is_some()
            && self.skinned_double_sided_pipeline.is_some()
            && self.skinned_blend_pipeline.is_some()
            && self.skinned_blend_double_sided_pipeline.is_some()
            && self.skinned_oit_pipeline.is_some()
            && self.skinned_oit_double_sided_pipeline.is_some()
            && self.skinned_additive_pipeline.is_some()
            && self.skinned_additive_double_sided_pipeline.is_some()
            && self.particle_pipeline.is_some()
            && self.particle_additive_pipeline.is_some()
            && self.particle_oit_pipeline.is_some()
            && self.gpu_particle_pipeline.is_some()
            && self.gpu_particle_additive_pipeline.is_some()
            && self.gpu_particle_oit_pipeline.is_some()
            && self.skybox_pipeline.is_some()
            && self.tone_map_pipeline.is_some()
            && self.fallback_environment.is_some()
            && self.ui_pipeline.is_some()
            && self.fallback_ui_texture.is_some()
            && self.shadow_pipeline.is_some()
            && self.skinned_shadow_pipeline.is_some()
            && self.shadow_texture.is_some()
            && self.shadow_framebuffer.is_some()
        {
            return;
        }

        let vs_bytes: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/scene_vs.dxil"));
        let ps_bytes: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/scene_ps.dxil"));
        if vs_bytes.is_empty() || ps_bytes.is_empty() {
            tracing::error!(
                target: "scene_renderer",
                "DXIL shaders are unavailable; DX12 rendering cannot start"
            );
            return;
        }

        let layout = match self
            .device
            .create_pipeline_layout(&PipelineLayoutDescriptor {
                push_constant_ranges: vec![PushConstantRange {
                    stage_flags: 3,
                    offset: 0,
                    size: 240,
                }],
                bind_group_layouts: vec![BindGroupLayoutDescriptor {
                    set_index: 0,
                    bindings: vec![
                        BindGroupLayoutBinding {
                            binding: 0,
                            resource_kind: "scene_resource_set".into(),
                        },
                        BindGroupLayoutBinding {
                            binding: 0,
                            resource_kind: "sampler_set7".into(),
                        },
                        BindGroupLayoutBinding {
                            binding: 1,
                            resource_kind: "uniform_buffer".into(),
                        },
                    ],
                }],
                debug_label: Some("scene_renderer".into()),
            }) {
            Ok(handle) => handle,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "create_pipeline_layout failed");
                return;
            }
        };
        self.pipeline_layout = Some(layout);

        let vertex_shader = match self.device.create_shader_module(&ShaderModuleDescriptor {
            format: ShaderFormat::Dxil,
            stage: ShaderStage::Vertex,
            source_bytes: vs_bytes.to_vec(),
            source_hash: [0; 32],
            entry_points: vec!["VSMain".into()],
            debug_label: Some("scene_renderer_vs".into()),
        }) {
            Ok(handle) => handle,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "vertex shader creation failed");
                return;
            }
        };

        let pixel_shader = match self.device.create_shader_module(&ShaderModuleDescriptor {
            format: ShaderFormat::Dxil,
            stage: ShaderStage::Fragment,
            source_bytes: ps_bytes.to_vec(),
            source_hash: [1; 32],
            entry_points: vec!["PSMain".into()],
            debug_label: Some("scene_renderer_ps".into()),
        }) {
            Ok(handle) => handle,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "pixel shader creation failed");
                return;
            }
        };

        let skinned_vs_bytes: &[u8] =
            include_bytes!(concat!(env!("OUT_DIR"), "/scene_skinned_vs.dxil"));
        let skinned_vertex_shader = match self.device.create_shader_module(
            &ShaderModuleDescriptor {
                format: ShaderFormat::Dxil,
                stage: ShaderStage::Vertex,
                source_bytes: skinned_vs_bytes.to_vec(),
                source_hash: [2; 32],
                entry_points: vec!["SkinnedVSMain".into()],
                debug_label: Some("scene_renderer_skinned_vs".into()),
            },
        ) {
            Ok(handle) => handle,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "skinned vertex shader creation failed");
                return;
            }
        };

        let vertex_layout = VertexLayout {
            stride_bytes: 32,
            attributes: vec![
                VertexAttribute {
                    semantic: "POSITION".into(),
                    format: "float32x3".into(),
                    offset_bytes: 0,
                },
                VertexAttribute {
                    semantic: "NORMAL".into(),
                    format: "float32x3".into(),
                    offset_bytes: 12,
                },
                VertexAttribute {
                    semantic: "TEXCOORD".into(),
                    format: "float32x2".into(),
                    offset_bytes: 24,
                },
            ],
        };
        let shadow_static_vertex_layout = vertex_layout.clone();
        let descriptor = PipelineDescriptor {
            shader_modules: vec![vertex_shader, pixel_shader],
            vertex_layout,
            pipeline_layout: Some(layout),
            topology: Some("triangle_list".into()),
            render_targets: vec![
                render_core::TextureFormat::Rgba16Float,
                render_core::TextureFormat::Rgba16Float,
                render_core::TextureFormat::Rgba16Float,
            ],
            depth_state: render_core::DepthState {
                format: Some(render_core::TextureFormat::Depth32Float),
                write_enabled: true,
                compare: Some("less".into()),
            },
            raster_state: render_core::RasterState {
                cull_mode: Some("back".into()),
                front_face: Some("ccw".into()),
            },
            render_pass: self.hdr_render_pass,
            ..PipelineDescriptor::default()
        };
        let static_variants = match create_surface_pipeline_variants(
            &mut self.device,
            &descriptor,
            "scene-static",
        ) {
            Ok(variants) => variants,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "DX12 static surface PSO creation failed");
                return;
            }
        };

        let skinned_vertex_layout = VertexLayout {
            stride_bytes: 64,
            attributes: vec![
                VertexAttribute {
                    semantic: "POSITION".into(),
                    format: "float32x3".into(),
                    offset_bytes: 0,
                },
                VertexAttribute {
                    semantic: "NORMAL".into(),
                    format: "float32x3".into(),
                    offset_bytes: 12,
                },
                VertexAttribute {
                    semantic: "TEXCOORD".into(),
                    format: "float32x2".into(),
                    offset_bytes: 24,
                },
                VertexAttribute {
                    semantic: "JOINTS".into(),
                    format: "uint32x4".into(),
                    offset_bytes: 32,
                },
                VertexAttribute {
                    semantic: "WEIGHTS".into(),
                    format: "float32x4".into(),
                    offset_bytes: 48,
                },
            ],
        };
        let skinned_descriptor = PipelineDescriptor {
            shader_modules: vec![skinned_vertex_shader, pixel_shader],
            vertex_layout: skinned_vertex_layout,
            ..descriptor.clone()
        };
        let skinned_variants = match create_surface_pipeline_variants(
            &mut self.device,
            &skinned_descriptor,
            "scene-skinned",
        ) {
            Ok(variants) => variants,
            Err(error) => {
                for pipeline in static_variants {
                    self.device.destroy_pipeline(pipeline);
                }
                tracing::error!(target: "scene_renderer", ?error, "DX12 skinned surface PSO creation failed");
                return;
            }
        };
        self.pipeline = Some(static_variants[0]);
        self.double_sided_pipeline = Some(static_variants[1]);
        self.blend_pipeline = Some(static_variants[2]);
        self.blend_double_sided_pipeline = Some(static_variants[3]);
        self.additive_pipeline = Some(static_variants[4]);
        self.additive_double_sided_pipeline = Some(static_variants[5]);
        self.oit_pipeline = Some(static_variants[6]);
        self.oit_double_sided_pipeline = Some(static_variants[7]);
        self.skinned_pipeline = Some(skinned_variants[0]);
        self.skinned_double_sided_pipeline = Some(skinned_variants[1]);
        self.skinned_blend_pipeline = Some(skinned_variants[2]);
        self.skinned_blend_double_sided_pipeline = Some(skinned_variants[3]);
        self.skinned_additive_pipeline = Some(skinned_variants[4]);
        self.skinned_additive_double_sided_pipeline = Some(skinned_variants[5]);
        self.skinned_oit_pipeline = Some(skinned_variants[6]);
        self.skinned_oit_double_sided_pipeline = Some(skinned_variants[7]);

        let particle_vs_bytes: &[u8] =
            include_bytes!(concat!(env!("OUT_DIR"), "/particle_vs.dxil"));
        let particle_vertex_shader = match self.device.create_shader_module(
            &ShaderModuleDescriptor {
                format: ShaderFormat::Dxil,
                stage: ShaderStage::Vertex,
                source_bytes: particle_vs_bytes.to_vec(),
                source_hash: [7; 32],
                entry_points: vec!["ParticleVSMain".into()],
                debug_label: Some("dx12-particle-vs".into()),
            },
        ) {
            Ok(shader) => shader,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "DX12 particle vertex shader creation failed");
                return;
            }
        };
        let particle_vertex_layout = VertexLayout {
            stride_bytes: 32,
            attributes: vec![
                VertexAttribute {
                    semantic: "POSITION".into(),
                    format: "float32x3".into(),
                    offset_bytes: 0,
                },
                VertexAttribute {
                    semantic: "NORMAL".into(),
                    format: "float32x3".into(),
                    offset_bytes: 12,
                },
                VertexAttribute {
                    semantic: "TEXCOORD".into(),
                    format: "float32x2".into(),
                    offset_bytes: 24,
                },
                VertexAttribute {
                    semantic: "INSTANCE_POSITION_SIZE".into(),
                    format: "float32x4".into(),
                    offset_bytes: 0,
                },
                VertexAttribute {
                    semantic: "INSTANCE_ROTATION_AGE".into(),
                    format: "float32x2".into(),
                    offset_bytes: 16,
                },
                VertexAttribute {
                    semantic: "INSTANCE_COLOR".into(),
                    format: "uint32".into(),
                    offset_bytes: 24,
                },
            ],
        };
        let particle_descriptor = PipelineDescriptor {
            shader_modules: vec![particle_vertex_shader, pixel_shader],
            vertex_layout: particle_vertex_layout,
            pipeline_layout: Some(layout),
            topology: Some("triangle_list".into()),
            render_targets: vec![
                render_core::TextureFormat::Rgba16Float,
                render_core::TextureFormat::Rgba16Float,
                render_core::TextureFormat::Rgba16Float,
            ],
            depth_state: render_core::DepthState {
                format: Some(render_core::TextureFormat::Depth32Float),
                write_enabled: false,
                compare: Some("less".into()),
            },
            raster_state: render_core::RasterState {
                cull_mode: Some("none".into()),
                front_face: Some("ccw".into()),
            },
            blend_state: render_core::BlendState {
                mode: Some("alpha".into()),
            },
            render_pass: self.hdr_render_pass,
            debug_label: Some("dx12-particle-billboard".into()),
            ..PipelineDescriptor::default()
        };
        let particle_pipeline = match self.device.create_pipeline(&particle_descriptor) {
            Ok(pipeline) => pipeline,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "DX12 particle PSO creation failed");
                return;
            }
        };
        let mut additive_particle_descriptor = particle_descriptor.clone();
        additive_particle_descriptor.blend_state.mode = Some("additive".into());
        additive_particle_descriptor.debug_label = Some("dx12-particle-billboard-additive".into());
        let particle_additive_pipeline = match self
            .device
            .create_pipeline(&additive_particle_descriptor)
        {
            Ok(pipeline) => pipeline,
            Err(error) => {
                self.device.destroy_pipeline(particle_pipeline);
                tracing::error!(target: "scene_renderer", ?error, "DX12 additive particle PSO creation failed");
                return;
            }
        };
        self.particle_pipeline = Some(particle_pipeline);
        self.particle_additive_pipeline = Some(particle_additive_pipeline);
        let mut oit_particle_descriptor = particle_descriptor.clone();
        oit_particle_descriptor.blend_state.mode = Some("weighted_oit".into());
        oit_particle_descriptor.debug_label = Some("dx12-particle-billboard-oit".into());
        let particle_oit_pipeline = match self.device.create_pipeline(&oit_particle_descriptor) {
            Ok(pipeline) => pipeline,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "DX12 OIT particle PSO creation failed");
                return;
            }
        };
        self.particle_oit_pipeline = Some(particle_oit_pipeline);

        let gpu_particle_vs_bytes: &[u8] =
            include_bytes!(concat!(env!("OUT_DIR"), "/gpu_particle_vs.dxil"));
        let gpu_particle_vertex_shader = match self.device.create_shader_module(
            &ShaderModuleDescriptor {
                format: ShaderFormat::Dxil,
                stage: ShaderStage::Vertex,
                source_bytes: gpu_particle_vs_bytes.to_vec(),
                source_hash: [8; 32],
                entry_points: vec!["GpuParticleVSMain".into()],
                debug_label: Some("dx12-gpu-particle-vs".into()),
            },
        ) {
            Ok(shader) => shader,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "DX12 GPU-particle vertex shader creation failed");
                return;
            }
        };
        let mut gpu_particle_descriptor = particle_descriptor;
        gpu_particle_descriptor.shader_modules = vec![gpu_particle_vertex_shader, pixel_shader];
        gpu_particle_descriptor.vertex_layout = VertexLayout {
            stride_bytes: 32,
            attributes: vec![
                VertexAttribute {
                    semantic: "POSITION".into(),
                    format: "float32x3".into(),
                    offset_bytes: 0,
                },
                VertexAttribute {
                    semantic: "NORMAL".into(),
                    format: "float32x3".into(),
                    offset_bytes: 12,
                },
                VertexAttribute {
                    semantic: "TEXCOORD".into(),
                    format: "float32x2".into(),
                    offset_bytes: 24,
                },
            ],
        };
        gpu_particle_descriptor.debug_label = Some("dx12-gpu-particle-billboard".into());
        let gpu_particle_pipeline = match self.device.create_pipeline(&gpu_particle_descriptor) {
            Ok(pipeline) => pipeline,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "DX12 GPU-particle PSO creation failed");
                return;
            }
        };
        gpu_particle_descriptor.blend_state.mode = Some("additive".into());
        gpu_particle_descriptor.debug_label = Some("dx12-gpu-particle-billboard-additive".into());
        let gpu_particle_additive_pipeline = match self
            .device
            .create_pipeline(&gpu_particle_descriptor)
        {
            Ok(pipeline) => pipeline,
            Err(error) => {
                self.device.destroy_pipeline(gpu_particle_pipeline);
                tracing::error!(target: "scene_renderer", ?error, "DX12 additive GPU-particle PSO creation failed");
                return;
            }
        };
        self.gpu_particle_pipeline = Some(gpu_particle_pipeline);
        self.gpu_particle_additive_pipeline = Some(gpu_particle_additive_pipeline);
        gpu_particle_descriptor.blend_state.mode = Some("weighted_oit".into());
        gpu_particle_descriptor.debug_label = Some("dx12-gpu-particle-billboard-oit".into());
        let gpu_particle_oit_pipeline = match self.device.create_pipeline(&gpu_particle_descriptor)
        {
            Ok(pipeline) => pipeline,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "DX12 OIT GPU-particle PSO creation failed");
                return;
            }
        };
        self.gpu_particle_oit_pipeline = Some(gpu_particle_oit_pipeline);

        let skybox_vs = include_bytes!(concat!(env!("OUT_DIR"), "/skybox_vs.dxil"));
        let skybox_ps = include_bytes!(concat!(env!("OUT_DIR"), "/skybox_ps.dxil"));
        let skybox_vertex_shader = match self.device.create_shader_module(&ShaderModuleDescriptor {
            format: ShaderFormat::Dxil,
            stage: ShaderStage::Vertex,
            source_bytes: skybox_vs.to_vec(),
            source_hash: [10; 32],
            entry_points: vec!["SkyboxVSMain".into()],
            debug_label: Some("dx12-skybox-vs".into()),
        }) {
            Ok(shader) => shader,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "DX12 skybox vertex shader creation failed");
                return;
            }
        };
        let skybox_pixel_shader = match self.device.create_shader_module(&ShaderModuleDescriptor {
            format: ShaderFormat::Dxil,
            stage: ShaderStage::Fragment,
            source_bytes: skybox_ps.to_vec(),
            source_hash: [11; 32],
            entry_points: vec!["SkyboxPSMain".into()],
            debug_label: Some("dx12-skybox-ps".into()),
        }) {
            Ok(shader) => shader,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "DX12 skybox pixel shader creation failed");
                return;
            }
        };
        let skybox_pipeline = match self.device.create_pipeline(&PipelineDescriptor {
            shader_modules: vec![skybox_vertex_shader, skybox_pixel_shader],
            vertex_layout: VertexLayout::default(),
            pipeline_layout: Some(layout),
            topology: Some("triangle_list".into()),
            render_targets: vec![
                render_core::TextureFormat::Rgba16Float,
                render_core::TextureFormat::Rgba16Float,
                render_core::TextureFormat::Rgba16Float,
            ],
            depth_state: render_core::DepthState {
                format: Some(render_core::TextureFormat::Depth32Float),
                write_enabled: false,
                compare: Some("less_equal".into()),
            },
            raster_state: render_core::RasterState {
                cull_mode: Some("none".into()),
                front_face: Some("ccw".into()),
            },
            render_pass: self.hdr_render_pass,
            debug_label: Some("dx12-skybox".into()),
            ..PipelineDescriptor::default()
        }) {
            Ok(pipeline) => pipeline,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "DX12 skybox PSO creation failed");
                return;
            }
        };
        self.skybox_pipeline = Some(skybox_pipeline);

        let tone_map_layout = match self
            .device
            .create_pipeline_layout(&PipelineLayoutDescriptor {
                push_constant_ranges: vec![PushConstantRange {
                    stage_flags: 2,
                    offset: 0,
                    size: 128,
                }],
                bind_group_layouts: vec![BindGroupLayoutDescriptor {
                    set_index: 0,
                    bindings: vec![
                        BindGroupLayoutBinding {
                            binding: 0,
                            resource_kind: "sampled_texture_triple".into(),
                        },
                        BindGroupLayoutBinding {
                            binding: 0,
                            resource_kind: "sampler_triple".into(),
                        },
                    ],
                }],
                debug_label: Some("dx12-tone-map".into()),
            }) {
            Ok(layout) => layout,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "DX12 tone-map root signature creation failed");
                return;
            }
        };
        let tone_map_vs = include_bytes!(concat!(env!("OUT_DIR"), "/tone_map_vs.dxil"));
        let tone_map_ps = include_bytes!(concat!(env!("OUT_DIR"), "/tone_map_ps.dxil"));
        let tone_map_vertex_shader = match self.device.create_shader_module(
            &ShaderModuleDescriptor {
                format: ShaderFormat::Dxil,
                stage: ShaderStage::Vertex,
                source_bytes: tone_map_vs.to_vec(),
                source_hash: [5; 32],
                entry_points: vec!["ToneMapVSMain".into()],
                debug_label: Some("dx12-tone-map-vs".into()),
            },
        ) {
            Ok(shader) => shader,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "DX12 tone-map vertex shader creation failed");
                return;
            }
        };
        let tone_map_pixel_shader = match self.device.create_shader_module(
            &ShaderModuleDescriptor {
                format: ShaderFormat::Dxil,
                stage: ShaderStage::Fragment,
                source_bytes: tone_map_ps.to_vec(),
                source_hash: [6; 32],
                entry_points: vec!["ToneMapPSMain".into()],
                debug_label: Some("dx12-tone-map-ps".into()),
            },
        ) {
            Ok(shader) => shader,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "DX12 tone-map pixel shader creation failed");
                return;
            }
        };
        let tone_map_pipeline = match self.device.create_pipeline(&PipelineDescriptor {
            shader_modules: vec![tone_map_vertex_shader, tone_map_pixel_shader],
            vertex_layout: VertexLayout::default(),
            pipeline_layout: Some(tone_map_layout),
            topology: Some("triangle_list".into()),
            render_targets: vec![render_core::TextureFormat::Bgra8Unorm],
            depth_state: render_core::DepthState {
                format: None,
                write_enabled: false,
                compare: Some("always".into()),
            },
            raster_state: render_core::RasterState {
                cull_mode: Some("none".into()),
                front_face: Some("ccw".into()),
            },
            debug_label: Some("dx12-tone-map".into()),
            ..PipelineDescriptor::default()
        }) {
            Ok(pipeline) => pipeline,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "DX12 tone-map PSO creation failed");
                return;
            }
        };
        self.tone_map_pipeline_layout = Some(tone_map_layout);
        self.tone_map_pipeline = Some(tone_map_pipeline);

        let ui_layout = match self
            .device
            .create_pipeline_layout(&PipelineLayoutDescriptor {
                push_constant_ranges: vec![PushConstantRange {
                    stage_flags: 1,
                    offset: 0,
                    size: 8,
                }],
                bind_group_layouts: vec![BindGroupLayoutDescriptor {
                    set_index: 0,
                    bindings: vec![
                        BindGroupLayoutBinding {
                            binding: 0,
                            resource_kind: "sampled_texture".into(),
                        },
                        BindGroupLayoutBinding {
                            binding: 0,
                            resource_kind: "sampler".into(),
                        },
                    ],
                }],
                debug_label: Some("dx12-ui".into()),
            }) {
            Ok(layout) => layout,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "DX12 UI root signature creation failed");
                return;
            }
        };
        let ui_vs = include_bytes!(concat!(env!("OUT_DIR"), "/ui_vs.dxil"));
        let ui_ps = include_bytes!(concat!(env!("OUT_DIR"), "/ui_ps.dxil"));
        let ui_vertex_shader = match self.device.create_shader_module(&ShaderModuleDescriptor {
            format: ShaderFormat::Dxil,
            stage: ShaderStage::Vertex,
            source_bytes: ui_vs.to_vec(),
            source_hash: [8; 32],
            entry_points: vec!["UiVSMain".into()],
            debug_label: Some("dx12-ui-vs".into()),
        }) {
            Ok(shader) => shader,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "DX12 UI vertex shader creation failed");
                return;
            }
        };
        let ui_pixel_shader = match self.device.create_shader_module(&ShaderModuleDescriptor {
            format: ShaderFormat::Dxil,
            stage: ShaderStage::Fragment,
            source_bytes: ui_ps.to_vec(),
            source_hash: [9; 32],
            entry_points: vec!["UiPSMain".into()],
            debug_label: Some("dx12-ui-ps".into()),
        }) {
            Ok(shader) => shader,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "DX12 UI pixel shader creation failed");
                return;
            }
        };
        let ui_pipeline = match self.device.create_pipeline(&PipelineDescriptor {
            shader_modules: vec![ui_vertex_shader, ui_pixel_shader],
            vertex_layout: VertexLayout {
                stride_bytes: 32,
                attributes: vec![
                    VertexAttribute {
                        semantic: "POSITION".into(),
                        format: "float32x2".into(),
                        offset_bytes: 0,
                    },
                    VertexAttribute {
                        semantic: "TEXCOORD".into(),
                        format: "float32x2".into(),
                        offset_bytes: 8,
                    },
                    VertexAttribute {
                        semantic: "COLOR".into(),
                        format: "float32x4".into(),
                        offset_bytes: 16,
                    },
                ],
            },
            pipeline_layout: Some(ui_layout),
            topology: Some("triangle_list".into()),
            render_targets: vec![render_core::TextureFormat::Bgra8Unorm],
            depth_state: render_core::DepthState {
                format: None,
                write_enabled: false,
                compare: Some("always".into()),
            },
            raster_state: render_core::RasterState {
                cull_mode: Some("none".into()),
                front_face: Some("ccw".into()),
            },
            blend_state: render_core::BlendState {
                mode: Some("alpha".into()),
            },
            debug_label: Some("dx12-ui".into()),
            ..PipelineDescriptor::default()
        }) {
            Ok(pipeline) => pipeline,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "DX12 UI PSO creation failed");
                return;
            }
        };
        self.ui_pipeline_layout = Some(ui_layout);
        self.ui_pipeline = Some(ui_pipeline);

        let shadow_texture = match self.device.create_texture(&TextureDescriptor {
            width: 2048,
            height: 2048,
            depth_or_layers: 1,
            mip_levels: 1,
            format: render_core::TextureFormat::Depth32Float,
            usage_flags: TextureUsage(TextureUsage::DEPTH_ATTACHMENT.0 | TextureUsage::SAMPLED.0),
            sample_count: 1,
            debug_label: Some("directional-shadow-map".into()),
        }) {
            Ok(handle) => handle,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "DX12 shadow texture creation failed");
                return;
            }
        };
        let shadow_render_pass = match self.device.create_render_pass(
            &render_core::RenderPassDescriptor {
                color_attachments: Vec::new(),
                depth_stencil_format: Some(render_core::TextureFormat::Depth32Float),
                sample_count: 1,
                present_after: false,
                debug_label: Some("directional-shadow-pass".into()),
            },
        ) {
            Ok(handle) => handle,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "DX12 shadow render pass creation failed");
                return;
            }
        };
        let shadow_framebuffer = match self.device.create_framebuffer(
            &render_core::FramebufferDescriptor {
                render_pass: shadow_render_pass,
                color_attachments: Vec::new(),
                depth_stencil_attachment: Some(shadow_texture),
                width: 2048,
                height: 2048,
                debug_label: Some("directional-shadow-framebuffer".into()),
            },
        ) {
            Ok(handle) => handle,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "DX12 shadow framebuffer creation failed");
                return;
            }
        };
        let shadow_layout = match self
            .device
            .create_pipeline_layout(&PipelineLayoutDescriptor {
                push_constant_ranges: vec![PushConstantRange {
                    stage_flags: 1,
                    offset: 0,
                    size: 192,
                }],
                bind_group_layouts: vec![BindGroupLayoutDescriptor {
                    set_index: 0,
                    bindings: vec![BindGroupLayoutBinding {
                        binding: 1,
                        resource_kind: "uniform_buffer".into(),
                    }],
                }],
                debug_label: Some("directional-shadow".into()),
            }) {
            Ok(handle) => handle,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "DX12 shadow root signature creation failed");
                return;
            }
        };
        let shadow_vs_bytes: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/shadow_vs.dxil"));
        let shadow_vertex_shader = match self.device.create_shader_module(&ShaderModuleDescriptor {
            format: ShaderFormat::Dxil,
            stage: ShaderStage::Vertex,
            source_bytes: shadow_vs_bytes.to_vec(),
            source_hash: [3; 32],
            entry_points: vec!["ShadowVSMain".into()],
            debug_label: Some("directional_shadow_vs".into()),
        }) {
            Ok(handle) => handle,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "DX12 shadow vertex shader creation failed");
                return;
            }
        };
        let skinned_shadow_vs_bytes: &[u8] =
            include_bytes!(concat!(env!("OUT_DIR"), "/shadow_skinned_vs.dxil"));
        let skinned_shadow_vertex_shader = match self.device.create_shader_module(
            &ShaderModuleDescriptor {
                format: ShaderFormat::Dxil,
                stage: ShaderStage::Vertex,
                source_bytes: skinned_shadow_vs_bytes.to_vec(),
                source_hash: [4; 32],
                entry_points: vec!["SkinnedShadowVSMain".into()],
                debug_label: Some("directional_shadow_skinned_vs".into()),
            },
        ) {
            Ok(handle) => handle,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "DX12 skinned shadow vertex shader creation failed");
                return;
            }
        };
        let shadow_descriptor = PipelineDescriptor {
            shader_modules: vec![shadow_vertex_shader],
            vertex_layout: shadow_static_vertex_layout,
            pipeline_layout: Some(shadow_layout),
            topology: Some("triangle_list".into()),
            render_targets: Vec::new(),
            depth_state: render_core::DepthState {
                format: Some(render_core::TextureFormat::Depth32Float),
                write_enabled: true,
                compare: Some("less".into()),
            },
            raster_state: render_core::RasterState {
                cull_mode: Some("back".into()),
                front_face: Some("ccw".into()),
            },
            ..PipelineDescriptor::default()
        };
        let shadow_pipeline = match self.device.create_pipeline(&shadow_descriptor) {
            Ok(handle) => handle,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "DX12 shadow PSO creation failed");
                return;
            }
        };
        let skinned_shadow_descriptor = PipelineDescriptor {
            shader_modules: vec![skinned_shadow_vertex_shader],
            vertex_layout: skinned_descriptor.vertex_layout,
            ..shadow_descriptor
        };
        let skinned_shadow_pipeline = match self.device.create_pipeline(&skinned_shadow_descriptor)
        {
            Ok(handle) => handle,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "DX12 skinned shadow PSO creation failed");
                return;
            }
        };
        self.shadow_texture = Some(shadow_texture);
        self.shadow_render_pass = Some(shadow_render_pass);
        self.shadow_framebuffer = Some(shadow_framebuffer);
        self.shadow_pipeline_layout = Some(shadow_layout);
        self.shadow_pipeline = Some(shadow_pipeline);
        self.skinned_shadow_pipeline = Some(skinned_shadow_pipeline);
    }

    fn material_surface(
        &self,
        input: &RenderFrameInput,
        material_id: &engine_serialize::AssetId,
    ) -> (Transparency, bool) {
        input
            .materials
            .iter()
            .find(|binding| binding.material_id == *material_id)
            .map(|binding| (binding.transparency.clone(), binding.double_sided))
            .or_else(|| {
                self.materials
                    .get(&material_id.id)
                    .map(|material| (material.transparency.clone(), material.double_sided))
            })
            .unwrap_or((Transparency::Opaque, false))
    }

    fn material_texture_ids(
        &self,
        input: &RenderFrameInput,
        material_id: &engine_serialize::AssetId,
    ) -> [Option<String>; 5] {
        input
            .materials
            .iter()
            .find(|binding| binding.material_id == *material_id)
            .map(|material| {
                MATERIAL_TEXTURE_BINDINGS.map(|binding| {
                    material
                        .textures
                        .iter()
                        .find(|slot| slot.binding == binding)
                        .map(|slot| slot.texture.id.clone())
                })
            })
            .or_else(|| {
                self.materials
                    .get(&material_id.id)
                    .map(|material| material.texture_ids.clone())
            })
            .unwrap_or_else(|| std::array::from_fn(|_| None))
    }

    fn material_texture_table(
        &self,
        texture_ids: &[Option<String>; 5],
        shadow_texture: TextureHandle,
        environment_texture: TextureHandle,
    ) -> [TextureHandle; 7] {
        let resolve = |texture_id: &Option<String>| {
            texture_id
                .as_ref()
                .and_then(|texture_id| self.textures.get(texture_id))
                .map(|texture| texture.handle)
                .unwrap_or(shadow_texture)
        };
        [
            resolve(&texture_ids[0]),
            shadow_texture,
            resolve(&texture_ids[1]),
            resolve(&texture_ids[2]),
            resolve(&texture_ids[3]),
            resolve(&texture_ids[4]),
            environment_texture,
        ]
    }

    fn environment_binding(
        &self,
        input: &RenderFrameInput,
        camera_position: glam::Vec3,
    ) -> Result<(TextureHandle, [u8; 16]), Vec<Diagnostic>> {
        let selected = select_environment_map(&input.render_options.environment, camera_position);
        let Some(environment_id) = selected else {
            let fallback = self.fallback_environment.ok_or_else(|| {
                vec![Diagnostic::new(
                    "DX1260",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    "DX12 fallback environment is unavailable",
                )]
            })?;
            return Ok((fallback, float4_bytes([0.0, 0.0, 0.0, 0.0])));
        };
        let environment = self.environments.get(&environment_id.id).ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1259",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!(
                    "environment map '{}' was selected before a successful DX12 upload",
                    environment_id.id
                ),
            )]
        })?;
        Ok((
            environment.handle,
            float4_bytes([
                input.render_options.environment.intensity,
                input.render_options.environment.rotation_radians,
                environment.mip_count.saturating_sub(1) as f32,
                1.0,
            ]),
        ))
    }

    fn prepare_bone_buffer(
        &mut self,
        cache_key: &str,
        palette: &[[f32; 16]],
    ) -> Result<BufferHandle, Vec<Diagnostic>> {
        let mut bytes = Vec::with_capacity(4096);
        for matrix in palette {
            for value in matrix {
                bytes.extend_from_slice(&value.to_ne_bytes());
            }
        }
        bytes.resize(4096, 0);
        if let Some(existing) = self.bone_buffers.get_mut(cache_key) {
            if existing.bytes != bytes {
                self.device
                    .write_buffer(existing.handle, &bytes, 0)
                    .map_err(|error| {
                        vec![Diagnostic::new(
                            "DX1219",
                            DiagnosticSeverity::Error,
                            "scene_renderer",
                            format!("update DX12 bone palette failed: {error:?}"),
                        )]
                    })?;
                existing.bytes = bytes;
            }
            return Ok(existing.handle);
        }
        let handle = self
            .device
            .create_buffer(&BufferDescriptor {
                size_bytes: 4096,
                usage_flags: render_core::BufferUsage::UNIFORM,
                memory_hint: MemoryHint::CpuToGpu,
                debug_label: Some(format!("bones-{cache_key}")),
            })
            .map_err(|error| {
                vec![Diagnostic::new(
                    "DX1218",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("create DX12 bone palette failed: {error:?}"),
                )]
            })?;
        if let Err(error) = self.device.write_buffer(handle, &bytes, 0) {
            self.device.destroy_buffer(handle);
            return Err(vec![Diagnostic::new(
                "DX1219",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("upload DX12 bone palette failed: {error:?}"),
            )]);
        }
        self.bone_buffers
            .insert(cache_key.to_owned(), Dx12BoneBuffer { handle, bytes });
        Ok(handle)
    }

    fn prepare_morphed_vertex_buffer(
        &mut self,
        cache_key: &str,
        mesh: &Dx12GpuMesh,
        target_set_id: &engine_serialize::AssetId,
        weights: &[f32],
    ) -> Result<BufferHandle, Vec<Diagnostic>> {
        let target_set = self
            .morph_target_sets
            .get(&target_set_id.id)
            .ok_or_else(|| {
                vec![Diagnostic::new(
                    "DX1263",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!(
                        "morph target set '{}' was referenced before a successful DX12 upload",
                        target_set_id.id
                    ),
                )]
            })?;
        if target_set.vertex_count != mesh.vertex_count {
            return Err(vec![Diagnostic::new(
                "DX1264",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!(
                    "morph target set '{}' has {} vertices but the skinned mesh has {}",
                    target_set_id.id, target_set.vertex_count, mesh.vertex_count
                ),
            )]);
        }
        if weights.iter().all(|weight| weight.abs() <= f32::EPSILON) {
            return Ok(mesh.vertex_buffer);
        }
        let stride = mesh.vertex_format.stride_bytes() as usize;
        let mut bytes = mesh.vertex_bytes.clone();
        for vertex_index in 0..mesh.vertex_count as usize {
            let base = vertex_index * stride;
            let read_vec3 = |source: &[u8], offset: usize| {
                glam::Vec3::new(
                    f32::from_ne_bytes(source[offset..offset + 4].try_into().unwrap()),
                    f32::from_ne_bytes(source[offset + 4..offset + 8].try_into().unwrap()),
                    f32::from_ne_bytes(source[offset + 8..offset + 12].try_into().unwrap()),
                )
            };
            let mut position = read_vec3(&mesh.vertex_bytes, base);
            let mut normal = read_vec3(&mesh.vertex_bytes, base + 12);
            for (target, weight) in target_set.targets.iter().zip(weights.iter().copied()) {
                position += glam::Vec3::from_array(target.position_deltas[vertex_index]) * weight;
                normal += glam::Vec3::from_array(target.normal_deltas[vertex_index]) * weight;
            }
            if normal.length_squared() > 1.0e-12 {
                normal = normal.normalize();
            }
            for (offset, value) in position
                .to_array()
                .into_iter()
                .chain(normal.to_array())
                .enumerate()
            {
                let start = base + offset * 4;
                bytes[start..start + 4].copy_from_slice(&value.to_ne_bytes());
            }
        }
        if let Some(existing) = self.morphed_vertex_buffers.get_mut(cache_key) {
            if existing.bytes != bytes {
                self.device
                    .write_buffer(existing.handle, &bytes, 0)
                    .map_err(|error| {
                        vec![Diagnostic::new(
                            "DX1266",
                            DiagnosticSeverity::Error,
                            "scene_renderer",
                            format!("update DX12 morphed vertex buffer failed: {error:?}"),
                        )]
                    })?;
                existing.bytes = bytes;
            }
            return Ok(existing.handle);
        }
        let handle = self
            .device
            .create_buffer(&BufferDescriptor {
                size_bytes: bytes.len() as u64,
                usage_flags: render_core::BufferUsage::VERTEX,
                memory_hint: MemoryHint::CpuToGpu,
                debug_label: Some(format!("morph-{cache_key}")),
            })
            .map_err(|error| {
                vec![Diagnostic::new(
                    "DX1265",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("create DX12 morphed vertex buffer failed: {error:?}"),
                )]
            })?;
        if let Err(error) = self.device.write_buffer(handle, &bytes, 0) {
            self.device.destroy_buffer(handle);
            return Err(vec![Diagnostic::new(
                "DX1266",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("upload DX12 morphed vertex buffer failed: {error:?}"),
            )]);
        }
        if let Err(error) = self
            .device
            .set_vertex_stride(handle, mesh.vertex_format.stride_bytes())
        {
            self.device.destroy_buffer(handle);
            return Err(vec![Diagnostic::new(
                "DX1267",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("set DX12 morphed vertex stride failed: {error:?}"),
            )]);
        }
        self.morphed_vertex_buffers.insert(
            cache_key.to_owned(),
            Dx12DynamicVertexBuffer {
                handle,
                capacity: bytes.len(),
                bytes,
            },
        );
        Ok(handle)
    }

    fn clear_morphed_vertex_buffers(&mut self) {
        for (_, buffer) in self.morphed_vertex_buffers.drain() {
            self.device.destroy_buffer(buffer.handle);
        }
    }

    fn prepare_particle_instance_buffer(
        &mut self,
        cache_key: &str,
        instances: &[engine_renderer::ParticleInstance],
    ) -> Result<Option<BufferHandle>, Vec<Diagnostic>> {
        if instances.is_empty() {
            return Ok(None);
        }
        let mut bytes = Vec::with_capacity(instances.len() * 32);
        for instance in instances {
            for value in [
                instance.position[0],
                instance.position[1],
                instance.position[2],
                instance.size,
                instance.rotation_radians,
                instance.normalized_age,
            ] {
                bytes.extend_from_slice(&value.to_ne_bytes());
            }
            bytes.extend_from_slice(&instance.color);
            bytes.extend_from_slice(&0_u32.to_ne_bytes());
        }
        if let Some(existing) = self.particle_instance_buffers.get_mut(cache_key) {
            if existing.capacity >= bytes.len() {
                if existing.bytes != bytes {
                    self.device
                        .write_buffer(existing.handle, &bytes, 0)
                        .map_err(|error| {
                            vec![Diagnostic::new(
                                "DX1270",
                                DiagnosticSeverity::Error,
                                "scene_renderer",
                                format!("update DX12 particle instance stream failed: {error:?}"),
                            )]
                        })?;
                    existing.bytes = bytes;
                }
                return Ok(Some(existing.handle));
            }
        }
        if let Some(old) = self.particle_instance_buffers.remove(cache_key) {
            self.device.destroy_buffer(old.handle);
        }
        let capacity = bytes.len().next_power_of_two();
        let handle = self
            .device
            .create_buffer(&BufferDescriptor {
                size_bytes: capacity as u64,
                usage_flags: render_core::BufferUsage::VERTEX,
                memory_hint: MemoryHint::CpuToGpu,
                debug_label: Some(format!("particles-{cache_key}")),
            })
            .map_err(|error| {
                vec![Diagnostic::new(
                    "DX1269",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("create DX12 particle instance stream failed: {error:?}"),
                )]
            })?;
        if let Err(error) = self.device.write_buffer(handle, &bytes, 0) {
            self.device.destroy_buffer(handle);
            return Err(vec![Diagnostic::new(
                "DX1270",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("upload DX12 particle instance stream failed: {error:?}"),
            )]);
        }
        if let Err(error) = self.device.set_vertex_stride(handle, 32) {
            self.device.destroy_buffer(handle);
            return Err(vec![Diagnostic::new(
                "DX1271",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("set DX12 particle instance stride failed: {error:?}"),
            )]);
        }
        self.particle_instance_buffers.insert(
            cache_key.to_owned(),
            Dx12DynamicVertexBuffer {
                handle,
                bytes,
                capacity,
            },
        );
        Ok(Some(handle))
    }

    fn clear_particle_instance_buffers(&mut self) {
        for (_, buffer) in self.particle_instance_buffers.drain() {
            self.device.destroy_buffer(buffer.handle);
        }
    }

    fn prepare_gpu_particle_parameter_buffer(
        &mut self,
        cache_key: &str,
        simulation: engine_renderer::GpuParticleSimulation,
    ) -> Result<BufferHandle, Vec<Diagnostic>> {
        let bytes = simulation.parameter_bytes().to_vec();
        if let Some(existing) = self.gpu_particle_parameter_buffers.get_mut(cache_key) {
            if existing.bytes != bytes {
                self.device
                    .write_buffer(existing.handle, &bytes, 0)
                    .map_err(|error| {
                        vec![Diagnostic::new(
                            "DX1286",
                            DiagnosticSeverity::Error,
                            "scene_renderer",
                            format!("update DX12 GPU-particle parameters failed: {error:?}"),
                        )]
                    })?;
                existing.bytes = bytes;
            }
            return Ok(existing.handle);
        }
        let handle = self
            .device
            .create_buffer(&BufferDescriptor {
                size_bytes: engine_renderer::GPU_PARTICLE_PARAMETER_SIZE as u64,
                usage_flags: render_core::BufferUsage::STORAGE,
                memory_hint: MemoryHint::CpuToGpu,
                debug_label: Some(format!("gpu-particles-{cache_key}")),
            })
            .map_err(|error| {
                vec![Diagnostic::new(
                    "DX1286",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("create DX12 GPU-particle parameters failed: {error:?}"),
                )]
            })?;
        if let Err(error) = self.device.write_buffer(handle, &bytes, 0) {
            self.device.destroy_buffer(handle);
            return Err(vec![Diagnostic::new(
                "DX1286",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("upload DX12 GPU-particle parameters failed: {error:?}"),
            )]);
        }
        self.gpu_particle_parameter_buffers.insert(
            cache_key.to_owned(),
            Dx12DynamicVertexBuffer {
                handle,
                bytes,
                capacity: engine_renderer::GPU_PARTICLE_PARAMETER_SIZE,
            },
        );
        Ok(handle)
    }

    fn prepare_clustered_light_buffers(
        &mut self,
        input: &RenderFrameInput,
        view: &engine_renderer::RenderView,
    ) -> Result<[BufferHandle; 4], Vec<Diagnostic>> {
        let light_refs = input.lights.iter().collect::<Vec<_>>();
        let clustered = engine_renderer::build_clustered_light_frame(
            &light_refs,
            view,
            self.width,
            self.height,
        );
        let light = update_dynamic_storage_buffer(
            &mut self.device,
            &mut self.clustered_light_buffer,
            &clustered.light_bytes,
            "dx12-clustered-lights",
        );
        let grid = update_dynamic_storage_buffer(
            &mut self.device,
            &mut self.clustered_grid_buffer,
            &clustered.cluster_grid_bytes,
            "dx12-cluster-grid",
        );
        let indices = update_dynamic_storage_buffer(
            &mut self.device,
            &mut self.clustered_index_buffer,
            &clustered.cluster_index_bytes,
            "dx12-cluster-indices",
        );
        let particle = update_dynamic_storage_buffer(
            &mut self.device,
            &mut self.gpu_particle_dummy_buffer,
            &[0_u8; engine_renderer::GPU_PARTICLE_PARAMETER_SIZE],
            "dx12-gpu-particle-dummy",
        );
        match (light, grid, indices, particle) {
            (Ok(light), Ok(grid), Ok(indices), Ok(particle)) => {
                Ok([light, grid, indices, particle])
            }
            results => Err(vec![Diagnostic::new(
                "DX1285",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!(
                    "prepare DX12 scene storage buffers failed: light={:?}, grid={:?}, indices={:?}, particles={:?}",
                    results.0.err(),
                    results.1.err(),
                    results.2.err(),
                    results.3.err()
                ),
            )]),
        }
    }

    fn directional_shadow_frame_data(
        input: &RenderFrameInput,
    ) -> Result<Option<Dx12ShadowFrameData>, Vec<Diagnostic>> {
        let Some(light) = input.lights.iter().find(|light| {
            light.kind == engine_renderer::LightKind::Directional
                && matches!(light.shadow_mode, ShadowMode::Hard | ShadowMode::Soft)
        }) else {
            return Ok(None);
        };
        let view = input.views.first().ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1247",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "directional shadow rendering requires a camera view",
            )]
        })?;
        let light_direction = glam::Vec3::from_array(light.direction);
        let length_squared = light_direction.length_squared();
        if !light_direction.is_finite() || !length_squared.is_finite() || length_squared <= 1.0e-12
        {
            return Err(vec![Diagnostic::new(
                "DX1248",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "directional shadow light direction must be finite and non-zero",
            )
            .entity(light.entity.clone())]);
        }
        let light_direction = light_direction / length_squared.sqrt();
        let camera_view = Mat4::from_cols_array(&view.view_matrix);
        let camera_projection = Mat4::from_cols_array(&view.projection_matrix);
        let camera_view_projection = camera_projection * camera_view;
        let determinant = camera_view_projection.determinant();
        if !determinant.is_finite() || determinant.abs() <= 1.0e-12 {
            return Err(vec![Diagnostic::new(
                "DX1249",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "camera view-projection matrix is not invertible for shadow fitting",
            )]);
        }
        let inverse = camera_view_projection.inverse();
        let mut world_corners = [glam::Vec3::ZERO; 8];
        let mut corner_index = 0;
        for depth in [0.0_f32, 1.0] {
            for y in [-1.0_f32, 1.0] {
                for x in [-1.0_f32, 1.0] {
                    let homogeneous = inverse * glam::vec4(x, y, depth, 1.0);
                    if !homogeneous.is_finite() || homogeneous.w.abs() <= 1.0e-8 {
                        return Err(vec![Diagnostic::new(
                            "DX1250",
                            DiagnosticSeverity::Error,
                            "scene_renderer",
                            "camera frustum is degenerate and cannot be fitted to a shadow map",
                        )]);
                    }
                    let corner = homogeneous.truncate() / homogeneous.w;
                    if !corner.is_finite() {
                        return Err(vec![Diagnostic::new(
                            "DX1250",
                            DiagnosticSeverity::Error,
                            "scene_renderer",
                            "camera frustum contains non-finite shadow corners",
                        )]);
                    }
                    world_corners[corner_index] = corner;
                    corner_index += 1;
                }
            }
        }
        let center = world_corners.iter().copied().sum::<glam::Vec3>() / 8.0;
        let radius = world_corners
            .iter()
            .map(|corner| corner.distance(center))
            .fold(0.0_f32, f32::max);
        if !center.is_finite() || !radius.is_finite() || radius <= 1.0e-5 {
            return Err(vec![Diagnostic::new(
                "DX1250",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "camera frustum has no finite extent for shadow fitting",
            )]);
        }
        let up = if light_direction.dot(glam::Vec3::Y).abs() > 0.99 {
            glam::Vec3::Z
        } else {
            glam::Vec3::Y
        };
        let light_position = center - light_direction * (radius * 2.0 + 1.0);
        let light_view = Mat4::look_at_rh(light_position, center, up);
        let mut minimum = glam::Vec3::splat(f32::MAX);
        let mut maximum = glam::Vec3::splat(f32::MIN);
        for corner in world_corners {
            let light_space = (light_view * corner.extend(1.0)).truncate();
            minimum = minimum.min(light_space);
            maximum = maximum.max(light_space);
        }
        let extent = maximum - minimum;
        if !extent.is_finite() || extent.min_element() <= 1.0e-5 {
            return Err(vec![Diagnostic::new(
                "DX1250",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "directional-light shadow bounds are degenerate",
            )]);
        }
        let pad_x = (extent.x * 0.025).max(1.0e-3);
        let pad_y = (extent.y * 0.025).max(1.0e-3);
        let pad_z = (extent.z * 0.025).max(1.0e-3);
        let near = (-maximum.z - pad_z).max(1.0e-4);
        let far = (-minimum.z + pad_z).max(near + 1.0e-3);
        let light_projection = Mat4::orthographic_rh(
            minimum.x - pad_x,
            maximum.x + pad_x,
            minimum.y - pad_y,
            maximum.y + pad_y,
            near,
            far,
        );
        Ok(Some(Dx12ShadowFrameData {
            light_view_projection: light_projection * light_view,
            light_direction_to_surface: -light_direction,
            soft: light.shadow_mode == ShadowMode::Soft,
        }))
    }

    fn record_directional_shadow_pass(
        &mut self,
        input: &RenderFrameInput,
    ) -> Result<(), Vec<Diagnostic>> {
        let Some(shadow_data) = Self::directional_shadow_frame_data(input)? else {
            self.shadow_frame_data = None;
            return Ok(());
        };
        let missing_meshes: Vec<&str> = input
            .drawables
            .iter()
            .filter(|drawable| drawable.cast_shadows)
            .map(|drawable| drawable.mesh.id.as_str())
            .chain(
                input
                    .skinned_items
                    .iter()
                    .filter(|item| item.cast_shadows)
                    .map(|item| item.mesh.id.as_str()),
            )
            .filter(|mesh_id| !self.meshes.contains_key(*mesh_id))
            .collect();
        if !missing_meshes.is_empty() {
            return Err(vec![Diagnostic::new(
                "DX1251",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!(
                    "shadow casters reference meshes that were not uploaded: {}",
                    missing_meshes.join(", ")
                ),
            )]);
        }
        let render_pass = self.shadow_render_pass.ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1245",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "DX12 directional shadow render pass is unavailable",
            )]
        })?;
        let framebuffer = self.shadow_framebuffer.ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1245",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "DX12 directional shadow framebuffer is unavailable",
            )]
        })?;
        let layout = self.shadow_pipeline_layout.ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1245",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "DX12 directional shadow root signature is unavailable",
            )]
        })?;
        let static_pipeline = self.shadow_pipeline.ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1245",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "DX12 directional shadow pipeline is unavailable",
            )]
        })?;
        let skinned_pipeline = self.skinned_shadow_pipeline.ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1245",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "DX12 skinned directional shadow pipeline is unavailable",
            )]
        })?;
        let mut frame = self.active_frame.take().ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1206",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "shadow pass called without an active DX12 frame",
            )]
        })?;
        frame.encoder.begin_render_pass(
            render_pass,
            framebuffer,
            (0, 0, 2048, 2048),
            [0.0; 4],
            Some(1.0),
        );
        frame.encoder.bind_pipeline(static_pipeline);
        for drawable in input
            .drawables
            .iter()
            .filter(|drawable| drawable.cast_shadows)
        {
            if matches!(
                self.material_surface(input, &drawable.material).0,
                Transparency::Blend | Transparency::Additive
            ) {
                continue;
            }
            let mesh = &self.meshes[&drawable.mesh.id];
            let world = Mat4::from_cols_array(&drawable.world_transform);
            let matrix = shadow_data.light_view_projection * world;
            let matrix_bytes = matrix_bytes(matrix);
            frame.encoder.push_constants(layout, 0x10, 0, &matrix_bytes);
            frame
                .encoder
                .bind_vertex_buffers(&[mesh.vertex_buffer], &[0]);
            frame
                .encoder
                .bind_index_buffer(mesh.index_buffer, 0, mesh.index_format);
            frame.encoder.draw_indexed(mesh.index_count, 1, 0, 0, 0);
            frame.draw_calls += 1;
            frame.triangles += u64::from(mesh.index_count / 3);
        }
        frame.encoder.bind_pipeline(skinned_pipeline);
        for (item_index, item) in input
            .skinned_items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.cast_shadows)
        {
            if matches!(
                self.material_surface(input, &item.material).0,
                Transparency::Blend | Transparency::Additive
            ) {
                continue;
            }
            let mesh = self.meshes[&item.mesh.id].clone();
            let world = Mat4::from_cols_array(&item.world_transform);
            let matrix = shadow_data.light_view_projection * world;
            let matrix_bytes = matrix_bytes(matrix);
            frame.encoder.push_constants(layout, 0x10, 0, &matrix_bytes);
            let cache_key = format!(
                "{}:{}:{}",
                item.skeleton.id,
                item.entity.as_deref().unwrap_or("anonymous"),
                item_index
            );
            let vertex_buffer = match item.morph_target_set.as_ref() {
                Some(target_set) => {
                    match self.prepare_morphed_vertex_buffer(
                        &format!("{cache_key}:{}", item.mesh.id),
                        &mesh,
                        target_set,
                        &item.morph_weights,
                    ) {
                        Ok(buffer) => buffer,
                        Err(diagnostics) => {
                            self.active_frame = Some(frame);
                            return Err(diagnostics);
                        }
                    }
                }
                None => mesh.vertex_buffer,
            };
            let bone_buffer = match self.prepare_bone_buffer(&cache_key, &item.bone_palette) {
                Ok(buffer) => buffer,
                Err(diagnostics) => {
                    self.active_frame = Some(frame);
                    return Err(diagnostics);
                }
            };
            if !frame.encoder.bind_uniform_buffer(layout, bone_buffer) {
                self.active_frame = Some(frame);
                return Err(vec![Diagnostic::new(
                    "DX1252",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    "DX12 skinned shadow pass could not bind its bone palette",
                )]);
            }
            frame.encoder.bind_vertex_buffers(&[vertex_buffer], &[0]);
            frame
                .encoder
                .bind_index_buffer(mesh.index_buffer, 0, mesh.index_format);
            frame.encoder.draw_indexed(mesh.index_count, 1, 0, 0, 0);
            frame.draw_calls += 1;
            frame.triangles += u64::from(mesh.index_count / 3);
        }
        frame.encoder.end_render_pass();
        self.active_frame = Some(frame);
        self.shadow_frame_data = Some(shadow_data);
        Ok(())
    }

    fn record_forward_pass(
        &mut self,
        input: &RenderFrameInput,
        view_id: Option<u32>,
    ) -> Result<(), Vec<Diagnostic>> {
        let view = view_id
            .and_then(|id| input.views.iter().find(|view| view.view_id == id))
            .or_else(|| input.views.first());
        if view.is_none()
            && (!input.drawables.is_empty()
                || !input.skinned_items.is_empty()
                || !input.particle_batches.is_empty())
        {
            return Err(vec![Diagnostic::new(
                "DX1211",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "cannot render drawables without a camera view",
            )]);
        }
        let missing_meshes: Vec<&str> = input
            .drawables
            .iter()
            .map(|drawable| &drawable.mesh)
            .chain(input.skinned_items.iter().map(|item| &item.mesh))
            .chain(input.particle_batches.iter().map(|batch| &batch.mesh))
            .filter_map(|mesh| (!self.meshes.contains_key(&mesh.id)).then_some(mesh.id.as_str()))
            .collect();
        if !missing_meshes.is_empty() {
            return Err(vec![Diagnostic::new(
                "DX1212",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!(
                    "drawables reference meshes that were not uploaded: {}",
                    missing_meshes.join(", ")
                ),
            )]);
        }
        let invalid_static_meshes: Vec<&str> = input
            .drawables
            .iter()
            .filter_map(|drawable| {
                (self.meshes[&drawable.mesh.id].vertex_format != RendererMeshVertexFormat::Pbr32)
                    .then_some(drawable.mesh.id.as_str())
            })
            .collect();
        if !invalid_static_meshes.is_empty() {
            return Err(vec![Diagnostic::new(
                "DX1217",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!(
                    "static drawables require Pbr32 meshes, got skinned layout for: {}",
                    invalid_static_meshes.join(", ")
                ),
            )]);
        }
        let invalid_skinned_meshes: Vec<&str> = input
            .skinned_items
            .iter()
            .filter_map(|item| {
                (self.meshes[&item.mesh.id].vertex_format != RendererMeshVertexFormat::Skinned64)
                    .then_some(item.mesh.id.as_str())
            })
            .collect();
        if !invalid_skinned_meshes.is_empty() {
            return Err(vec![Diagnostic::new(
                "DX1214",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!(
                    "skinned drawables require Skinned64 meshes, got static layout for: {}",
                    invalid_skinned_meshes.join(", ")
                ),
            )]);
        }
        let invalid_particle_meshes: Vec<&str> = input
            .particle_batches
            .iter()
            .filter_map(|batch| {
                (self.meshes[&batch.mesh.id].vertex_format != RendererMeshVertexFormat::Pbr32)
                    .then_some(batch.mesh.id.as_str())
            })
            .collect();
        if !invalid_particle_meshes.is_empty() {
            return Err(vec![Diagnostic::new(
                "DX1272",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!(
                    "particle batches require Pbr32 meshes: {}",
                    invalid_particle_meshes.join(", ")
                ),
            )]);
        }
        for item in &input.skinned_items {
            let count = match item.bone_palette_layout {
                engine_renderer::BonePaletteLayout::Full4x4 { count } => count,
                engine_renderer::BonePaletteLayout::Packed3x4 { .. } => {
                    return Err(vec![Diagnostic::new(
                        "DX1227",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        "DX12 skinning currently requires Full4x4 bone palettes",
                    )]);
                }
            };
            if count as usize != item.bone_palette.len()
                || item.bone_palette.is_empty()
                || item.bone_palette.len() > 64
                || !item
                    .bone_palette
                    .iter()
                    .flatten()
                    .all(|value| value.is_finite())
            {
                return Err(vec![Diagnostic::new(
                    "DX1228",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!(
                        "skinned item '{}' must contain 1..=64 finite Full4x4 bones and a matching count",
                        item.skeleton.id
                    ),
                )]);
            }
        }
        for (material_id, entity) in input
            .drawables
            .iter()
            .map(|item| (&item.material, item.entity.as_ref()))
            .chain(
                input
                    .skinned_items
                    .iter()
                    .map(|item| (&item.material, item.entity.as_ref())),
            )
            .chain(
                input
                    .particle_batches
                    .iter()
                    .map(|batch| (&batch.material, batch.emitter.as_ref())),
            )
        {
            let texture_ids = self.material_texture_ids(input, material_id);
            for texture_id in texture_ids.iter().flatten() {
                if !self.textures.contains_key(texture_id) {
                    return Err(vec![Diagnostic::new(
                        "DX1215",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        format!(
                            "material '{}' references texture '{}' before a successful DX12 upload",
                            material_id.id, texture_id
                        ),
                    )
                    .entity(entity.cloned())]);
                }
            }
        }
        let layout = self.pipeline_layout.ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1213",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "DX12 pipeline layout is unavailable",
            )]
        })?;
        let missing_pipeline = || {
            vec![Diagnostic::new(
                "DX1203",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "DX12 forward surface pipelines are unavailable",
            )]
        };
        let static_pipelines = [
            self.pipeline.ok_or_else(missing_pipeline)?,
            self.double_sided_pipeline.ok_or_else(missing_pipeline)?,
            self.blend_pipeline.ok_or_else(missing_pipeline)?,
            self.blend_double_sided_pipeline
                .ok_or_else(missing_pipeline)?,
            self.additive_pipeline.ok_or_else(missing_pipeline)?,
            self.additive_double_sided_pipeline
                .ok_or_else(missing_pipeline)?,
            self.oit_pipeline.ok_or_else(missing_pipeline)?,
            self.oit_double_sided_pipeline
                .ok_or_else(missing_pipeline)?,
        ];
        let weighted_oit = input.render_options.transparency_mode
            == engine_renderer::TransparencyMode::WeightedBlendedOit;
        let shadow_texture = self.shadow_texture.ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1245",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "DX12 shadow texture is unavailable",
            )]
        })?;
        let shadow_frame_data = self.shadow_frame_data;
        let hdr_render_pass = self.hdr_render_pass.ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1255",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "DX12 HDR forward render pass is unavailable",
            )]
        })?;
        let hdr_framebuffer = self.hdr_framebuffer.ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1255",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "DX12 HDR forward framebuffer is unavailable",
            )]
        })?;
        let skinned_pipelines = if input.skinned_items.is_empty() {
            None
        } else {
            let missing = || {
                vec![Diagnostic::new(
                    "DX1229",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    "DX12 skinned surface pipelines are unavailable",
                )]
            };
            Some([
                self.skinned_pipeline.ok_or_else(missing)?,
                self.skinned_double_sided_pipeline.ok_or_else(missing)?,
                self.skinned_blend_pipeline.ok_or_else(missing)?,
                self.skinned_blend_double_sided_pipeline
                    .ok_or_else(missing)?,
                self.skinned_additive_pipeline.ok_or_else(missing)?,
                self.skinned_additive_double_sided_pipeline
                    .ok_or_else(missing)?,
                self.skinned_oit_pipeline.ok_or_else(missing)?,
                self.skinned_oit_double_sided_pipeline.ok_or_else(missing)?,
            ])
        };
        let particle_pipelines = if input.particle_batches.is_empty() {
            None
        } else {
            let missing = || {
                vec![Diagnostic::new(
                    "DX1273",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    "DX12 particle billboard pipelines are unavailable",
                )]
            };
            Some([
                self.particle_pipeline.ok_or_else(missing)?,
                self.particle_additive_pipeline.ok_or_else(missing)?,
                self.particle_oit_pipeline.ok_or_else(missing)?,
                self.gpu_particle_pipeline.ok_or_else(missing)?,
                self.gpu_particle_additive_pipeline.ok_or_else(missing)?,
                self.gpu_particle_oit_pipeline.ok_or_else(missing)?,
            ])
        };
        let mut frame = self.active_frame.take().ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1206",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "execute_pass called without an active DX12 frame",
            )]
        })?;

        let mut current_pipeline = static_pipelines[0];
        let clear_color = view.map_or([0.0, 0.0, 0.0, 1.0], |view| view.clear_color);
        if !frame.hdr_pass_active {
            frame.encoder.begin_render_pass_with_color_clears(
                hdr_render_pass,
                hdr_framebuffer,
                (0, 0, self.width, self.height),
                &[clear_color, [0.0; 4], [0.0; 4]],
                Some(1.0),
            );
            frame.hdr_pass_active = true;
        }
        frame.encoder.bind_pipeline(current_pipeline);
        let viewport = view.map_or(engine_renderer::Rect::FULL, |view| {
            view.viewport_rect_normalized
        });
        let x = viewport.min[0] * self.width as f32;
        let y = viewport.min[1] * self.height as f32;
        let width = viewport.width() * self.width as f32;
        let height = viewport.height() * self.height as f32;
        frame.encoder.set_viewport(x, y, width, height, 0.0, 1.0);
        let scissor_x = x.floor();
        let scissor_y = y.floor();
        frame.encoder.set_scissor(
            scissor_x as i32,
            scissor_y as i32,
            ((x + width).ceil() - scissor_x) as u32,
            ((y + height).ceil() - scissor_y) as u32,
        );

        if let Some(view) = view {
            let view_matrix = Mat4::from_cols_array(&view.view_matrix);
            let projection_matrix = Mat4::from_cols_array(&view.projection_matrix);
            let camera_position = view_matrix.inverse().w_axis.truncate();
            let cluster_buffers = match self.prepare_clustered_light_buffers(input, view) {
                Ok(buffers) => buffers,
                Err(diagnostics) => {
                    self.active_frame = Some(frame);
                    return Err(diagnostics);
                }
            };
            let (environment_texture, environment_constants) =
                match self.environment_binding(input, camera_position) {
                    Ok(binding) => binding,
                    Err(diagnostics) => {
                        self.active_frame = Some(frame);
                        return Err(diagnostics);
                    }
                };
            if view.clear_flags == engine_renderer::ClearFlags::Skybox
                && select_environment_map(&input.render_options.environment, camera_position)
                    .is_some()
            {
                let Some(skybox_pipeline) = self.skybox_pipeline else {
                    self.active_frame = Some(frame);
                    return Err(vec![Diagnostic::new(
                        "DX1282",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        "DX12 skybox pipeline is unavailable",
                    )]);
                };
                frame.encoder.bind_pipeline(skybox_pipeline);
                let inverse_view_projection = (projection_matrix * view_matrix).inverse();
                frame.encoder.push_constants(
                    layout,
                    0x10,
                    0,
                    &matrix_bytes(inverse_view_projection),
                );
                frame.encoder.push_constants(
                    layout,
                    0x10,
                    64,
                    &float4_bytes([camera_position.x, camera_position.y, camera_position.z, 1.0]),
                );
                frame
                    .encoder
                    .push_constants(layout, 0x20, 224, &environment_constants);
                let empty_textures: [Option<String>; 5] = std::array::from_fn(|_| None);
                let texture_table = self.material_texture_table(
                    &empty_textures,
                    shadow_texture,
                    environment_texture,
                );
                if !frame
                    .encoder
                    .bind_scene_resource_set(layout, &texture_table, &cluster_buffers)
                {
                    self.active_frame = Some(frame);
                    return Err(vec![Diagnostic::new(
                        "DX1283",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        "DX12 skybox pass could not bind the selected environment",
                    )]);
                }
                frame.encoder.draw(3, 1, 0, 0);
                frame.draw_calls += 1;
                frame.triangles += 1;
                frame.encoder.bind_pipeline(current_pipeline);
            }
            for transparent_phase in [false, true] {
                let mut ordered_drawables = Vec::with_capacity(input.drawables.len());
                let mut blended_drawables = Vec::new();
                for drawable in &input.drawables {
                    let (transparency, _) = self.material_surface(input, &drawable.material);
                    let transparent =
                        matches!(transparency, Transparency::Blend | Transparency::Additive);
                    if transparent != transparent_phase {
                        continue;
                    }
                    if transparent {
                        let translation = Mat4::from_cols_array(&drawable.world_transform)
                            .w_axis
                            .truncate();
                        blended_drawables
                            .push(((translation - camera_position).length_squared(), drawable));
                    } else {
                        ordered_drawables.push(drawable);
                    }
                }
                if !weighted_oit {
                    blended_drawables.sort_by(|left, right| right.0.total_cmp(&left.0));
                }
                ordered_drawables
                    .extend(blended_drawables.into_iter().map(|(_, drawable)| drawable));

                for drawable in ordered_drawables {
                    // Existence was validated before recording any draw command.
                    let mesh = &self.meshes[&drawable.mesh.id];
                    let world_matrix = Mat4::from_cols_array(&drawable.world_transform);
                    let (transparency, double_sided) =
                        self.material_surface(input, &drawable.material);
                    let next_pipeline = static_pipelines
                        [surface_variant_index(&transparency, double_sided, weighted_oit)];
                    if next_pipeline != current_pipeline {
                        frame.encoder.bind_pipeline(next_pipeline);
                        current_pipeline = next_pipeline;
                    }
                    let mvp = (projection_matrix * view_matrix * world_matrix).to_cols_array();
                    let mut mvp_bytes = [0_u8; 64];
                    for (destination, value) in mvp_bytes.chunks_exact_mut(4).zip(mvp) {
                        destination.copy_from_slice(&value.to_ne_bytes());
                    }
                    frame.encoder.push_constants(layout, 0x10, 0, &mvp_bytes);
                    let input_material = input
                        .materials
                        .iter()
                        .find(|binding| binding.material_id == drawable.material);
                    let texture_ids = self.material_texture_ids(input, &drawable.material);
                    let texture_flags = material_texture_flags_from_ids(&texture_ids);
                    let mut material_constants = input_material
                        .map(|binding| {
                            material_constants_from_bytes(
                                &binding.uniforms.bytes,
                                texture_ids[0].is_some(),
                                &binding.transparency,
                                weighted_oit,
                            )
                        })
                        .or_else(|| {
                            self.materials
                                .get(&drawable.material.id)
                                .map(|material| material.constants)
                        })
                        .unwrap_or_else(default_material_constants);
                    set_material_surface_flags(
                        &mut material_constants,
                        texture_ids[0].is_some(),
                        &transparency,
                        weighted_oit,
                    );
                    frame
                        .encoder
                        .push_constants(layout, 0x20, 64, &material_constants);
                    let (light_matrix, shadow_parameters, light_direction) =
                        shadow_scene_constants(shadow_frame_data, world_matrix);
                    frame
                        .encoder
                        .push_constants(layout, 0x30, 96, &light_matrix);
                    frame
                        .encoder
                        .push_constants(layout, 0x20, 160, &shadow_parameters);
                    frame
                        .encoder
                        .push_constants(layout, 0x20, 176, &light_direction);
                    let emissive_constants = input_material
                        .map(|binding| {
                            emissive_constants_from_bytes(&binding.uniforms.bytes, texture_flags)
                        })
                        .or_else(|| {
                            self.materials
                                .get(&drawable.material.id)
                                .map(|material| material.emissive_constants)
                        })
                        .unwrap_or_else(default_emissive_constants);
                    frame
                        .encoder
                        .push_constants(layout, 0x20, 192, &emissive_constants);
                    let advanced_constants = input_material
                        .map(|binding| advanced_constants_from_bytes(&binding.uniforms.bytes))
                        .or_else(|| {
                            self.materials
                                .get(&drawable.material.id)
                                .map(|material| material.advanced_constants)
                        })
                        .unwrap_or_else(default_advanced_constants);
                    frame
                        .encoder
                        .push_constants(layout, 0x20, 208, &advanced_constants);
                    frame
                        .encoder
                        .push_constants(layout, 0x20, 224, &environment_constants);
                    let texture_table = self.material_texture_table(
                        &texture_ids,
                        shadow_texture,
                        environment_texture,
                    );
                    if !frame.encoder.bind_scene_resource_set(
                        layout,
                        &texture_table,
                        &cluster_buffers,
                    ) {
                        self.active_frame = Some(frame);
                        return Err(vec![Diagnostic::new(
                        "DX1216",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        "DX12 forward pass could not bind its seven-texture material/environment table",
                    )]);
                    }
                    frame
                        .encoder
                        .bind_vertex_buffers(&[mesh.vertex_buffer], &[0]);
                    frame
                        .encoder
                        .bind_index_buffer(mesh.index_buffer, 0, mesh.index_format);
                    frame.encoder.draw_indexed(mesh.index_count, 1, 0, 0, 0);
                    frame.draw_calls += 1;
                    frame.triangles += u64::from(mesh.index_count / 3);
                }

                let mut ordered_skinned = Vec::with_capacity(input.skinned_items.len());
                let mut blended_skinned = Vec::new();
                for (item_index, item) in input.skinned_items.iter().enumerate() {
                    let (transparency, _) = self.material_surface(input, &item.material);
                    let transparent =
                        matches!(transparency, Transparency::Blend | Transparency::Additive);
                    if transparent != transparent_phase {
                        continue;
                    }
                    if transparent {
                        let translation = Mat4::from_cols_array(&item.world_transform)
                            .w_axis
                            .truncate();
                        blended_skinned.push((
                            (translation - camera_position).length_squared(),
                            item_index,
                            item,
                        ));
                    } else {
                        ordered_skinned.push((item_index, item));
                    }
                }
                if !weighted_oit {
                    blended_skinned.sort_by(|left, right| right.0.total_cmp(&left.0));
                }
                ordered_skinned.extend(
                    blended_skinned
                        .into_iter()
                        .map(|(_, item_index, item)| (item_index, item)),
                );

                for (item_index, item) in ordered_skinned {
                    let mesh = self.meshes[&item.mesh.id].clone();
                    let world_matrix = Mat4::from_cols_array(&item.world_transform);
                    let (transparency, double_sided) = self.material_surface(input, &item.material);
                    let next_pipeline = skinned_pipelines
                        .expect("skinned pipelines exist for a non-empty skinned list")
                        [surface_variant_index(&transparency, double_sided, weighted_oit)];
                    if next_pipeline != current_pipeline {
                        frame.encoder.bind_pipeline(next_pipeline);
                        current_pipeline = next_pipeline;
                    }
                    let mvp = (projection_matrix * view_matrix * world_matrix).to_cols_array();
                    let mut mvp_bytes = [0_u8; 64];
                    for (destination, value) in mvp_bytes.chunks_exact_mut(4).zip(mvp) {
                        destination.copy_from_slice(&value.to_ne_bytes());
                    }
                    frame.encoder.push_constants(layout, 0x10, 0, &mvp_bytes);

                    let input_material = input
                        .materials
                        .iter()
                        .find(|binding| binding.material_id == item.material);
                    let texture_ids = self.material_texture_ids(input, &item.material);
                    let texture_flags = material_texture_flags_from_ids(&texture_ids);
                    let mut material_constants = input_material
                        .map(|binding| {
                            material_constants_from_bytes(
                                &binding.uniforms.bytes,
                                texture_ids[0].is_some(),
                                &binding.transparency,
                                weighted_oit,
                            )
                        })
                        .or_else(|| {
                            self.materials
                                .get(&item.material.id)
                                .map(|material| material.constants)
                        })
                        .unwrap_or_else(default_material_constants);
                    set_material_surface_flags(
                        &mut material_constants,
                        texture_ids[0].is_some(),
                        &transparency,
                        weighted_oit,
                    );
                    frame
                        .encoder
                        .push_constants(layout, 0x20, 64, &material_constants);
                    let (light_matrix, shadow_parameters, light_direction) =
                        shadow_scene_constants(shadow_frame_data, world_matrix);
                    frame
                        .encoder
                        .push_constants(layout, 0x30, 96, &light_matrix);
                    frame
                        .encoder
                        .push_constants(layout, 0x20, 160, &shadow_parameters);
                    frame
                        .encoder
                        .push_constants(layout, 0x20, 176, &light_direction);
                    let emissive_constants = input_material
                        .map(|binding| {
                            emissive_constants_from_bytes(&binding.uniforms.bytes, texture_flags)
                        })
                        .or_else(|| {
                            self.materials
                                .get(&item.material.id)
                                .map(|material| material.emissive_constants)
                        })
                        .unwrap_or_else(default_emissive_constants);
                    frame
                        .encoder
                        .push_constants(layout, 0x20, 192, &emissive_constants);
                    let advanced_constants = input_material
                        .map(|binding| advanced_constants_from_bytes(&binding.uniforms.bytes))
                        .or_else(|| {
                            self.materials
                                .get(&item.material.id)
                                .map(|material| material.advanced_constants)
                        })
                        .unwrap_or_else(default_advanced_constants);
                    frame
                        .encoder
                        .push_constants(layout, 0x20, 208, &advanced_constants);
                    frame
                        .encoder
                        .push_constants(layout, 0x20, 224, &environment_constants);
                    let texture_table = self.material_texture_table(
                        &texture_ids,
                        shadow_texture,
                        environment_texture,
                    );
                    if !frame.encoder.bind_scene_resource_set(
                        layout,
                        &texture_table,
                        &cluster_buffers,
                    ) {
                        self.active_frame = Some(frame);
                        return Err(vec![Diagnostic::new(
                        "DX1216",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        "DX12 skinned pass could not bind its seven-texture material/environment table",
                    )]);
                    }

                    let cache_key = format!(
                        "{}:{}:{}",
                        item.skeleton.id,
                        item.entity.as_deref().unwrap_or("anonymous"),
                        item_index
                    );
                    let vertex_buffer = match item.morph_target_set.as_ref() {
                        Some(target_set) => {
                            match self.prepare_morphed_vertex_buffer(
                                &format!("{cache_key}:{}", item.mesh.id),
                                &mesh,
                                target_set,
                                &item.morph_weights,
                            ) {
                                Ok(buffer) => buffer,
                                Err(diagnostics) => {
                                    self.active_frame = Some(frame);
                                    return Err(diagnostics);
                                }
                            }
                        }
                        None => mesh.vertex_buffer,
                    };
                    let bone_buffer = match self.prepare_bone_buffer(&cache_key, &item.bone_palette)
                    {
                        Ok(buffer) => buffer,
                        Err(diagnostics) => {
                            self.active_frame = Some(frame);
                            return Err(diagnostics);
                        }
                    };
                    if !frame.encoder.bind_uniform_buffer(layout, bone_buffer) {
                        self.active_frame = Some(frame);
                        return Err(vec![Diagnostic::new(
                            "DX1230",
                            DiagnosticSeverity::Error,
                            "scene_renderer",
                            "DX12 skinned pipeline could not bind its bone palette",
                        )]);
                    }
                    frame.encoder.bind_vertex_buffers(&[vertex_buffer], &[0]);
                    frame
                        .encoder
                        .bind_index_buffer(mesh.index_buffer, 0, mesh.index_format);
                    frame.encoder.draw_indexed(mesh.index_count, 1, 0, 0, 0);
                    frame.draw_calls += 1;
                    frame.triangles += u64::from(mesh.index_count / 3);
                }

                if transparent_phase {
                    if let Some(particle_pipelines) = particle_pipelines {
                        let mut current_particle_pipeline = particle_pipelines[0];
                        frame.encoder.bind_pipeline(current_particle_pipeline);
                        let view_projection = projection_matrix * view_matrix;
                        let view_projection_bytes = matrix_bytes(view_projection);
                        let camera_world_bytes = matrix_bytes(view_matrix.inverse());
                        let shadow_disabled = float4_bytes([0.0, 0.0, 0.0, 0.0]);
                        let (_, _, light_direction) =
                            shadow_scene_constants(shadow_frame_data, Mat4::IDENTITY);
                        let mut ordered_batches: Vec<(
                            f32,
                            usize,
                            &engine_renderer::ParticleBatch,
                        )> = input
                            .particle_batches
                            .iter()
                            .enumerate()
                            .map(|(index, batch)| {
                                let center = glam::Vec3::from_array([
                                    (batch.bounds.min[0] + batch.bounds.max[0]) * 0.5,
                                    (batch.bounds.min[1] + batch.bounds.max[1]) * 0.5,
                                    (batch.bounds.min[2] + batch.bounds.max[2]) * 0.5,
                                ]);
                                ((center - camera_position).length_squared(), index, batch)
                            })
                            .collect();
                        if !weighted_oit {
                            ordered_batches.sort_by(|left, right| right.0.total_cmp(&left.0));
                        }
                        for (_, batch_index, batch) in ordered_batches {
                            let (transparency, _) = self.material_surface(input, &batch.material);
                            let gpu_simulation = batch.gpu_simulation;
                            let weighted_batch =
                                weighted_oit && transparency == Transparency::Blend;
                            let next_particle_pipeline = match (
                                gpu_simulation.is_some(),
                                transparency == Transparency::Additive,
                                weighted_batch,
                            ) {
                                (false, true, _) => particle_pipelines[1],
                                (false, false, true) => particle_pipelines[2],
                                (false, false, false) => particle_pipelines[0],
                                (true, true, _) => particle_pipelines[4],
                                (true, false, true) => particle_pipelines[5],
                                (true, false, false) => particle_pipelines[3],
                            };
                            if next_particle_pipeline != current_particle_pipeline {
                                frame.encoder.bind_pipeline(next_particle_pipeline);
                                current_particle_pipeline = next_particle_pipeline;
                            }
                            let mesh = self.meshes[&batch.mesh.id].clone();
                            let cache_key = format!(
                                "{}:{}:{}",
                                batch.mesh.id,
                                batch.emitter.as_deref().unwrap_or("anonymous"),
                                batch_index
                            );
                            let instance_buffer = if gpu_simulation.is_none() {
                                match self
                                    .prepare_particle_instance_buffer(&cache_key, &batch.instances)
                                {
                                    Ok(Some(buffer)) => Some(buffer),
                                    Ok(None) => continue,
                                    Err(diagnostics) => {
                                        self.active_frame = Some(frame);
                                        return Err(diagnostics);
                                    }
                                }
                            } else {
                                None
                            };
                            let input_material = input
                                .materials
                                .iter()
                                .find(|binding| binding.material_id == batch.material);
                            let texture_ids = self.material_texture_ids(input, &batch.material);
                            let texture_flags = material_texture_flags_from_ids(&texture_ids);
                            let mut material_constants = input_material
                                .map(|binding| {
                                    material_constants_from_bytes(
                                        &binding.uniforms.bytes,
                                        texture_ids[0].is_some(),
                                        &binding.transparency,
                                        weighted_oit,
                                    )
                                })
                                .or_else(|| {
                                    self.materials
                                        .get(&batch.material.id)
                                        .map(|material| material.constants)
                                })
                                .unwrap_or_else(default_material_constants);
                            set_material_surface_flags(
                                &mut material_constants,
                                texture_ids[0].is_some(),
                                &transparency,
                                weighted_oit,
                            );
                            let emissive_constants = input_material
                                .map(|binding| {
                                    emissive_constants_from_bytes(
                                        &binding.uniforms.bytes,
                                        texture_flags,
                                    )
                                })
                                .or_else(|| {
                                    self.materials
                                        .get(&batch.material.id)
                                        .map(|material| material.emissive_constants)
                                })
                                .unwrap_or_else(default_emissive_constants);
                            let advanced_constants = input_material
                                .map(|binding| {
                                    advanced_constants_from_bytes(&binding.uniforms.bytes)
                                })
                                .or_else(|| {
                                    self.materials
                                        .get(&batch.material.id)
                                        .map(|material| material.advanced_constants)
                                })
                                .unwrap_or_else(default_advanced_constants);
                            frame
                                .encoder
                                .push_constants(layout, 0x10, 0, &view_projection_bytes);
                            frame
                                .encoder
                                .push_constants(layout, 0x20, 64, &material_constants);
                            frame
                                .encoder
                                .push_constants(layout, 0x10, 96, &camera_world_bytes);
                            frame
                                .encoder
                                .push_constants(layout, 0x20, 160, &shadow_disabled);
                            frame
                                .encoder
                                .push_constants(layout, 0x20, 176, &light_direction);
                            frame
                                .encoder
                                .push_constants(layout, 0x20, 192, &emissive_constants);
                            frame
                                .encoder
                                .push_constants(layout, 0x20, 208, &advanced_constants);
                            frame
                                .encoder
                                .push_constants(layout, 0x20, 224, &environment_constants);
                            let texture_table = self.material_texture_table(
                                &texture_ids,
                                shadow_texture,
                                environment_texture,
                            );
                            let mut particle_resources = cluster_buffers;
                            if let Some(simulation) = gpu_simulation {
                                particle_resources[3] = match self
                                    .prepare_gpu_particle_parameter_buffer(&cache_key, simulation)
                                {
                                    Ok(buffer) => buffer,
                                    Err(diagnostics) => {
                                        self.active_frame = Some(frame);
                                        return Err(diagnostics);
                                    }
                                };
                            }
                            if !frame.encoder.bind_scene_resource_set(
                                layout,
                                &texture_table,
                                &particle_resources,
                            ) {
                                self.active_frame = Some(frame);
                                return Err(vec![Diagnostic::new(
                            "DX1274",
                            DiagnosticSeverity::Error,
                            "scene_renderer",
                            "DX12 particle pass could not bind its material/environment table",
                        )]);
                            }
                            if let Some(instance_buffer) = instance_buffer {
                                frame.encoder.bind_vertex_buffers(
                                    &[mesh.vertex_buffer, instance_buffer],
                                    &[0, 0],
                                );
                            } else {
                                frame
                                    .encoder
                                    .bind_vertex_buffers(&[mesh.vertex_buffer], &[0]);
                            }
                            frame.encoder.bind_index_buffer(
                                mesh.index_buffer,
                                0,
                                mesh.index_format,
                            );
                            let instance_count = gpu_simulation.map_or_else(
                                || u32::try_from(batch.instances.len()).unwrap_or(u32::MAX),
                                |simulation| simulation.draw_range().1,
                            );
                            frame
                                .encoder
                                .draw_indexed(mesh.index_count, instance_count, 0, 0, 0);
                            frame.draw_calls += 1;
                            frame.triangles +=
                                u64::from(mesh.index_count / 3) * u64::from(instance_count);
                        }
                    }
                }
            }
        }

        self.active_frame = Some(frame);
        Ok(())
    }

    fn record_tone_map_pass(&mut self, input: &RenderFrameInput) -> Result<(), Vec<Diagnostic>> {
        let layout = self.tone_map_pipeline_layout.ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1256",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "DX12 tone-map root signature is unavailable",
            )]
        })?;
        let pipeline = self.tone_map_pipeline.ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1256",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "DX12 tone-map pipeline is unavailable",
            )]
        })?;
        let hdr_texture = self.hdr_texture.ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1256",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "DX12 HDR color target is unavailable",
            )]
        })?;
        let oit_accum = self.oit_accum_texture.ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1256",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "DX12 OIT accumulation target is unavailable",
            )]
        })?;
        let oit_optical_depth = self.oit_optical_depth_texture.ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1256",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "DX12 OIT optical-depth target is unavailable",
            )]
        })?;
        let constants = tone_map_constants(input)?;
        let mut frame = self.active_frame.take().ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1206",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "tone-map pass called without an active DX12 frame",
            )]
        })?;
        frame.encoder.end_render_pass();
        frame.encoder.bind_pipeline(pipeline);
        frame
            .encoder
            .set_viewport(0.0, 0.0, self.width as f32, self.height as f32, 0.0, 1.0);
        frame.encoder.set_scissor(0, 0, self.width, self.height);
        frame.encoder.push_constants(layout, 0x20, 0, &constants);
        if !frame
            .encoder
            .bind_sampled_texture_set(layout, &[hdr_texture, oit_accum, oit_optical_depth])
        {
            self.active_frame = Some(frame);
            return Err(vec![Diagnostic::new(
                "DX1257",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "DX12 tone-map pass could not bind the HDR/OIT target set",
            )]);
        }
        frame.encoder.draw(3, 1, 0, 0);
        frame.draw_calls += 1;
        frame.triangles += 1;
        self.active_frame = Some(frame);
        Ok(())
    }

    fn record_ui_overlay_pass(&mut self, input: &RenderFrameInput) -> Result<(), Vec<Diagnostic>> {
        if input.ui_batches.is_empty() {
            return Ok(());
        }
        let prepared =
            prepare_dx12_ui(&input.ui_batches, self.width, self.height).map_err(|message| {
                vec![Diagnostic::new(
                    "DX1275",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    message,
                )]
            })?;
        if prepared.draws.is_empty() {
            return Ok(());
        }
        for draw in &prepared.draws {
            if let Some(texture_id) = draw.texture_id.as_ref() {
                if !self.textures.contains_key(texture_id) {
                    return Err(vec![Diagnostic::new(
                        "DX1276",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        format!(
                            "UI batch references texture '{texture_id}' before a successful DX12 upload"
                        ),
                    )]);
                }
            }
        }
        let required = prepared.vertex_bytes.len();
        if self
            .ui_vertex_buffer
            .as_ref()
            .is_none_or(|buffer| buffer.capacity < required)
        {
            if let Some(old) = self.ui_vertex_buffer.take() {
                self.device.destroy_buffer(old.handle);
            }
            let capacity = required.next_power_of_two();
            let handle = self
                .device
                .create_buffer(&BufferDescriptor {
                    size_bytes: capacity as u64,
                    usage_flags: render_core::BufferUsage::VERTEX,
                    memory_hint: MemoryHint::CpuToGpu,
                    debug_label: Some("dx12-ui-overlay".into()),
                })
                .map_err(|error| {
                    vec![Diagnostic::new(
                        "DX1277",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        format!("create DX12 UI vertex stream failed: {error:?}"),
                    )]
                })?;
            if let Err(error) = self.device.set_vertex_stride(handle, 32) {
                self.device.destroy_buffer(handle);
                return Err(vec![Diagnostic::new(
                    "DX1278",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("set DX12 UI vertex stride failed: {error:?}"),
                )]);
            }
            self.ui_vertex_buffer = Some(Dx12DynamicVertexBuffer {
                handle,
                bytes: Vec::new(),
                capacity,
            });
        }
        let vertex_buffer = self
            .ui_vertex_buffer
            .as_mut()
            .expect("UI vertex buffer was created above");
        self.device
            .write_buffer(vertex_buffer.handle, &prepared.vertex_bytes, 0)
            .map_err(|error| {
                vec![Diagnostic::new(
                    "DX1279",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("write DX12 UI vertex stream failed: {error:?}"),
                )]
            })?;
        vertex_buffer.bytes = prepared.vertex_bytes;
        let vertex_handle = vertex_buffer.handle;
        let layout = self.ui_pipeline_layout.ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1280",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "DX12 UI root signature is unavailable",
            )]
        })?;
        let pipeline = self.ui_pipeline.ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1280",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "DX12 UI pipeline is unavailable",
            )]
        })?;
        let fallback = self.fallback_ui_texture.ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1280",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "DX12 UI fallback texture is unavailable",
            )]
        })?;
        let mut frame = self.active_frame.take().ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1206",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "UI overlay pass called without an active DX12 frame",
            )]
        })?;
        frame.encoder.bind_pipeline(pipeline);
        frame
            .encoder
            .set_viewport(0.0, 0.0, self.width as f32, self.height as f32, 0.0, 1.0);
        let mut screen_size = [0_u8; 8];
        screen_size[..4].copy_from_slice(&(self.width as f32).to_ne_bytes());
        screen_size[4..].copy_from_slice(&(self.height as f32).to_ne_bytes());
        frame.encoder.push_constants(layout, 0x10, 0, &screen_size);
        frame.encoder.bind_vertex_buffers(&[vertex_handle], &[0]);
        for draw in prepared.draws {
            frame.encoder.set_scissor(
                draw.scissor.0,
                draw.scissor.1,
                draw.scissor.2,
                draw.scissor.3,
            );
            let texture = draw
                .texture_id
                .as_ref()
                .and_then(|id| self.textures.get(id))
                .map_or(fallback, |texture| texture.handle);
            if !frame.encoder.bind_sampled_texture(layout, texture) {
                self.active_frame = Some(frame);
                return Err(vec![Diagnostic::new(
                    "DX1281",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    "DX12 UI pass could not bind its texture",
                )]);
            }
            frame
                .encoder
                .draw(draw.vertex_count, 1, draw.first_vertex, 0);
            frame.draw_calls += 1;
            frame.triangles += u64::from(draw.vertex_count / 3);
        }
        self.active_frame = Some(frame);
        Ok(())
    }
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
impl BackendRenderer for Dx12SceneRenderer {
    fn supports_weighted_blended_oit(&self) -> bool {
        true
    }

    fn supports_gpu_particle_simulation(&self) -> bool {
        true
    }

    fn begin_frame(&mut self, input: &RenderFrameInput) -> Result<(), Vec<Diagnostic>> {
        validate_dx12_frame_contract(input)?;
        if let Some(reason) = &self.fatal_frame_error {
            return Err(vec![Diagnostic::new(
                "DX1243",
                DiagnosticSeverity::Fatal,
                "scene_renderer",
                format!("DX12 renderer is in a failed frame state and must be recreated: {reason}"),
            )]);
        }
        if self.active_frame.is_some() {
            return Err(vec![Diagnostic::new(
                "DX1200",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "begin_frame called while another DX12 frame is active",
            )]);
        }
        self.shadow_frame_data = None;

        self.ensure_pipeline();
        let pipeline = self.pipeline.ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1203",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "DX12 forward pipeline is unavailable; shader or PSO creation failed",
            )]
        })?;
        self.device
            .set_next_frame_clear_color(input.views[0].clear_color);
        let (image_index, mut encoder) =
            self.device.begin_frame(self.swapchain).map_err(|error| {
                vec![Diagnostic::new(
                    "DX1201",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("begin_frame failed: {error:?}"),
                )]
            })?;
        encoder.set_viewport(0.0, 0.0, self.width as f32, self.height as f32, 0.0, 1.0);
        encoder.set_scissor(0, 0, self.width, self.height);
        encoder.bind_pipeline(pipeline);
        let extraction_stats = input
            .extraction_stats
            .unwrap_or(engine_renderer::ExtractionStats {
                visible_drawables: input.drawables.len().try_into().unwrap_or(u32::MAX),
                culled_drawables: 0,
                visible_lights: input.lights.len().try_into().unwrap_or(u32::MAX),
                culled_lights: 0,
            });
        self.active_frame = Some(Dx12FrameState {
            image_index,
            encoder,
            draw_calls: 0,
            triangles: 0,
            visible_drawables: extraction_stats.visible_drawables,
            culled_drawables: extraction_stats.culled_drawables,
            visible_lights: extraction_stats.visible_lights,
            culled_lights: extraction_stats.culled_lights,
            hdr_pass_active: false,
        });
        Ok(())
    }

    fn apply_pass_barriers(
        &mut self,
        _input: &RenderFrameInput,
        _pass: &render_graph2::PassNode,
        barriers: &[engine_renderer::render_graph2::CompiledBarrier],
    ) -> Result<(), Vec<Diagnostic>> {
        if let Some(unsupported) = barriers.iter().find(|barrier| {
            !matches!(
                barrier.resource_name.as_str(),
                "swapchain"
                    | "hdr_color"
                    | "oit_accumulation"
                    | "oit_optical_depth"
                    | "depth"
                    | "depth_stencil"
            )
        }) {
            return Err(vec![Diagnostic::new(
                "DX1248",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!(
                    "DX12 cannot apply render-graph barrier for resource '{}'",
                    unsupported.resource_name
                ),
            )]);
        }
        // The accepted direct-output resources are transitioned by
        // Dx12Device::begin_frame/end_frame and render-pass attachment setup.
        Ok(())
    }

    fn execute_pass(
        &mut self,
        input: &RenderFrameInput,
        pass: &render_graph2::PassNode,
        _stats: &mut FrameStats,
    ) -> Result<(), Vec<Diagnostic>> {
        match &pass.kind {
            render_graph2::PassKind::OpaquePbrForward => {
                self.record_forward_pass(input, Some(pass.view_id))
            }
            render_graph2::PassKind::Present => self.record_ui_overlay_pass(input),
            render_graph2::PassKind::ToneMap => self.record_tone_map_pass(input),
            render_graph2::PassKind::DirectionalShadow => {
                self.record_directional_shadow_pass(input)
            }
            render_graph2::PassKind::Custom(name) => Err(vec![Diagnostic::new(
                "DX1246",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("custom render pass '{name}' is not registered by the DX12 backend"),
            )]),
        }
    }

    fn end_frame(&mut self, stats: &mut FrameStats) -> Result<(), Vec<Diagnostic>> {
        let mut frame = self.active_frame.take().ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1204",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "end_frame called without an active DX12 frame",
            )]
        })?;
        frame.encoder.end_render_pass();
        let device_stats =
            match self
                .device
                .end_frame(self.swapchain, frame.encoder, frame.image_index)
            {
                Ok(stats) => stats,
                Err(error) => {
                    let reason = format!(
                        "end_frame failed after command-list ownership transfer: {error:?}"
                    );
                    self.fatal_frame_error = Some(reason.clone());
                    return Err(vec![Diagnostic::new(
                        "DX1202",
                        DiagnosticSeverity::Fatal,
                        "scene_renderer",
                        reason,
                    )]);
                }
            };
        stats.draw_calls = frame.draw_calls;
        stats.triangles = frame.triangles;
        stats.visible_drawables = frame.visible_drawables;
        stats.visible_lights = frame.visible_lights;
        stats.culled_drawables = frame.culled_drawables;
        stats.culled_lights = frame.culled_lights;
        stats.gpu_frame_ms = device_stats.gpu_frame_ms;
        Ok(())
    }

    fn abort_frame(&mut self) -> Result<(), Vec<Diagnostic>> {
        let Some(mut frame) = self.active_frame.take() else {
            return Ok(());
        };
        frame.encoder.end_render_pass();
        match self.device.abort_frame(frame.encoder) {
            Ok(()) => Ok(()),
            Err(error) => {
                let reason = format!("failed to abandon the active DX12 command list: {error:?}");
                self.fatal_frame_error = Some(reason.clone());
                Err(vec![Diagnostic::new(
                    "DX1205",
                    DiagnosticSeverity::Fatal,
                    "scene_renderer",
                    reason,
                )])
            }
        }
    }

    fn upload_mesh(&mut self, upload: MeshUpload) -> Result<UploadReceipt, Vec<Diagnostic>> {
        if self.active_frame.is_some() {
            return Err(vec![Diagnostic::new(
                "DX1224",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "cannot upload a mesh while a DX12 frame is active",
            )]);
        }

        let mesh_id = upload.mesh_id.id.clone();
        if let Some(existing) = self.meshes.get(&mesh_id) {
            if existing.content_hash == upload.content_hash {
                return Ok(UploadReceipt::new(existing.revision));
            }
        }

        let vertex_buffer = self
            .device
            .create_buffer(&BufferDescriptor {
                size_bytes: upload.vertex_bytes.len() as u64,
                usage_flags: render_core::BufferUsage::VERTEX,
                memory_hint: MemoryHint::CpuToGpu,
                debug_label: Some(format!("mesh-{mesh_id}-vertices")),
            })
            .map_err(|error| {
                vec![Diagnostic::new(
                    "DX1220",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("upload_mesh create_buffer(vertices): {error:?}"),
                )]
            })?;
        if let Err(error) = self
            .device
            .write_buffer(vertex_buffer, &upload.vertex_bytes, 0)
        {
            self.device.destroy_buffer(vertex_buffer);
            return Err(vec![Diagnostic::new(
                "DX1221",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("upload_mesh write_buffer(vertices): {error:?}"),
            )]);
        }
        if let Err(error) = self
            .device
            .set_vertex_stride(vertex_buffer, upload.vertex_format.stride_bytes())
        {
            self.device.destroy_buffer(vertex_buffer);
            return Err(vec![Diagnostic::new(
                "DX1226",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("upload_mesh set vertex stride: {error:?}"),
            )]);
        }

        let index_buffer = match self.device.create_buffer(&BufferDescriptor {
            size_bytes: upload.index_bytes.len() as u64,
            usage_flags: render_core::BufferUsage::INDEX,
            memory_hint: MemoryHint::CpuToGpu,
            debug_label: Some(format!("mesh-{mesh_id}-indices")),
        }) {
            Ok(buffer) => buffer,
            Err(error) => {
                self.device.destroy_buffer(vertex_buffer);
                return Err(vec![Diagnostic::new(
                    "DX1222",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("upload_mesh create_buffer(indices): {error:?}"),
                )]);
            }
        };
        if let Err(error) = self
            .device
            .write_buffer(index_buffer, &upload.index_bytes, 0)
        {
            self.device.destroy_buffer(index_buffer);
            self.device.destroy_buffer(vertex_buffer);
            return Err(vec![Diagnostic::new(
                "DX1223",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("upload_mesh write_buffer(indices): {error:?}"),
            )]);
        }

        let revision = match self
            .mesh_revisions
            .get(&mesh_id)
            .copied()
            .unwrap_or(0)
            .checked_add(1)
        {
            Some(revision) => revision,
            None => {
                self.device.destroy_buffer(index_buffer);
                self.device.destroy_buffer(vertex_buffer);
                return Err(vec![Diagnostic::new(
                    "DX1225",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("mesh revision overflow for '{mesh_id}'"),
                )]);
            }
        };
        let index_format = match upload.index_format {
            RendererIndexFormat::U16 => RhiIndexFormat::U16,
            RendererIndexFormat::U32 => RhiIndexFormat::U32,
        };
        let mesh = Dx12GpuMesh {
            vertex_buffer,
            index_buffer,
            index_count: upload.index_count,
            index_format,
            vertex_format: upload.vertex_format,
            vertex_count: upload.vertex_count,
            vertex_bytes: upload.vertex_bytes,
            content_hash: upload.content_hash,
            revision,
        };

        // Keep the old resource live until every allocation and write for the
        // replacement has succeeded. Waiting only when replacing avoids
        // releasing buffers still referenced by an in-flight command list.
        if self.meshes.contains_key(&mesh_id) {
            self.device.wait_idle();
            self.clear_morphed_vertex_buffers();
            self.clear_particle_instance_buffers();
        }
        if let Some(old) = self.meshes.insert(mesh_id.clone(), mesh) {
            self.device.destroy_buffer(old.vertex_buffer);
            self.device.destroy_buffer(old.index_buffer);
        }
        self.mesh_revisions.insert(mesh_id, revision);
        Ok(UploadReceipt::new(revision))
    }

    fn upload_material(
        &mut self,
        upload: MaterialUpload,
    ) -> Result<UploadReceipt, Vec<Diagnostic>> {
        if self.active_frame.is_some() {
            return Err(vec![Diagnostic::new(
                "DX1233",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "cannot upload a material while a DX12 frame is active",
            )]);
        }
        for texture in upload.texture_references().into_iter().flatten() {
            if !self.textures.contains_key(&texture.id) {
                return Err(vec![Diagnostic::new(
                    "DX1234",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!(
                        "DX12 material '{}' references texture '{}' before a successful upload",
                        upload.material_id.id, texture.id
                    ),
                )]);
            }
        }

        let material_id = upload.material_id.id.clone();
        if let Some(existing) = self.materials.get(&material_id) {
            if existing.content_hash == upload.content_hash {
                return Ok(UploadReceipt::new(existing.revision));
            }
        }
        let revision = self
            .materials
            .get(&material_id)
            .map_or(1, |existing| existing.revision.saturating_add(1));
        let texture_ids = upload
            .texture_references()
            .map(|texture| texture.map(|texture| texture.id.clone()));
        let texture_flags = material_texture_flags_from_ids(&texture_ids);
        self.materials.insert(
            material_id,
            Dx12MaterialState {
                constants: material_constants_from_upload(&upload),
                emissive_constants: emissive_constants(upload.emissive, texture_flags),
                advanced_constants: advanced_constants_from_upload(&upload),
                texture_ids,
                transparency: upload.transparency,
                double_sided: upload.double_sided,
                content_hash: upload.content_hash,
                revision,
            },
        );
        Ok(UploadReceipt::new(revision))
    }

    fn upload_texture(&mut self, upload: TextureUpload) -> Result<UploadReceipt, Vec<Diagnostic>> {
        if self.active_frame.is_some() {
            return Err(vec![Diagnostic::new(
                "DX1235",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "cannot upload a texture while a DX12 frame is active",
            )]);
        }
        let texture_id = upload.texture_id.id.clone();
        if let Some(existing) = self.textures.get(&texture_id) {
            if existing.content_hash == upload.content_hash {
                return Ok(UploadReceipt::new(existing.revision));
            }
        }
        let revision = self
            .textures
            .get(&texture_id)
            .map_or(1, |existing| existing.revision.saturating_add(1));
        let handle = self
            .device
            .upload_sampled_rgba8(
                upload.width,
                upload.height,
                upload.color_space,
                &upload.mip_levels,
                upload.sampler,
            )
            .map_err(|error| {
                vec![Diagnostic::new(
                    "DX1236",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("DX12 texture upload '{texture_id}' failed: {error:?}"),
                )]
            })?;
        if let Some(old) = self.textures.insert(
            texture_id,
            Dx12TextureState {
                handle,
                content_hash: upload.content_hash,
                revision,
            },
        ) {
            self.device.destroy_texture(old.handle);
        }
        Ok(UploadReceipt::new(revision))
    }

    fn upload_environment_map(
        &mut self,
        upload: EnvironmentMapUpload,
    ) -> Result<UploadReceipt, Vec<Diagnostic>> {
        if self.active_frame.is_some() {
            return Err(vec![Diagnostic::new(
                "DX1261",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "cannot upload an environment map while a DX12 frame is active",
            )]);
        }
        let environment_id = upload.environment_id.id.clone();
        if let Some(existing) = self.environments.get(&environment_id) {
            if existing.content_hash == upload.content_hash {
                return Ok(UploadReceipt::new(existing.revision));
            }
        }
        let revision = self
            .environments
            .get(&environment_id)
            .map_or(1, |existing| existing.revision.saturating_add(1));
        let handle = self
            .device
            .upload_sampled_rgba16f_cube(&upload.mip_levels)
            .map_err(|error| {
                vec![Diagnostic::new(
                    "DX1262",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!(
                        "DX12 environment-map upload '{}' failed: {error:?}",
                        environment_id
                    ),
                )]
            })?;
        let replacement = Dx12EnvironmentState {
            handle,
            mip_count: upload.mip_levels.len() as u32,
            content_hash: upload.content_hash,
            revision,
        };
        if let Some(old) = self.environments.insert(environment_id, replacement) {
            self.device.destroy_texture(old.handle);
        }
        Ok(UploadReceipt::new(revision))
    }

    fn upload_morph_target_set(
        &mut self,
        upload: MorphTargetSetUpload,
    ) -> Result<UploadReceipt, Vec<Diagnostic>> {
        if self.active_frame.is_some() {
            return Err(vec![Diagnostic::new(
                "DX1268",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "cannot upload a morph target set while a DX12 frame is active",
            )]);
        }
        let target_set_id = upload.target_set_id.id.clone();
        if let Some(existing) = self.morph_target_sets.get(&target_set_id) {
            if existing.content_hash == upload.content_hash {
                return Ok(UploadReceipt::new(existing.revision));
            }
        }
        let revision = self
            .morph_target_sets
            .get(&target_set_id)
            .map_or(1, |existing| existing.revision.saturating_add(1));
        if self.morph_target_sets.contains_key(&target_set_id) {
            self.device.wait_idle();
            self.clear_morphed_vertex_buffers();
        }
        self.morph_target_sets.insert(
            target_set_id,
            Dx12MorphTargetSet {
                vertex_count: upload.vertex_count,
                targets: upload.targets,
                content_hash: upload.content_hash,
                revision,
            },
        );
        Ok(UploadReceipt::new(revision))
    }

    fn remove_resource(&mut self, removal: ResourceRemoval) -> Result<(), Vec<Diagnostic>> {
        if self.active_frame.is_some() {
            return Err(vec![Diagnostic::new(
                "DX1232",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "cannot remove a resource while a DX12 frame is active",
            )]);
        }
        match removal.kind {
            ResourceKind::Mesh => {
                if let Some(mesh) = self.meshes.remove(&removal.resource_id.id) {
                    self.device.wait_idle();
                    self.clear_morphed_vertex_buffers();
                    self.clear_particle_instance_buffers();
                    self.device.destroy_buffer(mesh.vertex_buffer);
                    self.device.destroy_buffer(mesh.index_buffer);
                }
            }
            ResourceKind::Material => {
                self.materials.remove(&removal.resource_id.id);
            }
            ResourceKind::Texture => {
                if let Some(dependent) = self.materials.iter().find_map(|(id, material)| {
                    material
                        .texture_ids
                        .iter()
                        .flatten()
                        .any(|texture_id| texture_id == &removal.resource_id.id)
                        .then_some(id)
                }) {
                    return Err(vec![Diagnostic::new(
                        "DX1231",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        format!(
                            "cannot remove DX12 texture '{}' while material '{}' references it",
                            removal.resource_id.id, dependent
                        ),
                    )]);
                }
                if let Some(texture) = self.textures.remove(&removal.resource_id.id) {
                    self.device.wait_idle();
                    self.device.destroy_texture(texture.handle);
                }
            }
            ResourceKind::EnvironmentMap => {
                if let Some(environment) = self.environments.remove(&removal.resource_id.id) {
                    self.device.wait_idle();
                    self.device.destroy_texture(environment.handle);
                }
            }
            ResourceKind::MorphTargetSet => {
                if self
                    .morph_target_sets
                    .remove(&removal.resource_id.id)
                    .is_some()
                {
                    self.device.wait_idle();
                    self.clear_morphed_vertex_buffers();
                }
            }
        }
        Ok(())
    }

    fn resize(&mut self, width: u32, height: u32) -> Result<(), Vec<Diagnostic>> {
        self.resize_surface(width, height)
    }
}

#[cfg(all(test, target_os = "windows", feature = "backend-dx12"))]
mod material_tests {
    use super::*;
    use crate::{DirectX12Backend, Dx12Device};
    use engine_renderer::{ClearFlags, LightItem, LightKind, Rect, RenderView, ViewCompose};
    use render_core::{Backend, DeviceDescriptor, ValidationMode};

    fn read_f32(bytes: &[u8], offset: usize) -> f32 {
        f32::from_ne_bytes(bytes[offset..offset + 4].try_into().unwrap())
    }

    #[test]
    fn material_constants_match_hlsl_root_constant_layout() {
        let constants = material_constants([0.1, 0.2, 0.3, 0.4], 0.5, 0.6, 0.7);
        let values: Vec<f32> = (0..8)
            .map(|index| read_f32(&constants, index * 4))
            .collect();
        assert_eq!(values, vec![0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.0]);
    }

    #[test]
    fn short_material_binding_preserves_defaults_for_missing_values() {
        let constants = material_constants_from_bytes(
            &0.25_f32.to_ne_bytes(),
            false,
            &Transparency::Opaque,
            false,
        );
        assert_eq!(read_f32(&constants, 0), 0.25);
        assert_eq!(read_f32(&constants, 16), 0.0);
        assert_eq!(read_f32(&constants, 20), 1.0);
        assert_eq!(read_f32(&constants, 24), 1.0);
        assert_eq!(
            emissive_constants_from_bytes(&0.25_f32.to_ne_bytes(), 0),
            default_emissive_constants()
        );
    }

    #[test]
    fn emissive_constants_match_hlsl_tail_layout() {
        let constants = emissive_constants([0.2, 0.4, 0.6], 30);
        assert_eq!(read_f32(&constants, 0), 0.2);
        assert_eq!(read_f32(&constants, 4), 0.4);
        assert_eq!(read_f32(&constants, 8), 0.6);
        assert_eq!(read_f32(&constants, 12), 30.0);

        let mut binding = vec![0_u8; 48];
        binding[32..48].copy_from_slice(&constants);
        assert_eq!(emissive_constants_from_bytes(&binding, 30), constants);
    }

    #[test]
    fn advanced_constants_preserve_quantized_parameters() {
        let parameters = engine_renderer::AdvancedMaterialParameters {
            clearcoat: 0.25,
            clearcoat_roughness: 0.5,
            subsurface: 0.75,
            anisotropy: -0.25,
            subsurface_color: [1.0, 0.5, 0.0],
            sheen_color: [0.0, 0.25, 1.0],
            rim_color: [0.1, 0.2, 0.3],
            rim_power: 6.0,
        };
        let constants = advanced_constants(parameters);
        let packed_weights = u32::from_ne_bytes(constants[0..4].try_into().unwrap());
        assert!(((packed_weights & 255) as f32 / 255.0 - 0.25).abs() < 0.003);
        assert!((((packed_weights >> 8) & 255) as f32 / 255.0 - 0.5).abs() < 0.003);
        assert!((((packed_weights >> 16) & 255) as f32 / 255.0 - 0.75).abs() < 0.003);
        let anisotropy = ((packed_weights >> 24) & 255) as f32 / 255.0 * 2.0 - 1.0;
        assert!((anisotropy + 0.25).abs() < 0.005);
        let packed_subsurface = u32::from_ne_bytes(constants[4..8].try_into().unwrap());
        assert_eq!(packed_subsurface & 255, 255);
        assert!((((packed_subsurface >> 8) & 255) as f32 / 255.0 - 0.5).abs() < 0.003);
    }

    #[test]
    fn portable_material_bytes_feed_dx12_advanced_constants() {
        let mut bytes = vec![0_u8; 112];
        for (offset, value) in [
            (48, 0.2_f32),
            (52, 0.3),
            (56, 0.4),
            (60, -0.5),
            (64, 1.0),
            (68, 0.5),
            (72, 0.0),
            (80, 0.1),
            (84, 0.2),
            (88, 0.3),
            (96, 0.4),
            (100, 0.5),
            (104, 0.6),
            (108, 4.0),
        ] {
            bytes[offset..offset + 4].copy_from_slice(&value.to_ne_bytes());
        }
        let constants = advanced_constants_from_bytes(&bytes);
        let packed_weights = u32::from_ne_bytes(constants[0..4].try_into().unwrap());
        assert!(((packed_weights & 255) as f32 / 255.0 - 0.2).abs() < 0.003);
        let anisotropy = ((packed_weights >> 24) & 255) as f32 / 255.0 * 2.0 - 1.0;
        assert!((anisotropy + 0.5).abs() < 0.005);
    }

    #[test]
    fn tone_map_constants_match_portable_post_process_contract() {
        let mut input = shadow_input([0.0, -1.0, 0.0]);
        input.render_options.exposure_ev100 = Some(2.0);
        input.render_options.post_process.bloom.enabled = true;
        input.render_options.post_process.color_grading.enabled = true;
        input.render_options.post_process.vignette.enabled = true;
        let constants = tone_map_constants(&input).expect("valid settings");
        assert_eq!(u32::from_ne_bytes(constants[0..4].try_into().unwrap()), 0);
        assert_eq!(read_f32(&constants, 4), 0.25);
        assert_eq!(u32::from_ne_bytes(constants[12..16].try_into().unwrap()), 7);
        assert_eq!(
            read_f32(&constants, 16),
            input.render_options.post_process.bloom.threshold
        );
        assert_eq!(
            read_f32(&constants, 112),
            input.render_options.post_process.vignette.intensity
        );
        input.render_options.transparency_mode =
            engine_renderer::TransparencyMode::WeightedBlendedOit;
        let oit_constants = tone_map_constants(&input).expect("valid OIT settings");
        assert_eq!(
            u32::from_ne_bytes(oit_constants[12..16].try_into().unwrap()),
            15
        );
    }

    #[test]
    fn material_texture_flags_follow_portable_slot_order() {
        let texture_ids = [
            None,
            Some("normal".to_string()),
            Some("metallic-roughness".to_string()),
            Some("occlusion".to_string()),
            Some("emissive".to_string()),
        ];
        assert_eq!(material_texture_flags_from_ids(&texture_ids), 30);
    }

    #[test]
    fn material_flags_encode_texture_mask_cutoff_and_surface_variant() {
        assert_eq!(material_surface_flags(false, &Transparency::Opaque), 0.0);
        assert_eq!(material_surface_flags(true, &Transparency::Blend), 1.0);
        assert_eq!(
            material_surface_flags(false, &Transparency::Masked { cutoff: 0.4 }),
            2.2
        );
        assert_eq!(
            material_surface_flags(true, &Transparency::Masked { cutoff: 0.4 }),
            3.2
        );
        assert_eq!(
            surface_variant_index(&Transparency::Opaque, false, false),
            0
        );
        assert_eq!(surface_variant_index(&Transparency::Opaque, true, false), 1);
        assert_eq!(surface_variant_index(&Transparency::Blend, false, false), 2);
        assert_eq!(surface_variant_index(&Transparency::Blend, true, false), 3);
        assert_eq!(
            surface_variant_index(&Transparency::Additive, false, false),
            4
        );
        assert_eq!(
            surface_variant_index(&Transparency::Additive, true, false),
            5
        );
        assert_eq!(surface_variant_index(&Transparency::Blend, false, true), 6);
        assert_eq!(surface_variant_index(&Transparency::Blend, true, true), 7);
        let weighted = material_constants_from_bytes(&[], true, &Transparency::Blend, true);
        assert_eq!(read_f32(&weighted, 28), 9.0);
    }

    fn shadow_input(direction: [f32; 3]) -> RenderFrameInput {
        let mut input = RenderFrameInput::empty(0);
        input.views.push(RenderView {
            view_id: 1,
            camera_entity: None,
            viewport: Rect::FULL,
            viewport_rect_normalized: Rect::FULL,
            view_matrix: Mat4::look_at_rh(
                glam::Vec3::new(0.0, 2.0, 5.0),
                glam::Vec3::ZERO,
                glam::Vec3::Y,
            )
            .to_cols_array(),
            projection_matrix: Mat4::perspective_rh(60_f32.to_radians(), 16.0 / 9.0, 0.1, 100.0)
                .to_cols_array(),
            clear_flags: ClearFlags::ColorAndDepth,
            clear_color: [0.0, 0.0, 0.0, 1.0],
            render_layer_mask: u32::MAX,
            msaa_samples: 1,
            compose: ViewCompose::Base {
                clear: ClearFlags::ColorAndDepth,
                clear_color: [0.0, 0.0, 0.0, 1.0],
            },
            stack_order: 0,
            frustum: None,
        });
        input.lights.push(LightItem {
            entity: None,
            kind: LightKind::Directional,
            color: [1.0; 3],
            intensity: 1.0,
            range: 0.0,
            position: [0.0; 3],
            direction,
            spot_angles: None,
            shadow_mode: ShadowMode::Soft,
        });
        input
    }

    #[test]
    fn directional_shadow_fit_uses_camera_and_light() {
        let data =
            Dx12SceneRenderer::directional_shadow_frame_data(&shadow_input([0.4, -1.0, 0.2]))
                .expect("valid shadow fit")
                .expect("shadow light");
        assert!(data
            .light_view_projection
            .to_cols_array()
            .iter()
            .all(|value| value.is_finite()));
        assert!(data.soft);
        assert!((data.light_direction_to_surface.length() - 1.0).abs() < 1.0e-5);
    }

    #[test]
    fn directional_shadow_fit_rejects_zero_light_direction() {
        let diagnostics =
            Dx12SceneRenderer::directional_shadow_frame_data(&shadow_input([0.0, 0.0, 0.0]))
                .expect_err("zero light direction must fail");
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "DX1248"));
    }

    #[test]
    fn dx12_frame_options_fail_closed_instead_of_being_ignored() {
        let mut input = shadow_input([0.0, -1.0, 0.0]);
        input.render_options.pass_graph_config.output_mode =
            engine_renderer::PassGraphOutputMode::HdrThenToneMap;
        assert!(validate_dx12_frame_contract(&input).is_ok());

        input.views[0].msaa_samples = 4;
        assert_eq!(
            validate_dx12_frame_contract(&input).unwrap_err()[0].code,
            "DX1249"
        );
        input.views[0].msaa_samples = 1;
        input.views[0].viewport.max = [0.5, 1.0];
        assert_eq!(
            validate_dx12_frame_contract(&input).unwrap_err()[0].code,
            "DX1250"
        );
        input.views[0].viewport = Rect::FULL;
        input.render_options.pass_graph_config.output_mode =
            engine_renderer::PassGraphOutputMode::DirectToSwapchain;
        assert_eq!(
            validate_dx12_frame_contract(&input).unwrap_err()[0].code,
            "DX1247"
        );
    }

    #[test]
    fn dx12_scene_shader_keeps_linear_hdr_output() {
        let source = include_str!("shaders.hlsl");
        assert!(source.contains("output.hdr = float4(color, sampled_base_color.a)"));
        assert!(source.contains("output.oit_accumulation"));
        assert!(source.contains("output.oit_optical_depth"));
        assert!(!source.contains("linear_to_srgb(color)"));
    }

    #[test]
    fn dx12_scene_creates_forward_and_shadow_pipelines() {
        let adapter = DirectX12Backend::new()
            .enumerate_adapters()
            .expect("adapter enumeration")
            .into_iter()
            .next()
            .expect("DX12 adapter");
        let descriptor = DeviceDescriptor {
            required_limits: adapter.capabilities.limits.clone(),
            adapter,
            required_features: Vec::new(),
            debug_label: Some("scene-pipeline-smoke".into()),
            validation_mode: ValidationMode::Standard,
        };
        let device = Dx12Device::create(&descriptor).expect("DX12 device");
        let mut renderer =
            Dx12SceneRenderer::new(device, SwapchainHandle::new(u32::MAX, u32::MAX), 1280, 720);
        renderer.ensure_pipeline();
        assert!(renderer.pipeline.is_some());
        assert!(renderer.skinned_pipeline.is_some());
        assert!(renderer.shadow_pipeline.is_some());
        assert!(renderer.skinned_shadow_pipeline.is_some());
        assert!(renderer.shadow_texture.is_some());
        assert!(renderer.shadow_framebuffer.is_some());
        assert!(renderer.hdr_texture.is_some());
        assert!(renderer.oit_accum_texture.is_some());
        assert!(renderer.oit_optical_depth_texture.is_some());
        assert!(renderer.hdr_framebuffer.is_some());
        assert!(renderer.oit_pipeline.is_some());
        assert!(renderer.gpu_particle_oit_pipeline.is_some());
        assert!(renderer.tone_map_pipeline.is_some());
        assert!(renderer.fallback_environment.is_some());
        assert!(renderer.ui_pipeline.is_some());
        assert!(renderer.fallback_ui_texture.is_some());
    }

    #[test]
    fn dx12_uploads_hdr_cubemap_and_tracks_revision() {
        let adapter = DirectX12Backend::new()
            .enumerate_adapters()
            .expect("adapter enumeration")
            .into_iter()
            .next()
            .expect("DX12 adapter");
        let descriptor = DeviceDescriptor {
            required_limits: adapter.capabilities.limits.clone(),
            adapter,
            required_features: Vec::new(),
            debug_label: Some("environment-upload-smoke".into()),
            validation_mode: ValidationMode::Standard,
        };
        let device = Dx12Device::create(&descriptor).expect("DX12 device");
        let mut renderer =
            Dx12SceneRenderer::new(device, SwapchainHandle::new(u32::MAX, u32::MAX), 64, 64);
        let one = 0x3c00_u16.to_le_bytes();
        let pixel = [
            one[0], one[1], one[0], one[1], one[0], one[1], one[0], one[1],
        ];
        let receipt = renderer
            .upload_environment_map(EnvironmentMapUpload {
                environment_id: engine_renderer::AssetId::new("sky"),
                format: engine_renderer::EnvironmentMapFormat::Rgba16Float,
                mip_levels: vec![engine_renderer::EnvironmentCubeMip {
                    face_size: 1,
                    faces: vec![pixel.to_vec(); 6],
                }],
                content_hash: [7; 32],
            })
            .expect("environment upload");
        assert_eq!(receipt.revision, 1);
        assert_eq!(renderer.environments["sky"].mip_count, 1);
    }

    #[test]
    fn dx12_applies_morphs_before_skinning_and_uploads_particle_instances() {
        let adapter = DirectX12Backend::new()
            .enumerate_adapters()
            .expect("adapter enumeration")
            .into_iter()
            .next()
            .expect("DX12 adapter");
        let descriptor = DeviceDescriptor {
            required_limits: adapter.capabilities.limits.clone(),
            adapter,
            required_features: Vec::new(),
            debug_label: Some("dynamic-character-vfx-smoke".into()),
            validation_mode: ValidationMode::Standard,
        };
        let device = Dx12Device::create(&descriptor).expect("DX12 device");
        let mut renderer =
            Dx12SceneRenderer::new(device, SwapchainHandle::new(u32::MAX, u32::MAX), 64, 64);
        let mut vertex_bytes = vec![0_u8; 64];
        for (offset, value) in [
            (0, 1.0_f32),
            (4, 0.0),
            (8, 0.0),
            (12, 0.0),
            (16, 1.0),
            (20, 0.0),
            (48, 1.0),
        ] {
            vertex_bytes[offset..offset + 4].copy_from_slice(&value.to_ne_bytes());
        }
        renderer
            .upload_mesh(MeshUpload {
                mesh_id: engine_renderer::AssetId::new("face"),
                vertex_format: RendererMeshVertexFormat::Skinned64,
                vertex_count: 1,
                vertex_bytes,
                index_format: RendererIndexFormat::U16,
                index_count: 3,
                index_bytes: vec![0; 6],
                bounds: engine_renderer::AxisAlignedBox::UNIT,
                content_hash: [2; 32],
            })
            .expect("mesh upload");
        let target_set_id = engine_renderer::AssetId::new("face.morphs");
        renderer
            .upload_morph_target_set(MorphTargetSetUpload {
                target_set_id: target_set_id.clone(),
                vertex_count: 1,
                targets: vec![engine_renderer::MorphTarget {
                    name: "smile".into(),
                    position_deltas: vec![[1.0, 0.0, 0.0]],
                    normal_deltas: vec![[0.0, 0.0, 1.0]],
                }],
                content_hash: [3; 32],
            })
            .expect("morph upload");
        let mesh = renderer.meshes["face"].clone();
        renderer
            .prepare_morphed_vertex_buffer("face:actor", &mesh, &target_set_id, &[0.5])
            .expect("morph deformation");
        let morphed = &renderer.morphed_vertex_buffers["face:actor"].bytes;
        assert!((read_f32(morphed, 0) - 1.5).abs() < 1.0e-6);
        assert!((read_f32(morphed, 16) - 0.8944272).abs() < 1.0e-5);
        assert!((read_f32(morphed, 20) - 0.4472136).abs() < 1.0e-5);

        let instances = [
            engine_renderer::ParticleInstance {
                position: [1.0, 2.0, 3.0],
                size: 0.5,
                rotation_radians: 0.25,
                normalized_age: 0.75,
                color: [255, 128, 64, 32],
            },
            engine_renderer::ParticleInstance {
                position: [4.0, 5.0, 6.0],
                size: 1.5,
                rotation_radians: 0.5,
                normalized_age: 0.25,
                color: [10, 20, 30, 40],
            },
        ];
        renderer
            .prepare_particle_instance_buffer("sparks", &instances)
            .expect("particle stream")
            .expect("non-empty stream");
        assert_eq!(renderer.particle_instance_buffers["sparks"].bytes.len(), 64);
        assert_eq!(
            read_f32(&renderer.particle_instance_buffers["sparks"].bytes, 12),
            0.5
        );
        assert_eq!(
            &renderer.particle_instance_buffers["sparks"].bytes[24..28],
            &[255, 128, 64, 32]
        );
    }

    #[test]
    fn dx12_ui_preparation_expands_indices_and_preserves_clip_order() {
        let vertices = vec![
            engine_renderer::UiVertex {
                position: [10.0, 20.0],
                uv: [0.0, 0.0],
                color: [255, 0, 0, 255],
            },
            engine_renderer::UiVertex {
                position: [30.0, 20.0],
                uv: [1.0, 0.0],
                color: [0, 255, 0, 255],
            },
            engine_renderer::UiVertex {
                position: [30.0, 40.0],
                uv: [1.0, 1.0],
                color: [0, 0, 255, 255],
            },
        ];
        let prepared = prepare_dx12_ui(
            &[engine_renderer::UiBatch {
                canvas_id: "hud".into(),
                z_order: 0,
                clip_rect: engine_renderer::Rect {
                    min: [5.2, 6.4],
                    max: [50.1, 60.8],
                },
                texture: None,
                vertices,
                indices: vec![0, 1, 2],
                material: engine_renderer::AssetId::new("ui"),
            }],
            100,
            80,
        )
        .expect("valid UI");
        assert_eq!(prepared.vertex_bytes.len(), 3 * 32);
        assert_eq!(prepared.draws.len(), 1);
        assert_eq!(prepared.draws[0].first_vertex, 0);
        assert_eq!(prepared.draws[0].vertex_count, 3);
        assert_eq!(prepared.draws[0].scissor, (5, 6, 46, 55));
        assert_eq!(read_f32(&prepared.vertex_bytes, 16), 1.0);
    }
}

#[cfg(all(test, not(all(target_os = "windows", feature = "backend-dx12"))))]
mod stub_tests {
    use super::*;
    use engine_renderer::{
        AssetId, AxisAlignedBox, IndexFormat, MeshVertexFormat, PBR32_VERTEX_STRIDE,
    };

    #[test]
    fn unavailable_backend_never_reports_render_upload_or_resize_success() {
        let mut renderer = Dx12SceneRenderer;
        assert!(renderer.render_frame(&RenderFrameInput::empty(0)).is_err());
        assert!(renderer
            .upload_mesh(MeshUpload {
                mesh_id: AssetId::new("mesh"),
                vertex_format: MeshVertexFormat::Pbr32,
                vertex_count: 1,
                vertex_bytes: vec![0; PBR32_VERTEX_STRIDE as usize],
                index_format: IndexFormat::U16,
                index_count: 3,
                index_bytes: vec![0; 6],
                bounds: AxisAlignedBox::UNIT,
                content_hash: [1; 32],
            })
            .is_err());
        assert!(renderer.resize(1280, 720).is_err());
        assert!(renderer.resize(0, 720).is_err());
    }
}

// ============================================================================
// Non-Windows / no backend-dx12: fail-closed placeholder
// ============================================================================

#[cfg(not(all(target_os = "windows", feature = "backend-dx12")))]
use engine_renderer::{
    BackendRenderer, Diagnostic, DiagnosticSeverity, FrameStats, MeshUpload, RenderFrameInput,
    UploadReceipt,
};

#[cfg(not(all(target_os = "windows", feature = "backend-dx12")))]
pub struct Dx12SceneRenderer;

#[cfg(not(all(target_os = "windows", feature = "backend-dx12")))]
impl Dx12SceneRenderer {
    pub fn new(
        _device: crate::device::Dx12Device,
        _swapchain: render_core::SwapchainHandle,
        _width: u32,
        _height: u32,
    ) -> Self {
        Self
    }

    fn unavailable(operation: &str) -> Vec<Diagnostic> {
        vec![Diagnostic::new(
            "DX1290",
            DiagnosticSeverity::Error,
            "scene_renderer",
            format!("cannot perform {operation}: the DX12 backend is unavailable on this target"),
        )]
    }
}

#[cfg(not(all(target_os = "windows", feature = "backend-dx12")))]
impl BackendRenderer for Dx12SceneRenderer {
    fn begin_frame(&mut self, _input: &RenderFrameInput) -> Result<(), Vec<Diagnostic>> {
        Err(Self::unavailable("begin_frame"))
    }

    fn abort_frame(&mut self) -> Result<(), Vec<Diagnostic>> {
        Err(Self::unavailable("abort_frame"))
    }

    fn upload_mesh(&mut self, _upload: MeshUpload) -> Result<UploadReceipt, Vec<Diagnostic>> {
        Err(Self::unavailable("mesh upload"))
    }

    fn resize(&mut self, width: u32, height: u32) -> Result<(), Vec<Diagnostic>> {
        if width == 0 || height == 0 {
            return Err(vec![Diagnostic::new(
                "DX1240",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("resize dimensions must be non-zero, got {width}x{height}"),
            )]);
        }
        Err(Self::unavailable("resize"))
    }
}
