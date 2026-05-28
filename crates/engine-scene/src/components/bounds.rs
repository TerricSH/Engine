use crate::Component;
use serde::{Deserialize, Serialize};

/// Axis-aligned bounding box for an entity.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Bounds {
    pub center: [f32; 3],
    pub half_extents: [f32; 3],
}

impl Component for Bounds {
    const TYPE_ID: &'static str = "engine.bounds";
}
