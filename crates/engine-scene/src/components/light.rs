use crate::Component;
use serde::{Deserialize, Serialize};

/// Kind of light source.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LightKind {
    Directional,
    Point,
    Spot,
}

/// Light component.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Light {
    pub kind: LightKind,
    pub color: [f32; 3],
    /// Lux for directional, lumens for point/spot.
    pub intensity: f32,
    /// Maximum range of the light (for culling).
    pub range: f32,
    /// Inner/outer cone angles in radians (only for Spot).
    pub spot_angles: Option<[f32; 2]>,
    /// Shadow mode: 0 = off, 1 = hard, 2 = soft.
    pub shadow_mode: u8,
    /// Light direction in world space.
    pub direction: [f32; 3],
}

impl Component for Light {
    const TYPE_ID: &'static str = "engine.light";
}
