//! DirectX 12 implementation of [`engine_renderer::BackendRenderer`].
//!
//! The portable upload path currently supports owned static PBR32 meshes.
//! Texture and material uploads deliberately use the trait's structured
//! unsupported diagnostics until descriptor heaps and texture staging are
//! implemented by this backend.

// ============================================================================
// Windows + backend-dx12: full implementation
// ============================================================================

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
use std::collections::HashMap;

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
use engine_renderer::{
    render_graph, BackendRenderer, Diagnostic, DiagnosticSeverity, FrameStats,
    IndexFormat as RendererIndexFormat, MeshUpload, RenderFrameInput, ResourceKind,
    ResourceRemoval, UploadReceipt,
};
#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
use glam::Mat4;
#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
use render_core::{
    BufferDescriptor, BufferHandle, CommandEncoder, Device, IndexFormat as RhiIndexFormat,
    MemoryHint, PipelineHandle, PipelineLayoutHandle, SwapchainHandle,
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
    pub content_hash: [u8; 32],
    pub revision: u64,
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
pub struct Dx12SceneRenderer {
    device: Dx12Device,
    meshes: HashMap<String, Dx12GpuMesh>,
    // Revisions survive removal so recreating the same logical resource never
    // moves its receipt backwards.
    mesh_revisions: HashMap<String, u64>,
    width: u32,
    height: u32,
    swapchain: SwapchainHandle,
    pipeline_layout: Option<PipelineLayoutHandle>,
    pipeline: Option<PipelineHandle>,
    active_frame: Option<Dx12FrameState>,
    /// Any failure after handing a command list to `end_frame` makes allocator
    /// reuse ambiguous. Refuse subsequent frames until the backend is rebuilt.
    fatal_frame_error: Option<String>,
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
impl Dx12SceneRenderer {
    pub fn new(device: Dx12Device, swapchain: SwapchainHandle, width: u32, height: u32) -> Self {
        Self {
            device,
            meshes: HashMap::new(),
            mesh_revisions: HashMap::new(),
            width: width.max(1),
            height: height.max(1),
            swapchain,
            pipeline_layout: None,
            pipeline: None,
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
            PipelineDescriptor, PipelineLayoutDescriptor, PushConstantRange, ShaderFormat,
            ShaderModuleDescriptor, VertexAttribute, VertexLayout,
        };

        if self.pipeline.is_some() {
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
                    stage_flags: 1,
                    offset: 0,
                    size: 64,
                }],
                bind_group_layouts: vec![],
                debug_label: Some("scene_renderer".into()),
            }) {
            Ok(handle) => handle,
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "create_pipeline_layout failed");
                return;
            }
        };
        self.pipeline_layout = Some(layout);

        self.device.shader_cache.insert([0; 32], vs_bytes.to_vec());
        let vertex_shader = match self.device.create_shader_module(&ShaderModuleDescriptor {
            format: ShaderFormat::Dxil,
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

        self.device.shader_cache.insert([1; 32], ps_bytes.to_vec());
        let pixel_shader = match self.device.create_shader_module(&ShaderModuleDescriptor {
            format: ShaderFormat::Dxil,
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
        let descriptor = PipelineDescriptor {
            shader_modules: vec![vertex_shader, pixel_shader],
            vertex_layout,
            pipeline_layout: Some(layout),
            topology: Some("triangle_list".into()),
            render_targets: vec![render_core::TextureFormat::Bgra8Unorm],
            depth_state: render_core::DepthState {
                format: None,
                write_enabled: false,
                compare: Some("always".into()),
            },
            raster_state: render_core::RasterState {
                cull_mode: Some("back".into()),
                front_face: Some("ccw".into()),
            },
            ..PipelineDescriptor::default()
        };
        match self.device.create_pipeline(&descriptor) {
            Ok(handle) => self.pipeline = Some(handle),
            Err(error) => {
                tracing::error!(target: "scene_renderer", ?error, "DX12 PSO creation failed");
            }
        }
    }

    fn record_forward_pass(
        &mut self,
        input: &RenderFrameInput,
        view_id: Option<u32>,
    ) -> Result<(), Vec<Diagnostic>> {
        if !input.skinned_items.is_empty() {
            return Err(vec![Diagnostic::new(
                "DX1214",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "DX12 skinned rendering is not implemented",
            )]);
        }

        let view = view_id
            .and_then(|id| input.views.iter().find(|view| view.view_id == id))
            .or_else(|| input.views.first());
        if view.is_none() && !input.drawables.is_empty() {
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
            .filter_map(|drawable| {
                (!self.meshes.contains_key(&drawable.mesh.id)).then_some(drawable.mesh.id.as_str())
            })
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
        let layout = self.pipeline_layout.ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1213",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "DX12 pipeline layout is unavailable",
            )]
        })?;
        let mut frame = self.active_frame.take().ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1206",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "execute_pass called without an active DX12 frame",
            )]
        })?;

        if let Some(view) = view {
            let view_matrix = Mat4::from_cols_array(&view.view_matrix);
            let projection_matrix = Mat4::from_cols_array(&view.projection_matrix);
            for drawable in &input.drawables {
                // Existence was validated before recording any draw command.
                let mesh = &self.meshes[&drawable.mesh.id];
                let world_matrix = Mat4::from_cols_array(&drawable.world_transform);
                let mvp = (projection_matrix * view_matrix * world_matrix).to_cols_array();
                let mut mvp_bytes = [0_u8; 64];
                for (destination, value) in mvp_bytes.chunks_exact_mut(4).zip(mvp) {
                    destination.copy_from_slice(&value.to_ne_bytes());
                }
                frame.encoder.push_constants(layout, 0x10, 0, &mvp_bytes);
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
    fn render_frame(&mut self, input: &RenderFrameInput) -> Result<FrameStats, Vec<Diagnostic>> {
        self.begin_frame(input)?;
        if let Err(mut diagnostics) = self.record_forward_pass(input, None) {
            if let Err(mut abort_diagnostics) = self.abort_frame() {
                diagnostics.append(&mut abort_diagnostics);
            }
            return Err(diagnostics);
        }

        let mut stats = FrameStats::default();
        self.end_frame(&mut stats)?;
        Ok(stats)
    }

    fn begin_frame(&mut self, input: &RenderFrameInput) -> Result<(), Vec<Diagnostic>> {
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
        if input.views.len() > 1 {
            return Err(vec![Diagnostic::new(
                "DX1244",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "the DX12 backend currently supports exactly one render view per frame",
            )]);
        }

        self.ensure_pipeline();
        let pipeline = self.pipeline.ok_or_else(|| {
            vec![Diagnostic::new(
                "DX1203",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "DX12 forward pipeline is unavailable; shader or PSO creation failed",
            )]
        })?;
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

    fn execute_pass(
        &mut self,
        input: &RenderFrameInput,
        pass: &render_graph::PassNode,
        _stats: &mut FrameStats,
    ) -> Result<(), Vec<Diagnostic>> {
        match &pass.kind {
            render_graph::PassKind::OpaquePbrForward => {
                self.record_forward_pass(input, Some(pass.view_id))
            }
            // The basic DX12 path writes LDR color directly to the swapchain.
            // The remaining canonical graph nodes do not add commands yet.
            render_graph::PassKind::ToneMap | render_graph::PassKind::Present => Ok(()),
            render_graph::PassKind::DirectionalShadow => Err(vec![Diagnostic::new(
                "DX1245",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "directional shadow rendering is not implemented by the DX12 backend",
            )]),
            render_graph::PassKind::Custom(name) => Err(vec![Diagnostic::new(
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

    fn remove_resource(&mut self, removal: ResourceRemoval) -> Result<(), Vec<Diagnostic>> {
        if self.active_frame.is_some() {
            return Err(vec![Diagnostic::new(
                "DX1232",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "cannot remove a resource while a DX12 frame is active",
            )]);
        }
        if removal.kind != ResourceKind::Mesh {
            return Err(vec![Diagnostic::new(
                "DX1231",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!(
                    "DX12 resource removal is not implemented for {:?}",
                    removal.kind
                ),
            )]);
        }

        if let Some(mesh) = self.meshes.remove(&removal.resource_id.id) {
            self.device.wait_idle();
            self.device.destroy_buffer(mesh.vertex_buffer);
            self.device.destroy_buffer(mesh.index_buffer);
        }
        Ok(())
    }

    fn resize(&mut self, width: u32, height: u32) -> Result<(), Vec<Diagnostic>> {
        self.resize_surface(width, height)
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
    fn render_frame(&mut self, _input: &RenderFrameInput) -> Result<FrameStats, Vec<Diagnostic>> {
        Err(Self::unavailable("render_frame"))
    }

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
