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
    render_graph2, BackendRenderer, Diagnostic, DiagnosticSeverity, FrameStats,
    IndexFormat as RendererIndexFormat, MaterialUpload, MeshUpload,
    MeshVertexFormat as RendererMeshVertexFormat, RenderFrameInput, ResourceKind, ResourceRemoval,
    ShadowMode, TextureUpload, Transparency, UploadReceipt,
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
#[derive(Clone, Debug)]
pub struct Dx12GpuMesh {
    pub vertex_buffer: BufferHandle,
    pub index_buffer: BufferHandle,
    pub index_count: u32,
    pub index_format: RhiIndexFormat,
    pub vertex_format: RendererMeshVertexFormat,
    pub content_hash: [u8; 32],
    pub revision: u64,
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
#[derive(Clone, Debug)]
struct Dx12MaterialState {
    constants: [u8; 32],
    emissive_constants: [u8; 16],
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
struct Dx12BoneBuffer {
    handle: BufferHandle,
    bytes: Vec<u8>,
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
#[derive(Clone, Copy, Debug)]
struct Dx12ShadowFrameData {
    light_view_projection: Mat4,
    light_direction_to_surface: glam::Vec3,
    soft: bool,
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
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
fn validate_dx12_frame_contract(input: &RenderFrameInput) -> Result<(), Vec<Diagnostic>> {
    if input.render_options.pass_graph_config.output_mode
        != engine_renderer::PassGraphOutputMode::DirectToSwapchain
        || input.render_options.tone_mapping != engine_renderer::ToneMapping::None
    {
        return Err(vec![Diagnostic::new(
            "DX1247",
            DiagnosticSeverity::Error,
            "scene_renderer",
            "DX12 currently supports only DirectToSwapchain with ToneMapping::None; HDR composition is not implemented",
        )]);
    }
    if input.views.len() != 1 {
        return Err(vec![Diagnostic::new(
            "DX1244",
            DiagnosticSeverity::Error,
            "scene_renderer",
            "the DX12 backend currently supports exactly one render view per frame",
        )]);
    }
    if input.render_options.msaa_samples != 1 || input.views[0].msaa_samples != 1 {
        return Err(vec![Diagnostic::new(
            "DX1249",
            DiagnosticSeverity::Error,
            "scene_renderer",
            "DX12 SceneRenderer does not yet implement multisample resolve; use 1x MSAA",
        )]);
    }
    if input.views[0].viewport != engine_renderer::Rect::FULL
        || input.views[0].viewport_rect_normalized != engine_renderer::Rect::FULL
    {
        return Err(vec![Diagnostic::new(
            "DX1250",
            DiagnosticSeverity::Error,
            "scene_renderer",
            "DX12 SceneRenderer currently supports only a full-surface viewport",
        )]);
    }
    if input.views[0].clear_flags != engine_renderer::ClearFlags::ColorAndDepth {
        return Err(vec![Diagnostic::new(
            "DX1251",
            DiagnosticSeverity::Error,
            "scene_renderer",
            "DX12 SceneRenderer currently supports only ColorAndDepth clear mode",
        )]);
    }
    if input.render_options.exposure_ev100.is_some() {
        return Err(vec![Diagnostic::new(
            "DX1252",
            DiagnosticSeverity::Error,
            "scene_renderer",
            "DX12 direct-output path cannot apply an exposure override",
        )]);
    }
    if !input.ui_batches.is_empty() {
        return Err(vec![Diagnostic::new(
            "DX1253",
            DiagnosticSeverity::Error,
            "scene_renderer",
            "DX12 SceneRenderer does not yet implement UI batch rendering",
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
    bone_buffers: HashMap<String, Dx12BoneBuffer>,
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
    skinned_pipeline: Option<PipelineHandle>,
    skinned_double_sided_pipeline: Option<PipelineHandle>,
    skinned_blend_pipeline: Option<PipelineHandle>,
    skinned_blend_double_sided_pipeline: Option<PipelineHandle>,
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
fn surface_variant_index(transparency: &Transparency, double_sided: bool) -> usize {
    usize::from(double_sided)
        + if matches!(transparency, Transparency::Blend) {
            2
        } else {
            0
        }
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
fn create_surface_pipeline_variants(
    device: &mut Dx12Device,
    base: &PipelineDescriptor,
    label: &str,
) -> Result<[PipelineHandle; 4], render_core::RhiError> {
    let mut pipelines = Vec::with_capacity(4);
    for (double_sided, blended) in [(false, false), (true, false), (false, true), (true, true)] {
        let mut descriptor = base.clone();
        descriptor.raster_state.cull_mode = Some(if double_sided { "none" } else { "back" }.into());
        descriptor.depth_state.write_enabled = !blended;
        descriptor.blend_state.mode = blended.then(|| "alpha".into());
        descriptor.debug_label = Some(format!(
            "{label}-{}-{}",
            if double_sided {
                "double-sided"
            } else {
                "single-sided"
            },
            if blended { "blend" } else { "opaque-mask" }
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
impl Dx12SceneRenderer {
    pub fn new(device: Dx12Device, swapchain: SwapchainHandle, width: u32, height: u32) -> Self {
        Self {
            device,
            meshes: HashMap::new(),
            materials: HashMap::new(),
            textures: HashMap::new(),
            bone_buffers: HashMap::new(),
            mesh_revisions: HashMap::new(),
            width: width.max(1),
            height: height.max(1),
            swapchain,
            pipeline_layout: None,
            pipeline: None,
            double_sided_pipeline: None,
            blend_pipeline: None,
            blend_double_sided_pipeline: None,
            skinned_pipeline: None,
            skinned_double_sided_pipeline: None,
            skinned_blend_pipeline: None,
            skinned_blend_double_sided_pipeline: None,
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
        Ok(())
    }

    /// Create the minimal static PBR32 forward PSO used by this backend.
    fn ensure_pipeline(&mut self) {
        use render_core::{
            BindGroupLayoutBinding, BindGroupLayoutDescriptor, PipelineDescriptor,
            PipelineLayoutDescriptor, PushConstantRange, ShaderFormat, ShaderModuleDescriptor,
            ShaderStage, TextureDescriptor, TextureUsage, VertexAttribute, VertexLayout,
        };

        if self.pipeline.is_some()
            && self.double_sided_pipeline.is_some()
            && self.blend_pipeline.is_some()
            && self.blend_double_sided_pipeline.is_some()
            && self.skinned_pipeline.is_some()
            && self.skinned_double_sided_pipeline.is_some()
            && self.skinned_blend_pipeline.is_some()
            && self.skinned_blend_double_sided_pipeline.is_some()
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
                    size: 208,
                }],
                bind_group_layouts: vec![BindGroupLayoutDescriptor {
                    set_index: 0,
                    bindings: vec![
                        BindGroupLayoutBinding {
                            binding: 0,
                            resource_kind: "sampled_texture_set".into(),
                        },
                        BindGroupLayoutBinding {
                            binding: 0,
                            resource_kind: "sampler_set".into(),
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
            render_targets: vec![render_core::TextureFormat::Bgra8Unorm],
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
        self.skinned_pipeline = Some(skinned_variants[0]);
        self.skinned_double_sided_pipeline = Some(skinned_variants[1]);
        self.skinned_blend_pipeline = Some(skinned_variants[2]);
        self.skinned_blend_double_sided_pipeline = Some(skinned_variants[3]);

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
    ) -> [TextureHandle; 6] {
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
        ]
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
                Transparency::Blend
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
                Transparency::Blend
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
        if view.is_none() && (!input.drawables.is_empty() || !input.skinned_items.is_empty()) {
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
        ];
        let shadow_texture = self.shadow_texture.ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1245",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "DX12 shadow texture is unavailable",
            )]
        })?;
        let shadow_frame_data = self.shadow_frame_data;
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
        frame.encoder.bind_pipeline(current_pipeline);
        frame
            .encoder
            .set_viewport(0.0, 0.0, self.width as f32, self.height as f32, 0.0, 1.0);
        frame.encoder.set_scissor(0, 0, self.width, self.height);

        if let Some(view) = view {
            let view_matrix = Mat4::from_cols_array(&view.view_matrix);
            let projection_matrix = Mat4::from_cols_array(&view.projection_matrix);
            let camera_position = view_matrix.inverse().w_axis.truncate();
            let mut ordered_drawables = Vec::with_capacity(input.drawables.len());
            let mut blended_drawables = Vec::new();
            for drawable in &input.drawables {
                let (transparency, _) = self.material_surface(input, &drawable.material);
                if matches!(transparency, Transparency::Blend) {
                    let translation = Mat4::from_cols_array(&drawable.world_transform)
                        .w_axis
                        .truncate();
                    blended_drawables
                        .push(((translation - camera_position).length_squared(), drawable));
                } else {
                    ordered_drawables.push(drawable);
                }
            }
            blended_drawables.sort_by(|left, right| right.0.total_cmp(&left.0));
            ordered_drawables.extend(blended_drawables.into_iter().map(|(_, drawable)| drawable));

            for drawable in ordered_drawables {
                // Existence was validated before recording any draw command.
                let mesh = &self.meshes[&drawable.mesh.id];
                let world_matrix = Mat4::from_cols_array(&drawable.world_transform);
                let (transparency, double_sided) = self.material_surface(input, &drawable.material);
                let next_pipeline =
                    static_pipelines[surface_variant_index(&transparency, double_sided)];
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
                let material_constants = input_material
                    .map(|binding| {
                        material_constants_from_bytes(
                            &binding.uniforms.bytes,
                            texture_ids[0].is_some(),
                            &binding.transparency,
                        )
                    })
                    .or_else(|| {
                        self.materials
                            .get(&drawable.material.id)
                            .map(|material| material.constants)
                    })
                    .unwrap_or_else(default_material_constants);
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
                let texture_table = self.material_texture_table(&texture_ids, shadow_texture);
                if !frame
                    .encoder
                    .bind_sampled_texture_set(layout, &texture_table)
                {
                    self.active_frame = Some(frame);
                    return Err(vec![Diagnostic::new(
                        "DX1216",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        "DX12 forward pass could not bind its six-texture material table",
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
                if matches!(transparency, Transparency::Blend) {
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
            blended_skinned.sort_by(|left, right| right.0.total_cmp(&left.0));
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
                    [surface_variant_index(&transparency, double_sided)];
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
                let material_constants = input_material
                    .map(|binding| {
                        material_constants_from_bytes(
                            &binding.uniforms.bytes,
                            texture_ids[0].is_some(),
                            &binding.transparency,
                        )
                    })
                    .or_else(|| {
                        self.materials
                            .get(&item.material.id)
                            .map(|material| material.constants)
                    })
                    .unwrap_or_else(default_material_constants);
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
                let texture_table = self.material_texture_table(&texture_ids, shadow_texture);
                if !frame
                    .encoder
                    .bind_sampled_texture_set(layout, &texture_table)
                {
                    self.active_frame = Some(frame);
                    return Err(vec![Diagnostic::new(
                        "DX1216",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        "DX12 skinned pass could not bind its six-texture material table",
                    )]);
                }

                let cache_key = format!(
                    "{}:{}:{}",
                    item.skeleton.id,
                    item.entity.as_deref().unwrap_or("anonymous"),
                    item_index
                );
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
                        "DX1230",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        "DX12 skinned pipeline could not bind its bone palette",
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
        }

        self.active_frame = Some(frame);
        Ok(())
    }
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
impl BackendRenderer for Dx12SceneRenderer {
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
                "swapchain" | "depth" | "depth_stencil"
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
            render_graph2::PassKind::Present => Ok(()),
            render_graph2::PassKind::ToneMap => Err(vec![Diagnostic::new(
                "DX1247",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "ToneMap reached the direct-to-swapchain DX12 path",
            )]),
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
            content_hash: upload.content_hash,
            revision,
        };

        // Keep the old resource live until every allocation and write for the
        // replacement has succeeded. Waiting only when replacing avoids
        // releasing buffers still referenced by an in-flight command list.
        if self.meshes.contains_key(&mesh_id) {
            self.device.wait_idle();
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
        }
        Ok(())
    }

    fn resize(&mut self, width: u32, height: u32) -> Result<(), Vec<Diagnostic>> {
        self.resize_surface(width, height)
    }
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
fn default_material_constants() -> [u8; 32] {
    material_constants([0.8, 0.6, 0.4, 1.0], 0.0, 1.0, 1.0)
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
fn default_emissive_constants() -> [u8; 16] {
    [0; 16]
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
fn matrix_bytes(matrix: Mat4) -> [u8; 64] {
    let mut bytes = [0_u8; 64];
    for (destination, value) in bytes
        .chunks_exact_mut(4)
        .zip(matrix.to_cols_array().into_iter())
    {
        destination.copy_from_slice(&value.to_ne_bytes());
    }
    bytes
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
fn float4_bytes(values: [f32; 4]) -> [u8; 16] {
    let mut bytes = [0_u8; 16];
    for (destination, value) in bytes.chunks_exact_mut(4).zip(values) {
        destination.copy_from_slice(&value.to_ne_bytes());
    }
    bytes
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
fn shadow_scene_constants(
    shadow: Option<Dx12ShadowFrameData>,
    world: Mat4,
) -> ([u8; 64], [u8; 16], [u8; 16]) {
    let fallback_direction = glam::Vec3::new(0.5, 0.8, 0.3).normalize();
    let (light_matrix, parameters, direction) = match shadow {
        Some(shadow) => (
            shadow.light_view_projection * world,
            [
                1.0,
                if shadow.soft { 1.0 } else { 0.0 },
                1.0 / 2048.0,
                0.0015,
            ],
            shadow.light_direction_to_surface,
        ),
        None => (
            Mat4::IDENTITY,
            [0.0, 0.0, 1.0 / 2048.0, 0.0015],
            fallback_direction,
        ),
    };
    (
        matrix_bytes(light_matrix),
        float4_bytes(parameters),
        float4_bytes([direction.x, direction.y, direction.z, 0.0]),
    )
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
fn material_constants_from_upload(upload: &MaterialUpload) -> [u8; 32] {
    let mut constants = material_constants(
        upload.base_color,
        upload.metallic,
        upload.roughness,
        upload.ambient_occlusion,
    );
    constants[28..32].copy_from_slice(
        &material_surface_flags(upload.base_color_texture.is_some(), &upload.transparency)
            .to_ne_bytes(),
    );
    constants
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
fn material_constants_from_bytes(
    bytes: &[u8],
    uses_texture: bool,
    transparency: &Transparency,
) -> [u8; 32] {
    let fallback = default_material_constants();
    let mut constants = fallback;
    let copy_len = bytes.len().min(constants.len());
    constants[..copy_len].copy_from_slice(&bytes[..copy_len]);
    constants[28..32]
        .copy_from_slice(&material_surface_flags(uses_texture, transparency).to_ne_bytes());
    constants
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
fn emissive_constants_from_bytes(bytes: &[u8], texture_flags: u32) -> [u8; 16] {
    let mut constants = default_emissive_constants();
    if bytes.len() > 32 {
        let copy_len = (bytes.len() - 32).min(12);
        constants[..copy_len].copy_from_slice(&bytes[32..32 + copy_len]);
    }
    constants[12..16].copy_from_slice(&(texture_flags as f32).to_ne_bytes());
    constants
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
fn emissive_constants(emissive: [f32; 3], texture_flags: u32) -> [u8; 16] {
    float4_bytes([emissive[0], emissive[1], emissive[2], texture_flags as f32])
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
fn material_texture_flags_from_ids(texture_ids: &[Option<String>; 5]) -> u32 {
    texture_ids
        .iter()
        .enumerate()
        .fold(0_u32, |flags, (index, texture)| {
            flags | u32::from(texture.is_some()) << index
        })
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
fn material_surface_flags(uses_texture: bool, transparency: &Transparency) -> f32 {
    match transparency {
        Transparency::Masked { cutoff } => (if uses_texture { 3.0 } else { 2.0 }) + cutoff * 0.5,
        Transparency::Opaque | Transparency::Blend => {
            if uses_texture {
                1.0
            } else {
                0.0
            }
        }
    }
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
fn material_constants(
    base_color: [f32; 4],
    metallic: f32,
    roughness: f32,
    ambient_occlusion: f32,
) -> [u8; 32] {
    let mut bytes = [0_u8; 32];
    for (destination, value) in bytes.chunks_exact_mut(4).zip(base_color.into_iter().chain([
        metallic,
        roughness,
        ambient_occlusion,
        0.0,
    ])) {
        destination.copy_from_slice(&value.to_ne_bytes());
    }
    bytes
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
        let constants =
            material_constants_from_bytes(&0.25_f32.to_ne_bytes(), false, &Transparency::Opaque);
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
        assert_eq!(surface_variant_index(&Transparency::Opaque, false), 0);
        assert_eq!(surface_variant_index(&Transparency::Opaque, true), 1);
        assert_eq!(surface_variant_index(&Transparency::Blend, false), 2);
        assert_eq!(surface_variant_index(&Transparency::Blend, true), 3);
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
        input.render_options.tone_mapping = engine_renderer::ToneMapping::None;
        input.render_options.pass_graph_config.output_mode =
            engine_renderer::PassGraphOutputMode::DirectToSwapchain;
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
        input.render_options.exposure_ev100 = Some(0.0);
        assert_eq!(
            validate_dx12_frame_contract(&input).unwrap_err()[0].code,
            "DX1252"
        );
    }

    #[test]
    fn dx12_direct_shader_encodes_linear_output_for_unorm_swapchain() {
        let source = include_str!("shaders.hlsl");
        assert!(source.contains("linear_to_srgb(color)"));
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
