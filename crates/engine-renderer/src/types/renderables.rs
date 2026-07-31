use serde::{Deserialize, Serialize};

use super::{AssetId, AxisAlignedBox, BonePaletteLayout, Mat4, PersistentId};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RadialVertexMorph {
    /// Morph factor from the detailed vertex (0) to its parent surface (1).
    pub factor: f32,
    /// Scale used to decode the signed radial delta stored in normal length.
    pub delta_scale: f32,
    /// Planet center in mesh-local coordinates.
    pub local_origin: [f32; 3],
}

/// Per-draw coordinates for continuous world/planet-relative material
/// projection. Backends apply this mapping consistently to every PBR texture
/// slot; renderables without it retain their authored mesh UV path.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TriplanarMaterialMapping {
    /// Projection origin in mesh-local coordinates.
    pub local_origin: [f32; 3],
    /// World-space length represented by one texture repeat.
    pub meters_per_tile: f32,
    /// Exponent controlling the blend between projection axes.
    pub blend_sharpness: f32,
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
    /// Optional continuous terrain LOD morph evaluated by capable backends.
    #[serde(default)]
    pub radial_vertex_morph: Option<RadialVertexMorph>,
    /// Optional continuous material mapping evaluated by capable backends.
    #[serde(default)]
    pub triplanar_material_mapping: Option<TriplanarMaterialMapping>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SkinnedItem {
    pub entity: Option<PersistentId>,
    pub mesh: AssetId,
    pub material: AssetId,
    pub skeleton: AssetId,
    pub bone_palette: Vec<Mat4>,
    pub bone_palette_layout: BonePaletteLayout,
    /// Optional GPU morph-target set applied before skeletal skinning.
    #[serde(default)]
    pub morph_target_set: Option<AssetId>,
    /// Per-target weights in asset order (up to eight).
    #[serde(default)]
    pub morph_weights: Vec<f32>,
    pub world_transform: Mat4,
    pub bounds: AxisAlignedBox,
    pub render_layer: String,
    pub cast_shadows: bool,
    pub sort_key: u64,
}
