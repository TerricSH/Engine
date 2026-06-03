//! DirectX 12 implementation of [`BackendRenderer`].
//!
//! Dx12SceneRenderer provides the engine's high-level rendering path for
//! the DX12 backend. It owns a [`Dx12Device`] and processes [`RenderFrameInput`]
//! by uploading meshes on first encounter and issuing indexed draw calls.
//!
//! # Current limitations
//!
//! * **Pipeline/PSO creation requires DXIL shader bytecode** — the actual
//!   draw calls are recorded but will not produce visible output until a
//!   compiled vertex+pixel shader pair is supplied.
//! * **No descriptor set binding** — materials and UBO data are not yet
//!   connected to the D3D12 root signature.
//! * **No texture or shadow support** — only basic forward rendering.

// ============================================================================
// Windows + backend-dx12: full implementation
// ============================================================================

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
use std::collections::HashMap;

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
use engine_renderer::{
    BackendRenderer, Diagnostic, DiagnosticSeverity, FrameStats, RenderFrameInput,
};
#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
use glam::Mat4;
#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
use render_core::{
    BufferDescriptor, BufferHandle, Device, IndexFormat, MemoryHint, PipelineHandle,
    PipelineLayoutHandle, SwapchainHandle,
};

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
use crate::device::Dx12Device;

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
#[derive(Clone, Debug)]
pub struct Dx12GpuMesh {
    pub vertex_buffer: BufferHandle,
    pub index_buffer: BufferHandle,
    pub index_count: u32,
    pub index_format: IndexFormat,
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
pub struct Dx12SceneRenderer {
    device: Dx12Device,
    meshes: HashMap<String, Dx12GpuMesh>,
    width: u32,
    height: u32,
    swapchain: SwapchainHandle,
    pipeline_layout: Option<PipelineLayoutHandle>,
    pipeline: Option<PipelineHandle>,
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
impl Dx12SceneRenderer {
    pub fn new(device: Dx12Device, swapchain: SwapchainHandle, width: u32, height: u32) -> Self {
        Self {
            device,
            meshes: HashMap::new(),
            width: width.max(1),
            height: height.max(1),
            swapchain,
            pipeline_layout: None,
            pipeline: None,
        }
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        self.width = w.max(1);
        self.height = h.max(1);
        let _ = self.device.recreate_swapchain(self.swapchain, self.width, self.height);
    }

    pub fn wait_idle(&self) {
        self.device.wait_idle();
    }

    /// Create a minimal forward PSO (position + color → MVP → raster).
    /// Logs a warning on failure and leaves `self.pipeline` as `None`.
    fn ensure_pipeline(&mut self) {
        use render_core::{
            PipelineDescriptor, PipelineLayoutDescriptor, PushConstantRange,
            ShaderModuleDescriptor, ShaderFormat, VertexAttribute, VertexLayout,
        };

        if self.pipeline.is_some() {
            return;
        }

        // ── HLSL source for the minimal forward vertex shader ────────
        const VS_SOURCE: &[u8] = b"
            struct VSIn { float3 pos : POSITION; float4 col : COLOR; };
            struct VSOut { float4 pos : SV_POSITION; float4 col : COLOR; };
            cbuffer MVP : register(b0) { float4x4 mvp; };
            VSOut main(VSIn i) {
                VSOut o;
                o.pos = mul(float4(i.pos, 1.0), mvp);
                o.col = i.col;
                return o;
            }
        ";
        const PS_SOURCE: &[u8] = b"
            struct PSIn { float4 pos : SV_POSITION; float4 col : COLOR; };
            float4 main(PSIn i) : SV_TARGET { return i.col; }
        ";

        // ── Try to compile shaders via D3DCompile ────────────────────
        let vs_dxil = compile_hlsl(VS_SOURCE, "main", "vs_5_0", &self.device);
        let ps_dxil = compile_hlsl(PS_SOURCE, "main", "ps_5_0", &self.device);

        let (vs_bytes, ps_bytes) = match (vs_dxil, ps_dxil) {
            (Ok(vs), Ok(ps)) => (vs, ps),
            _ => {
                tracing::warn!(
                    target: "scene_renderer",
                    "HLSL shader compilation failed — draw calls will be no-ops"
                );
                return;
            }
        };

        // ── Pipeline layout (root signature with MVP root constant) ──
        let pll_desc = PipelineLayoutDescriptor {
            push_constant_ranges: vec![PushConstantRange {
                stage_flags: 0x10, // VERTEX
                offset: 0,
                size: 64,          // 4×4 matrix × 4 bytes
            }],
            bind_group_layouts: vec![],
            debug_label: Some("scene_renderer".into()),
        };
        let pll = match self.device.create_pipeline_layout(&pll_desc) {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!(target: "scene_renderer", "create_pipeline_layout: {e:?}");
                return;
            }
        };
        self.pipeline_layout = Some(pll);

