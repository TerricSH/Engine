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
//!   output on all drivers.  The frame lifecycle (acquire → record → submit
//!   → present) is otherwise complete.
//! * No material system – a flat white material is assumed.
//! * No texture loading, skeletal animation, or multi-pass optimisation.

use std::collections::{BTreeMap, HashMap, HashSet};

use engine_renderer::{
    render_graph, BackendRenderer, Diagnostic, DiagnosticSeverity, FrameStats, MaterialBinding,
    PipelineCache, PipelineCacheKey, RenderFrameInput,
};
use render_core::{
    self, BufferDescriptor, BufferHandle, CommandEncoder, Device, IndexFormat, MemoryHint,
    PipelineDescriptor, PipelineHandle, PipelineLayoutDescriptor, PipelineLayoutHandle,
    PipelineVariantKey, PushConstantRange, RenderPassDescriptor, RenderPassHandle,
    SwapchainDescriptor, SwapchainHandle, TextureFormat, VertexAttribute, VertexLayout,
};

use crate::device_impl::VulkanDevice;
use crate::shaders_embedded::{FORWARD_FRAG_SPV, FORWARD_VERT_SPV};

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
// Fallback mesh data  –  a coloured quad
// ============================================================================

/// Single vertex for the fallback mesh.
#[repr(C)]
struct FallbackVertex {
    position: [f32; 3],
    color: [f32; 4],
}

const FALLBACK_VERTICES: [FallbackVertex; 4] = [
    FallbackVertex {
        position: [-0.5, -0.5, 0.0],
        color: [1.0, 0.0, 0.0, 1.0],
    },
    FallbackVertex {
        position: [0.5, -0.5, 0.0],
        color: [0.0, 1.0, 0.0, 1.0],
    },
    FallbackVertex {
        position: [0.5, 0.5, 0.0],
        color: [0.0, 0.0, 1.0, 1.0],
    },
    FallbackVertex {
        position: [-0.5, 0.5, 0.0],
        color: [1.0, 1.0, 0.0, 1.0],
    },
];

const FALLBACK_INDICES: [u16; 6] = [0, 1, 2, 2, 3, 0];

fn fallback_vertex_bytes() -> Vec<u8> {
    let mut bytes = Vec::with_capacity(FALLBACK_VERTICES.len() * 28);
    for v in &FALLBACK_VERTICES {
        for f in v.position.iter().copied().chain(v.color.iter().copied()) {
            bytes.extend_from_slice(&f.to_ne_bytes());
        }
    }
    bytes
}

