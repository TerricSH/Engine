use crate::Component;
use serde::{Deserialize, Serialize};

/// Marks an entity as renderable with a mesh and material.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Renderable {
    pub mesh_asset: String,
    pub material_asset: String,
    pub visible: bool,
    pub cast_shadows: bool,
    pub render_layer: String,
}

impl Component for Renderable {
    const TYPE_ID: &'static str = "engine.renderable";
}