        // ── Vertex shader module ─────────────────────────────────────
        // Populate the device's shader bytecode cache so create_shader_module
        // can find the DXIL.
        self.device.shader_cache.insert([0; 32], vs_bytes.clone());
        let vs_mod = match self.device.create_shader_module(&ShaderModuleDescriptor {
            format: ShaderFormat::Dxil,
            source_hash: [0; 32],
            entry_points: vec!["main".into()],
            debug_label: Some("scene_renderer_vs".into()),
        }) {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!(target: "scene_renderer", "create_shader_module(VS): {e:?}");
                return;
            }
        };

        // ── Pixel shader module ──────────────────────────────────────
        self.device.shader_cache.insert([1; 32], ps_bytes.clone());
        let ps_mod = match self.device.create_shader_module(&ShaderModuleDescriptor {
            format: ShaderFormat::Dxil,
            source_hash: [1; 32],
            entry_points: vec!["main".into()],
            debug_label: Some("scene_renderer_ps".into()),
        }) {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!(target: "scene_renderer", "create_shader_module(PS): {e:?}");
                return;
            }
        };

        // ── Vertex layout (position + color, 32-byte stride) ─────────
        let vertex_layout = VertexLayout {
            stride_bytes: 32,
            attributes: vec![
                VertexAttribute {
                    semantic: "POSITION".into(),
                    format: "float32x3".into(),
                    offset_bytes: 0,
                },
                VertexAttribute {
                    semantic: "COLOR".into(),
                    format: "float32x4".into(),
                    offset_bytes: 12,
                },
            ],
        };

