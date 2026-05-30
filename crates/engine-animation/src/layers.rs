use serde::{Deserialize, Serialize};

/// Blend mode for an animation layer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum LayerBlendMode {
    /// Overwrite lower layers completely (base layer behaviour).
    Overwrite,
    /// Additive blending on top of lower layers.
    Additive,
}

/// Configuration for a single animation layer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnimLayer {
    pub name: String,
    /// Blend weight in `0..1`.
    pub weight: f32,
    pub blend_mode: LayerBlendMode,
    /// If non-empty, only affects these bone indices (by `BoneIndex.0`).
    pub bone_mask: Vec<u16>,
}

impl AnimLayer {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            weight: 1.0,
            blend_mode: LayerBlendMode::Overwrite,
            bone_mask: Vec::new(),
        }
    }

    pub fn with_weight(mut self, weight: f32) -> Self {
        self.weight = weight.clamp(0.0, 1.0);
        self
    }

    pub fn with_mask(mut self, mask: Vec<u16>) -> Self {
        self.bone_mask = mask;
        self
    }

    pub fn with_blend_mode(mut self, mode: LayerBlendMode) -> Self {
        self.blend_mode = mode;
        self
    }
}
