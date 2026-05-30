#![forbid(unsafe_code)]

mod component;
pub mod components;
mod entity;
mod extraction;
pub mod prefab;
pub mod registry;
mod scene;
mod validation;
mod world;

pub use component::{Component, ComponentStorageDyn, SparseSet};
pub use entity::{Entity, EntityManager};
pub use extraction::{
    aabb_in_frustum, extract_frustum_planes, extract_renderer_input,
    extract_renderer_input_from_world,
};
pub use prefab::{Prefab, PREFAB_CONTRACT, PREFAB_SCHEMA_VERSION};
pub use registry::{
    AssetTypeExtension, AssetTypeMeta, AssetTypeRegistry, ComponentExtension, ComponentMeta,
    ComponentRegistry,
};
pub use scene::*;
pub use validation::validate_scene;
pub use world::World;

#[cfg(test)]
mod tests;
