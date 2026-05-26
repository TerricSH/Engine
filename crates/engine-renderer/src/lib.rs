#![forbid(unsafe_code)]

pub mod render_graph;

pub use engine_serialize::{
    AssetId, ContractVersion, Diagnostic, DiagnosticSeverity, HashDigest, PersistentId,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const RENDERER_INPUT_CONTRACT: &str = "RendererInput-v0.2.0";
pub const IDENTITY_MAT4: Mat4 = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

pub type Vec2 = [f32; 2];
pub type Vec3 = [f32; 3];
pub type Vec4 = [f32; 4];
pub type Quat = [f32; 4];
pub type Mat4 = [f32; 16];
pub type LinearRgb = [f32; 4];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RenderFrameInput {
    pub contract_version: ContractVersion,
    pub frame_index: u64,
    pub views: Vec<RenderView>,
    pub drawables: Vec<RenderableItem>,
    pub skinned_items: Vec<SkinnedItem>,
    pub materials: Vec<MaterialBinding>,
    pub meshes: Vec<MeshBinding>,
    pub lights: Vec<LightItem>,
    pub debug_primitives: Vec<DebugPrimitive>,
    pub ui_batches: Vec<UiBatch>,
    pub render_options: RenderOptions,
    pub stats_scope: Option<String>,
}

impl RenderFrameInput {
    pub fn empty(frame_index: u64) -> Self {
        Self {
            contract_version: RENDERER_INPUT_CONTRACT.to_string(),
            frame_index,
            views: Vec::new(),
            drawables: Vec::new(),
            skinned_items: Vec::new(),
            materials: Vec::new(),
            meshes: Vec::new(),
            lights: Vec::new(),
            debug_primitives: Vec::new(),
            ui_batches: Vec::new(),
            render_options: RenderOptions::default(),
            stats_scope: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RenderView {
    pub view_id: u32,
    pub camera_entity: Option<PersistentId>,
    pub viewport: Rect,
    pub viewport_rect_normalized: Rect,
    pub view_matrix: Mat4,
    pub projection_matrix: Mat4,
    pub clear_flags: ClearFlags,
    pub clear_color: LinearRgb,
    pub render_layer_mask: u32,
    pub msaa_samples: u8,
    pub compose: ViewCompose,
    pub stack_order: i32,
    pub frustum: Option<[Vec4; 6]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub min: Vec2,
    pub max: Vec2,
}

impl Rect {
    pub const FULL: Self = Self {
        min: [0.0, 0.0],
        max: [1.0, 1.0],
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClearFlags {
    ColorAndDepth,
    DepthOnly,
    Nothing,
    Skybox,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlendMode {
    Replace,
    AlphaBlend,
    Additive,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ViewCompose {
    Base {
        clear: ClearFlags,
        clear_color: LinearRgb,
    },
    Overlay {
        base_view_id: u32,
        blend_mode: BlendMode,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RenderableItem {
    pub entity: Option<PersistentId>,
    pub mesh: AssetId,
    pub material: AssetId,
    pub world_transform: Mat4,
    pub bounds: AxisAlignedBox,
    pub render_layer: String,
    pub cast_shadows: bool,
    pub sort_key: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SkinnedItem {
    pub entity: Option<PersistentId>,
    pub mesh: AssetId,
    pub material: AssetId,
    pub skeleton: AssetId,
    pub bone_palette: Vec<Mat4>,
    pub bone_palette_layout: BonePaletteLayout,
    pub world_transform: Mat4,
    pub bounds: AxisAlignedBox,
    pub render_layer: String,
    pub cast_shadows: bool,
    pub sort_key: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum BonePaletteLayout {
    Full4x4 { count: u32 },
    Packed3x4 { count: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AxisAlignedBox {
    pub min: Vec3,
    pub max: Vec3,
}

impl AxisAlignedBox {
    pub const UNIT: Self = Self {
        min: [-0.5, -0.5, -0.5],
        max: [0.5, 0.5, 0.5],
    };
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LightItem {
    pub entity: Option<PersistentId>,
    pub kind: LightKind,
    pub color: [f32; 3],
    pub intensity: f32,
    pub range: f32,
    pub position: Vec3,
    pub direction: Vec3,
    pub spot_angles: Option<SpotAngles>,
    pub shadow_mode: ShadowMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LightKind {
    Directional,
    Point,
    Spot,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpotAngles {
    pub inner: f32,
    pub outer: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShadowMode {
    Off,
    Hard,
    Soft,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MaterialBinding {
    pub material_id: AssetId,
    pub pipeline: AssetId,
    pub variant_key: u64,
    pub textures: Vec<TextureSlot>,
    pub uniforms: ParamBlock,
    pub pass_mask: u32,
    pub transparency: Transparency,
    pub double_sided: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextureSlot {
    pub binding: u32,
    pub texture: AssetId,
    pub sampler: AssetId,
    pub color_space: ColorSpace,
    pub mip_bias: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColorSpace {
    Linear,
    Srgb,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ParamBlock {
    pub bytes: Vec<u8>,
    pub layout_hash: HashDigest,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Transparency {
    Opaque,
    Masked { cutoff: f32 },
    Blend,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MeshBinding {
    pub mesh_id: AssetId,
    pub vertex_layout: VertexLayout,
    pub index_format: IndexFormat,
    pub submeshes: Vec<Submesh>,
    pub bounds: AxisAlignedBox,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VertexLayout {
    pub stride_bytes: u32,
    pub attributes: Vec<VertexAttribute>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VertexAttribute {
    pub semantic: VertexSemantic,
    pub format: VertexAttributeFormat,
    pub offset_bytes: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum VertexSemantic {
    Position,
    Normal,
    Tangent,
    Uv0,
    Uv1,
    Color0,
    Joints0,
    Weights0,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum VertexAttributeFormat {
    Float32x2,
    Float32x3,
    Float32x4,
    Uint16x4,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum IndexFormat {
    U16,
    U32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Submesh {
    pub name: String,
    pub index_offset: u32,
    pub index_count: u32,
    pub material_slot: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DebugPrimitive {
    pub source_system: String,
    pub severity: DiagnosticSeverity,
    pub primitive_kind: DebugPrimitiveKind,
    pub color: LinearRgb,
    pub lifetime_frames: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum DebugPrimitiveKind {
    Line {
        from: Vec3,
        to: Vec3,
    },
    Triangle {
        a: Vec3,
        b: Vec3,
        c: Vec3,
    },
    Sphere {
        center: Vec3,
        radius: f32,
    },
    Box {
        center: Vec3,
        half_extents: Vec3,
        rotation: Quat,
    },
    Text {
        position: Vec3,
        text: String,
        size_px: f32,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UiBatch {
    pub canvas_id: PersistentId,
    pub z_order: i32,
    pub clip_rect: Rect,
    pub texture: Option<AssetId>,
    pub vertices: Vec<UiVertex>,
    pub indices: Vec<u32>,
    pub material: AssetId,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct UiVertex {
    pub position: Vec2,
    pub uv: Vec2,
    pub color: [u8; 4],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RenderOptions {
    pub tone_mapping: ToneMapping,
    pub exposure_ev100: Option<f32>,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            tone_mapping: ToneMapping::Aces,
            exposure_ev100: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToneMapping {
    Aces,
    Reinhard,
    None,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FrameStats {
    pub visible_drawables: u32,
    pub culled_drawables: u32,
    pub visible_lights: u32,
    pub culled_lights: u32,
    pub draw_calls: u32,
    pub triangles: u64,
    pub gpu_frame_ms: f32,
}

pub fn validate_frame_input(input: &RenderFrameInput) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    if !input.contract_version.starts_with("RendererInput-v0") {
        diagnostics.push(
            Diagnostic::new(
                "RV0012",
                DiagnosticSeverity::Error,
                "engine-renderer",
                "renderer input contract version is not RendererInput-v0",
            )
            .contract("RendererInput-v0", input.contract_version.clone()),
        );
    }
    if input.views.is_empty() {
        diagnostics.push(
            Diagnostic::new(
                "RV0013",
                DiagnosticSeverity::Error,
                "engine-renderer",
                "renderer input contains no render views",
            )
            .contract("RendererInput-v0", input.contract_version.clone())
            .path("views"),
        );
    }

    let mut view_ids = BTreeSet::new();
    for view in &input.views {
        if !view_ids.insert(view.view_id) {
            diagnostics.push(
                Diagnostic::new(
                    "RV0014",
                    DiagnosticSeverity::Error,
                    "engine-renderer",
                    "RenderView.view_id values must be unique",
                )
                .contract("RendererInput-v0", input.contract_version.clone())
                .path("views.view_id"),
            );
        }
    }
    for view in &input.views {
        if let ViewCompose::Overlay { base_view_id, .. } = view.compose {
            if !view_ids.contains(&base_view_id) {
                diagnostics.push(
                    Diagnostic::new(
                        "RV0007",
                        DiagnosticSeverity::Warning,
                        "engine-renderer",
                        "overlay render view references a missing base view",
                    )
                    .contract("RendererInput-v0", input.contract_version.clone()),
                );
            }
        }
    }

    // Light validation diagnostics (Gate 3 acceptance)
    for (light_idx, light) in input.lights.iter().enumerate() {
        // ShadowMode::Hard or Soft on point/spot lights produces diagnostic
        // and is downgraded (the frame never aborts — Warning only)
        if matches!(light.kind, LightKind::Point | LightKind::Spot) {
            if matches!(light.shadow_mode, ShadowMode::Hard | ShadowMode::Soft) {
                let entity_id = light
                    .entity
                    .as_ref()
                    .map(|e| e.to_string())
                    .unwrap_or_else(|| "<unknown>".to_string());
                diagnostics.push(
                    Diagnostic::new(
                        "RV0015",
                        DiagnosticSeverity::Warning,
                        "engine-renderer",
                        &format!(
                            "ShadowMode::{:?} is not supported for {:?} light (entity {}); downgraded to Off",
                            light.shadow_mode, light.kind, entity_id
                        ),
                    )
                    .contract("RendererInput-v0", input.contract_version.clone())
                    .path(format!("lights[{light_idx}].shadow_mode")),
                );
            }
        }

        // Intensity must be positive
        if light.intensity <= 0.0 {
            diagnostics.push(
                Diagnostic::new(
                    "RV0016",
                    DiagnosticSeverity::Warning,
                    "engine-renderer",
                    &format!(
                        "Light intensity must be positive (got {}) for {:?} light",
                        light.intensity, light.kind
                    ),
                )
                .contract("RendererInput-v0", input.contract_version.clone())
                .path(format!("lights[{light_idx}].intensity")),
            );
        }
    }

    diagnostics
}

/// Backend renderer trait — implemented by concrete rendering backends
/// (Vulkan, OpenGL, DX12) to bridge scene input to GPU execution.
pub trait BackendRenderer: Send {
    /// Render one frame from the given scene input (legacy single-pass path).
    fn render_frame(&mut self, input: &RenderFrameInput) -> Result<FrameStats, Vec<Diagnostic>>;

    /// Begin a new frame. Called once before [`execute_pass`](Self::execute_pass).
    /// Default: no-op, rendering happens in render_frame.
    fn begin_frame(&mut self, _input: &RenderFrameInput) -> Result<(), Vec<Diagnostic>> {
        Ok(())
    }

    /// End the current frame. Called once after all passes.
    /// Default: no-op.
    fn end_frame(&mut self, _stats: &mut FrameStats) -> Result<(), Vec<Diagnostic>> {
        Ok(())
    }

    /// Execute a single render-graph pass. The default implementation
    /// delegates to [`render_frame`](Self::render_frame) for backwards compat.
    fn execute_pass(
        &mut self,
        input: &RenderFrameInput,
        pass: &render_graph::PassNode,
        frame_stats: &mut FrameStats,
    ) -> Result<(), Vec<Diagnostic>> {
        let _ = pass;
        let _ = frame_stats;
        self.render_frame(input).map(|_| ())
    }
}

pub struct Renderer {
    backend: Option<Box<dyn BackendRenderer>>,
}

impl Renderer {
    pub fn new() -> Self {
        Self { backend: None }
    }

    pub fn new_with_backend(backend: Box<dyn BackendRenderer>) -> Self {
        Self {
            backend: Some(backend),
        }
    }

    pub fn set_backend(&mut self, backend: Box<dyn BackendRenderer>) {
        self.backend = Some(backend);
    }

    /// Render a frame by building the render graph and executing each pass.
    pub fn draw_scene(&mut self, input: &RenderFrameInput) -> Result<FrameStats, Vec<Diagnostic>> {
        let diagnostics = validate_frame_input(input);
        if diagnostics.iter().any(|d| {
            matches!(
                d.severity,
                DiagnosticSeverity::Error | DiagnosticSeverity::Fatal
            )
        }) {
            return Err(diagnostics);
        }

        if let Some(backend) = self.backend.as_mut() {
            // Build the render graph from the frame input
            let graph = render_graph::RenderGraph::build(input);

            let mut stats = FrameStats::default();

            // Begin frame (backend allocates per-frame resources)
            backend.begin_frame(input)?;

            // Execute each pass with tracing spans
            for pass in &graph.passes {
                let span = tracing::info_span!("frame.view.{}.{}", input.frame_index, pass.name);
                let _guard = span.enter();
                tracing::info!(pass = pass.name, "executing render pass");

                backend.execute_pass(input, pass, &mut stats)?;
            }

            // End frame (backend submits and presents)
            backend.end_frame(&mut stats)?;

            Ok(stats)
        } else {
            // No backend attached — return mock stats (for contract-only testing)
            Ok(FrameStats {
                visible_drawables: input.drawables.len() as u32 + input.skinned_items.len() as u32,
                visible_lights: input.lights.len() as u32,
                draw_calls: input.drawables.len() as u32 + input.skinned_items.len() as u32,
                ..FrameStats::default()
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{RenderFrameInput, Renderer};

    #[test]
    fn empty_frame_is_rejected() {
        let input = RenderFrameInput::empty(0);
        assert!(Renderer::new().draw_scene(&input).is_err());
    }
}
