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
//!   output on all drivers.  The frame lifecycle (acquire 鈫?record 鈫?submit
//!   鈫?present) is otherwise complete.
//! * Material handling currently covers pipeline variants plus basic raster and
//!   blend state, but textures and uniform bindings are not yet consumed.
//! * No texture loading, skeletal animation, or multi-pass optimisation.

use std::collections::BTreeMap;

use engine_renderer::{
    render_graph, AssetId, BackendRenderer, Diagnostic, DiagnosticSeverity, FrameStats,
    MaterialBinding, MaterialPipelineContext, MaterialResolver, ParamBlock, RenderFrameInput,
    RenderableItem, Transparency,
};
use render_core::{
    self, BufferDescriptor, BufferHandle, CommandEncoder, Device, IndexFormat, MemoryHint,
    PipelineHandle, PipelineLayoutDescriptor, PipelineLayoutHandle, PipelineVariantKey,
    PushConstantRange, RenderPassDescriptor, RenderPassHandle, SwapchainDescriptor,
    SwapchainHandle, TextureFormat, VertexAttribute, VertexLayout,
};

#[cfg(test)]
use render_core::PipelineDescriptor;

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
// Fallback mesh data  鈥? a coloured quad
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

const SCENE_FORWARD_PIPELINE_ID: &str = "scene-forward";

fn scene_forward_vertex_layout() -> VertexLayout {
    VertexLayout {
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
    }
}