fn fallback_index_bytes() -> Vec<u8> {
    let mut bytes = Vec::with_capacity(FALLBACK_INDICES.len() * 2);
    for i in &FALLBACK_INDICES {
        bytes.extend_from_slice(&i.to_ne_bytes());
    }
    bytes
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

    // Cached pipeline handles (created via the Device trait).
    rp: Option<RenderPassHandle>,
    pll: Option<PipelineLayoutHandle>,

    /// Material-to-pipeline cache (variant-aware, supports hot-reload).
    pipeline_cache: PipelineCache,

    // Frame lifecycle state (stored between begin_frame / execute_pass / end_frame).
    cur_sc: Option<SwapchainHandle>,
    cur_ii: Option<u32>,
    cur_enc: Option<Box<dyn CommandEncoder>>,

    /// Window dimensions (logical pixels).
    width: u32,
    height: u32,
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
            rp: None,
            pll: None,
            pipeline_cache: PipelineCache::new(),
            cur_sc: None,
            cur_ii: None,
            cur_enc: None,
            width: width.max(1),
            height: height.max(1),
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
    // Pipeline initialisation  (lazy – called on the first frame)
    // ------------------------------------------------------------------

    /// Create the render pass, pipeline layout, and graphics pipeline.
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

        // --- Render pass  (colour + depth) ---
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
                &format!("create_render_pass: {e:?}"),
            )]
        })?;

        // --- Pipeline layout  (push constants for MVP) ---
        let pll_desc = PipelineLayoutDescriptor {
            bind_group_layouts: vec![],
            push_constant_ranges: vec![PushConstantRange {
                // VK_SHADER_STAGE_VERTEX_BIT = 0x01
                stage_flags: 0x01,
                offset: 0,
                size: 128, // 4×4 f32 matrix (64 B) + spare uniform data
            }],
            debug_label: Some("scene-pll".into()),
        };
        let pll = self.device.create_pipeline_layout(&pll_desc).map_err(|e| {
            vec![Diagnostic::new(
                "RV0201",
                DiagnosticSeverity::Error,
                "scene_renderer",
                &format!("create_pipeline_layout: {e:?}"),
            )]
        })?;

        // --- Graphics pipeline descriptor  (forward-shaded) ---
        let pl_desc = PipelineDescriptor {
            shader_modules: vec![],
            vertex_layout: VertexLayout {
                stride_bytes: 28, // float32x3 + float32x4
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
            },
            bind_layouts: vec![],
            pipeline_layout: Some(pll),
            raster_state: render_core::RasterState {
                cull_mode: Some("none".into()),
                front_face: None,
            },
            depth_state: render_core::DepthState {
                format: Some(TextureFormat::Depth32Float),
                write_enabled: true,
                compare: Some("less".into()),
            },
            blend_state: render_core::BlendState { mode: None },
            render_targets: vec![TextureFormat::Bgra8Unorm],
            debug_label: Some("scene-pl".into()),
            topology: Some("triangle_list".into()),
            polygon_mode: Some("fill".into()),
            sample_count: Some(1),
            render_pass: None,
        };

        // Register the forward-shaded pipeline descriptor in the variant cache.
        self.pipeline_cache
            .register_descriptor("forward_pbr", pl_desc);

        self.rp = Some(rp);
        self.pll = Some(pll);
        self.initialized = true;
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

        // First encounter – upload a fallback coloured quad.
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
                &format!("create_buffer(vertices): {e:?}"),
            )]
        })?;
        self.device.write_buffer(vb, &vertex_bytes, 0).map_err(|e| {
            vec![Diagnostic::new(
                "RV0204",
                DiagnosticSeverity::Error,
                "scene_renderer",
                &format!("write_buffer(vertices): {e:?}"),
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
                &format!("create_buffer(indices): {e:?}"),
            )]
        })?;
        self.device.write_buffer(ib, &index_bytes, 0).map_err(|e| {
            vec![Diagnostic::new(
                "RV0206",
                DiagnosticSeverity::Error,
                "scene_renderer",
                &format!("write_buffer(indices): {e:?}"),
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
    fn begin_frame_impl(
        &mut self,
    ) -> Result<(SwapchainHandle, u32, Box<dyn CommandEncoder>), Vec<Diagnostic>> {
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
                &format!("create_swapchain: {e:?}"),
            )]
        })?;

        let (ii, encoder) = self.device.begin_frame(sc_h).map_err(|e| {
            vec![Diagnostic::new(
                "RV0208",
                DiagnosticSeverity::Error,
                "scene_renderer",
                &format!("begin_frame: {e:?}"),
            )]
        })?;

        Ok((sc_h, ii, encoder))
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
        let (sc_h, ii, mut encoder) = self.begin_frame_impl()?;

        // Begin a render pass covering the full viewport.
        if let Some(rp) = self.rp {
            // TODO(Gate 3): wire up a real framebuffer backed by the
            // swapchain image-views.  The current handle is a dummy that
            // will cause `cmd_begin_render_pass` to be skipped by the
            // encoder (null image-view guard in `VkCmdEncoder`).
            let fb = render_core::FramebufferHandle::new(0, 0);
            encoder.begin_render_pass(
                rp,
                fb,
                (0, 0, self.width, self.height),
                [0.02, 0.02, 0.06, 1.0],
                None,
            );
        }

        encoder.set_viewport(0.0, 0.0, self.width as f32, self.height as f32, 0.0, 1.0);
        encoder.set_scissor(0, 0, self.width, self.height);

        // Bind descriptor sets once (shared by all pipelines with this layout).
        if let Some(pll) = self.pll {
            encoder.bind_descriptor_sets(pll, 0, &[], &[]);
        }

        // Build material lookup: material_id → MaterialBinding
        let material_map: HashMap<&str, &MaterialBinding> =
            input.materials.iter().map(|m| (m.material_id.id.as_str(), m)).collect();

        // Build set of skinned material IDs for variant resolution.
        let skinned_materials: HashSet<&str> =
            input.skinned_items.iter().map(|s| s.material.id.as_str()).collect();

        let device = &mut self.device;
        let cache = &mut self.pipeline_cache;

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

            // ── Resolve pipeline via variant cache ──────────────────────
            let is_skinned = skinned_materials.contains(drawable.material.id.as_str());
            let base_variant = if is_skinned {
                PipelineVariantKey::SKINNED
            } else {
                PipelineVariantKey::NONE
            };

            let pipeline_result = match material_map.get(drawable.material.id.as_str()) {
                Some(mat) => {
                    let key = PipelineCacheKey {
                        pipeline_asset: mat.pipeline.id.clone(),
                        variant_key: PipelineVariantKey::new(mat.variant_key | base_variant.bits()),
                    };
                    cache.get_or_create(&key, device)
                }
                None => {
                    // No material binding – fall back to the default forward PBR descriptor.
                    let key = PipelineCacheKey {
                        pipeline_asset: "forward_pbr".into(),
                        variant_key: base_variant,
                    };
                    cache.get_or_create(&key, device)
                }
            };

            let pl = match pipeline_result {
                Ok(h) => h,
                Err(e) => {
                    tracing::warn!(
                        target: "scene_renderer",
                        error = ?e,
                        "pipeline resolution failed for drawable, skipping"
                    );
                    continue;
                }
            };
            encoder.bind_pipeline(pl);

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

        encoder.end_render_pass();

        let stats = device.end_frame(sc_h, encoder, ii).map_err(|e| {
            vec![Diagnostic::new(
                "RV0209",
                DiagnosticSeverity::Error,
                "scene_renderer",
                &format!("end_frame: {e:?}"),
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

    fn begin_frame(&mut self, _input: &RenderFrameInput) -> Result<(), Vec<Diagnostic>> {
        let (sc_h, ii, enc) = self.begin_frame_impl()?;
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
        let Some(ref mut encoder) = self.cur_enc else {
            return Ok(());
        };

        match pass.kind {
            render_graph::PassKind::OpaquePbrForward => {
                // Begin render pass for this view.
                if let Some(rp) = self.rp {
                    // Dummy framebuffer – see TODO in render_frame.
                    let fb = render_core::FramebufferHandle::new(0, 0);
                    encoder.begin_render_pass(
                        rp,
                        fb,
                        (0, 0, self.width, self.height),
                        [0.02, 0.02, 0.06, 1.0],
                        None,
                    );
                }

                encoder.set_viewport(
                    0.0,
                    0.0,
                    self.width as f32,
                    self.height as f32,
                    0.0,
                    1.0,
                );
                encoder.set_scissor(0, 0, self.width, self.height);

                // Bind descriptor sets once (shared by all pipelines).
                if let Some(pll) = self.pll {
                    encoder.bind_descriptor_sets(pll, 0, &[], &[]);
                }

                // Build material and skinned-item lookups (same as render_frame).
                let material_map: HashMap<&str, &MaterialBinding> =
                    input.materials.iter().map(|m| (m.material_id.id.as_str(), m)).collect();
                let skinned_materials: HashSet<&str> =
                    input.skinned_items.iter().map(|s| s.material.id.as_str()).collect();

                let device = &mut self.device;
                let cache = &mut self.pipeline_cache;

                for drawable in &input.drawables {
                    let mesh_id = &drawable.mesh.id;
                    // execute_pass assumes meshes were already cached by
                    // render_frame or by an earlier warm-up call.
                    let mesh = match self.meshes.get(mesh_id) {
                        Some(m) => m,
                        None => {
                            tracing::trace!(
                                target: "scene_renderer",
                                mesh = mesh_id,
                                "skipping un-cached mesh in execute_pass"
                            );
                            continue;
                        }
                    };

                    // ── Resolve pipeline via variant cache ──────────────
                    let is_skinned = skinned_materials.contains(drawable.material.id.as_str());
                    let base_variant = if is_skinned {
                        PipelineVariantKey::SKINNED
                    } else {
                        PipelineVariantKey::NONE
                    };

                    let pipeline_result = match material_map.get(drawable.material.id.as_str()) {
                        Some(mat) => {
                            let key = PipelineCacheKey {
                                pipeline_asset: mat.pipeline.id.clone(),
                                variant_key: PipelineVariantKey::new(
                                    mat.variant_key | base_variant.bits(),
                                ),
                            };
                            cache.get_or_create(&key, device)
                        }
                        None => {
                            let key = PipelineCacheKey {
                                pipeline_asset: "forward_pbr".into(),
                                variant_key: base_variant,
                            };
                            cache.get_or_create(&key, device)
                        }
                    };

                    let pl = match pipeline_result {
                        Ok(h) => h,
                        Err(e) => {
                            tracing::warn!(
                                target: "scene_renderer",
                                error = ?e,
                                "pipeline resolution failed in execute_pass, skipping"
                            );
                            continue;
                        }
                    };
                    encoder.bind_pipeline(pl);

                    if let Some(pll) = self.pll {
                        let world = &drawable.world_transform;
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

                    stats.draw_calls += 1;
                    stats.triangles += mesh.index_count as u64 / 3;
                }

                encoder.end_render_pass();
                stats.visible_drawables = input.drawables.len() as u32;
                stats.visible_lights = input.lights.len() as u32;
            }

            // Shadow and tone-map passes are no-ops in this first-pass
            // implementation.
            render_graph::PassKind::DirectionalShadow
            | render_graph::PassKind::ToneMap => {}

            // Present is handled entirely by end_frame.
            render_graph::PassKind::Present => {}
        }

        Ok(())
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
                    &format!("end_frame: {e:?}"),
                )]
            })?;
            stats.draw_calls = s.draw_calls;
            stats.triangles = s.triangles;
            stats.gpu_frame_ms = s.gpu_frame_ms;
        }
        Ok(())
    }
}
