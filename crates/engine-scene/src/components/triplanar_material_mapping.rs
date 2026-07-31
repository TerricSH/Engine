use crate::Component;
use serde::{Deserialize, Serialize};

/// Transient per-renderable coordinates for continuous projected PBR textures.
///
/// `local_origin` is expressed in the renderable mesh's local frame. Keeping
/// the origin close to the streamed patch avoids precision loss at large
/// logical-world coordinates while still allowing world- or planet-relative
/// mapping. This component is produced by geometry systems; it is not an
/// authored material or gameplay/biome policy.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TriplanarMaterialMapping {
    pub local_origin: [f32; 3],
    pub meters_per_tile: f32,
    pub blend_sharpness: f32,
}

impl Default for TriplanarMaterialMapping {
    fn default() -> Self {
        Self {
            local_origin: [0.0; 3],
            meters_per_tile: 1.0,
            blend_sharpness: 4.0,
        }
    }
}

impl Component for TriplanarMaterialMapping {
    const TYPE_ID: &'static str = "engine.triplanar-material-mapping";
}