fn scene_forward_pipeline_context(
    pll: PipelineLayoutHandle,
    rp: RenderPassHandle,
) -> MaterialPipelineContext {
    MaterialPipelineContext {
        shader_modules: vec![],
        vertex_layout: scene_forward_vertex_layout(),
        bind_layouts: vec![],
        pipeline_layout: pll,
        render_pass: rp,
        render_targets: vec![TextureFormat::Bgra8Unorm],
        depth_format: Some(TextureFormat::Depth32Float),
        depth_write_enabled: true,
        depth_compare: Some("less".into()),
        front_face: None,
        topology: Some("triangle_list".into()),
        polygon_mode: Some("fill".into()),
        sample_count: 1,
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

fn get_or_create_scene_forward_pipeline(
    material_resolver: &mut MaterialResolver,
    device: &mut dyn Device,
    material: &MaterialBinding,
    pll: PipelineLayoutHandle,
    rp: RenderPassHandle,
    variant_key: PipelineVariantKey,
) -> Result<PipelineHandle, render_core::RhiError> {
    let context = scene_forward_pipeline_context(pll, rp);
    let (pipeline_key, pipeline_desc) = material_resolver.resolve(material, &context, variant_key);
    material_resolver
        .library_mut()
        .get_or_create(device, pipeline_key, &pipeline_desc)
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

    rp: Option<RenderPassHandle>,
    pll: Option<PipelineLayoutHandle>,

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
            material_resolver: MaterialResolver::new(16),
            meshes: BTreeMap::new(),
            rp: None,
            pll: None,
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
    // Pipeline initialisation  (lazy 鈥?called on the first frame)
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
                size: 128, // 4脳4 f32 matrix (64 B) + spare uniform data
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

        self.rp = Some(rp);
        self.pll = Some(pll);
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
        )
        .map_err(|e| {
            vec![Diagnostic::new(
                "RV0204",
                DiagnosticSeverity::Error,
                "scene_renderer",
                &format!("resolve pipeline: {e:?}"),
            )]
        })
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

        // First encounter 鈥?upload a fallback coloured quad.
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
        self.device
            .write_buffer(vb, &vertex_bytes, 0)
            .map_err(|e| {
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

            let pipeline = self.pipeline_for_drawable(input, drawable)?;
            encoder.bind_pipeline(pipeline);

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

        let stats = self.device.end_frame(sc_h, encoder, ii).map_err(|e| {
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
        if self.cur_enc.is_none() {
            return Ok(());
        }

        match pass.kind {
            render_graph::PassKind::OpaquePbrForward => {
                // Begin render pass for this view.
                if let Some(rp) = self.rp {
                    // Dummy framebuffer 鈥?see TODO in render_frame.
                    let fb = render_core::FramebufferHandle::new(0, 0);
                    self.cur_enc.as_mut().expect("encoder checked above").begin_render_pass(
                        rp,
                        fb,
                        (0, 0, self.width, self.height),
                        [0.02, 0.02, 0.06, 1.0],
                        None,
                    );
                }

                self.cur_enc
                    .as_mut()
                    .expect("encoder checked above")
                    .set_viewport(0.0, 0.0, self.width as f32, self.height as f32, 0.0, 1.0);
                self.cur_enc
                    .as_mut()
                    .expect("encoder checked above")
                    .set_scissor(0, 0, self.width, self.height);

                if let Some(pll) = self.pll {
                    self.cur_enc
                        .as_mut()
                        .expect("encoder checked above")
                        .bind_descriptor_sets(pll, 0, &[], &[]);
                }

                for drawable in &input.drawables {
                    let mesh_id = &drawable.mesh.id;
                    // execute_pass assumes meshes were already cached by
                    // render_frame or by an earlier warm-up call.
                    let mesh = match self.meshes.get(mesh_id).cloned() {
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

                    if let Some(pll) = self.pll {
                        let world = &drawable.world_transform;
                        let mut pc_bytes = Vec::with_capacity(128);
                        for v in world {
                            pc_bytes.extend_from_slice(&v.to_ne_bytes());
                        }
                        pc_bytes.resize(128, 0u8);
                        self.cur_enc
                            .as_mut()
                            .expect("encoder checked above")
                            .push_constants(pll, 0x01, 0, &pc_bytes);
                    }

                    let pipeline = self.pipeline_for_drawable(input, drawable)?;
                    self.cur_enc
                        .as_mut()
                        .expect("encoder checked above")
                        .bind_pipeline(pipeline);

                    self.cur_enc
                        .as_mut()
                        .expect("encoder checked above")
                        .bind_vertex_buffers(&[mesh.vertex_buffer], &[0]);
                    self.cur_enc
                        .as_mut()
                        .expect("encoder checked above")
                        .bind_index_buffer(mesh.index_buffer, 0, mesh.index_format);
                    self.cur_enc
                        .as_mut()
                        .expect("encoder checked above")
                        .draw_indexed(mesh.index_count, 1, 0, 0, 0);

                    stats.draw_calls += 1;
                    stats.triangles += mesh.index_count as u64 / 3;
                }

                self.cur_enc
                    .as_mut()
                    .expect("encoder checked above")
                    .end_render_pass();
                stats.visible_drawables = input.drawables.len() as u32;
                stats.visible_lights = input.lights.len() as u32;
            }

            // Shadow and tone-map passes are no-ops in this first-pass
            // implementation.
            render_graph::PassKind::DirectionalShadow | render_graph::PassKind::ToneMap => {}

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
            &scene_forward_pipeline_context(pll, rp),
            PipelineVariantKey::NONE,
        );

        assert_eq!(key.shader_asset_id, SCENE_FORWARD_PIPELINE_ID);
        assert_eq!(key.vertex_layout_hash, hash_vertex_layout(&desc.vertex_layout));
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
        )
        .expect("first pipeline create should succeed");
        let second = get_or_create_scene_forward_pipeline(
            &mut resolver,
            &mut device,
            &material,
            pll,
            rp,
            PipelineVariantKey::NONE,
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
        )
        .expect("first pipeline create should succeed");
        let second = get_or_create_scene_forward_pipeline(
            &mut resolver,
            &mut device,
            &material,
            pll,
            rp,
            PipelineVariantKey::SKINNED,
        )
        .expect("second pipeline create should succeed");

        assert_ne!(first, second);
        assert_eq!(device.create_calls, 2);
        assert_eq!(device.destroyed, vec![first]);
        assert_eq!(resolver.library().len(), 1);
    }
}
