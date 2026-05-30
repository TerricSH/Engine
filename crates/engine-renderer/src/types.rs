pub use engine_serialize::{
    AssetId, ContractVersion, Diagnostic, DiagnosticSeverity, HashDigest, PersistentId,
};
use serde::{Deserialize, Serialize};

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
    /// Number of MSAA samples (1 = disabled, 2/4/8 = MSAA).
    pub msaa_samples: u8,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            tone_mapping: ToneMapping::Aces,
            exposure_ev100: None,
            msaa_samples: 1,
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
