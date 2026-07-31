use crate::Component;
use serde::{Deserialize, Serialize};

/// Runtime render component that continuously moves a detailed mesh toward a
/// coarser radial surface. Terrain streaming owns the encoded vertex deltas;
/// render extraction only transports these generic parameters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct VertexGeomorph {
    pub factor: f32,
    pub delta_scale: f32,
    pub local_origin: [f32; 3],
}

impl Component for VertexGeomorph {
    const TYPE_ID: &'static str = "engine.vertex-geomorph";
}
