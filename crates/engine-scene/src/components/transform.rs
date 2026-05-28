use crate::{Component, Entity};
use serde::{Deserialize, Serialize};

/// Local transform of an entity in world-space or parent-space.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Transform {
    pub translation: glam::Vec3,
    pub rotation: glam::Quat,
    pub scale: glam::Vec3,
    pub parent: Option<Entity>,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            translation: glam::Vec3::ZERO,
            rotation: glam::Quat::IDENTITY,
            scale: glam::Vec3::ONE,
            parent: None,
        }
    }
}

impl Component for Transform {
    const TYPE_ID: &'static str = "engine.transform";
}