        // ── Pipeline ─────────────────────────────────────────────────
        let pso_desc = PipelineDescriptor {
            shader_modules: vec![vs_mod, ps_mod],
            vertex_layout,
            pipeline_layout: self.pipeline_layout,
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

        let pso = match self.device.create_pipeline(&pso_desc) {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!(target: "scene_renderer", "create_pipeline: {e:?}");
                return;
            }
        };
        self.pipeline = Some(pso);
    }
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
fn compile_hlsl(
    _source: &[u8],
    entry: &str,
    _target: &str,
    _device: &Dx12Device,
) -> Result<Vec<u8>, Vec<Diagnostic>> {
    // Load pre-compiled DXIL from build script output (OUT_DIR).
    // See build.rs for shader compilation via dxc.exe.
    let out_dir = std::env::var("OUT_DIR").ok();
    let dxil_name = if entry == "VSMain" {
        "scene_vs.dxil"
    } else {
        "scene_ps.dxil"
    };

    if let Some(dir) = out_dir {
        let path = std::path::Path::new(&dir).join(dxil_name);
        if let Ok(bytes) = std::fs::read(&path) {
            if !bytes.is_empty() {
                return Ok(bytes);
            }
        }
    }

    Err(vec![Diagnostic::new(
        "DX1270",
        DiagnosticSeverity::Warning,
        "scene_renderer",
        format!(
            "DXIL shader '{dxil_name}' not found — run build.rs with dxc.exe, or install the Windows SDK. \
             Draw calls will be recorded but produce no visible output until a PSO is available."
        ),
    )])
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
impl BackendRenderer for Dx12SceneRenderer {
    fn render_frame(&mut self, input: &RenderFrameInput) -> Result<FrameStats, Vec<Diagnostic>> {
        // Ensure a pipeline is available (creates shaders + PSO on first call).
        self.ensure_pipeline();

        let (_image_index, mut encoder) = match self.device.begin_frame(self.swapchain) {
            Ok(r) => r,
            Err(e) => {
                return Err(vec![Diagnostic::new(
                    "DX1201",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("begin_frame failed: {e:?}"),
                )]);
            }
        };

        encoder.set_viewport(0.0, 0.0, self.width as f32, self.height as f32, 0.0, 1.0);
        encoder.set_scissor(0, 0, self.width, self.height);

        // Bind the pipeline if one was created successfully.
        if let Some(pso) = self.pipeline {
            encoder.bind_pipeline(pso);
        }

        let mut draw_calls: u32 = 0;
        let mut triangles: u64 = 0;

        for drawable in &input.drawables {
            let mesh_id = &drawable.mesh.id;
            let mesh = match self.meshes.get(mesh_id) {
                Some(m) => m,
                None => {
                    tracing::warn!(
                        target: "scene_renderer",
                        mesh = mesh_id,
                        "no uploaded mesh for drawable — call upload_mesh() first"
                    );
                    continue;
                }
            };

            // Push MVP matrix via root constants on the first camera's view.
            if let (Some(pll), Some(view)) = (self.pipeline_layout, input.views.first()) {
                let view_m = Mat4::from_cols_array(&view.view_matrix);
                let proj_m = Mat4::from_cols_array(&view.projection_matrix);
                let world_m = Mat4::from_cols_array(&drawable.world_transform);
                let mvp = proj_m * view_m * world_m;
                let mvp_bytes: &[u8] = unsafe {
                    std::slice::from_raw_parts(&mvp as *const _ as *const u8, 64)
                };
                encoder.push_constants(pll, 0x10, 0, mvp_bytes);
            }

            encoder.bind_vertex_buffers(&[mesh.vertex_buffer], &[0]);
            encoder.bind_index_buffer(mesh.index_buffer, 0, mesh.index_format);
            encoder.draw_indexed(mesh.index_count, 1, 0, 0, 0);

            draw_calls += 1;
            triangles += mesh.index_count as u64 / 3;
        }

        encoder.end_render_pass();

        let stats = match self.device.end_frame(self.swapchain, encoder, _image_index) {
            Ok(s) => s,
            Err(e) => {
                return Err(vec![Diagnostic::new(
                    "DX1202",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("end_frame failed: {e:?}"),
                )]);
            }
        };

        Ok(FrameStats {
            draw_calls,
            triangles,
            visible_drawables: input.drawables.len() as u32,
            culled_drawables: 0,
            visible_lights: input.lights.len() as u32,
            culled_lights: 0,
            gpu_frame_ms: stats.gpu_frame_ms,
        })
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

        let vb_desc = BufferDescriptor {
            size_bytes: vertex_bytes.len() as u64,
            usage_flags: render_core::BufferUsage::VERTEX,
            memory_hint: MemoryHint::CpuToGpu,
            debug_label: Some(format!("mesh-{mesh_id}-vertices")),
        };
        let vb = self.device.create_buffer(&vb_desc).map_err(|e| {
            vec![Diagnostic::new(
                "DX1220",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("upload_mesh create_buffer(vertices): {e:?}"),
            )]
        })?;
        self.device.write_buffer(vb, vertex_bytes, 0).map_err(|e| {
            vec![Diagnostic::new(
                "DX1221",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("upload_mesh write_buffer(vertices): {e:?}"),
            )]
        })?;

        let ib_desc = BufferDescriptor {
            size_bytes: index_bytes.len() as u64,
            usage_flags: render_core::BufferUsage::INDEX,
            memory_hint: MemoryHint::CpuToGpu,
            debug_label: Some(format!("mesh-{mesh_id}-indices")),
        };
        let ib = self.device.create_buffer(&ib_desc).map_err(|e| {
            vec![Diagnostic::new(
                "DX1222",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("upload_mesh create_buffer(indices): {e:?}"),
            )]
        })?;
        self.device.write_buffer(ib, index_bytes, 0).map_err(|e| {
            vec![Diagnostic::new(
                "DX1223",
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

        let mesh = Dx12GpuMesh {
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
        self.device
            .recreate_swapchain(self.swapchain, self.width, self.height)
            .map_err(|e| {
                vec![Diagnostic::new(
                    "DX1241",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("resize/recreate_swapchain failed: {e:?}"),
                )]
            })?;
        Ok(())
    }
}

// ============================================================================
// Non-Windows / no backend-dx12: stub (compile-time placeholder)
// ============================================================================

#[cfg(not(all(target_os = "windows", feature = "backend-dx12")))]
pub struct Dx12SceneRenderer;

#[cfg(not(all(target_os = "windows", feature = "backend-dx12")))]
impl Dx12SceneRenderer {
    pub fn new(_device: crate::device::Dx12Device, _width: u32, _height: u32) -> Self {
        Self
    }
}

#[cfg(not(all(target_os = "windows", feature = "backend-dx12")))]
impl BackendRenderer for Dx12SceneRenderer {
    fn render_frame(
        &mut self,
        _input: &RenderFrameInput,
    ) -> Result<FrameStats, Vec<Diagnostic>> {
        Ok(FrameStats::default())
    }

    fn upload_mesh(
        &mut self,
        _mesh_id: &str,
        _vertex_bytes: &[u8],
        _index_bytes: &[u8],
        _index_count: u32,
        _index_format_u16: bool,
    ) -> Result<(), Vec<Diagnostic>> {
        Ok(())
    }
}

// Import FrameStats and Diagnostic for the stub
#[cfg(not(all(target_os = "windows", feature = "backend-dx12")))]
use engine_renderer::{Diagnostic, FrameStats, RenderFrameInput};
#[cfg(not(all(target_os = "windows", feature = "backend-dx12")))]
use render_core::BackendRenderer;
