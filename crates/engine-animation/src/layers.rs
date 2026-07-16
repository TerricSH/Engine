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
    /// Animation clip asset sampled by this layer. `None` is valid for the
    /// base layer, whose clip comes from the player or state machine.
    #[serde(default)]
    pub clip_asset: Option<String>,
    /// Blend weight in `0..1`.
    pub weight: f32,
    pub blend_mode: LayerBlendMode,
    /// If non-empty, only affects these bone indices (by `BoneIndex.0`).
    pub bone_mask: Vec<u16>,
    /// Independent playback cursor for this layer.
    #[serde(default)]
    pub current_time: f32,
    #[serde(default = "default_speed")]
    pub speed: f32,
    #[serde(default = "default_looping")]
    pub looping: bool,
}

const fn default_speed() -> f32 {
    1.0
}

const fn default_looping() -> bool {
    true
}

impl AnimLayer {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            clip_asset: None,
            weight: 1.0,
            blend_mode: LayerBlendMode::Overwrite,
            bone_mask: Vec::new(),
            current_time: 0.0,
            speed: 1.0,
            looping: true,
        }
    }

    pub fn with_weight(mut self, weight: f32) -> Self {
        self.weight = if weight.is_finite() {
            weight.clamp(0.0, 1.0)
        } else {
            0.0
        };
        self
    }

    pub fn with_clip(mut self, clip_asset: impl Into<String>) -> Self {
        self.clip_asset = Some(clip_asset.into());
        self.current_time = 0.0;
        self
    }

    pub fn with_speed(mut self, speed: f32) -> Self {
        self.speed = if speed.is_finite() { speed } else { 1.0 };
        self
    }

    pub fn with_looping(mut self, looping: bool) -> Self {
        self.looping = looping;
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
