#![forbid(unsafe_code)]

mod scene;
mod validation;
mod extraction;
mod entity;
mod component;
pub mod components;
mod world;
pub mod registry;

pub use scene::*;
pub use validation::validate_scene;
pub use extraction::{
    extract_renderer_input, extract_renderer_input_from_world, extract_frustum_planes,
    aabb_in_frustum,
};
pub use entity::{Entity, EntityManager};
pub use component::{Component, SparseSet, ComponentStorageDyn};
pub use registry::{
    ComponentRegistry, ComponentExtension, ComponentMeta,
    AssetTypeRegistry, AssetTypeExtension, AssetTypeMeta,
};
pub use world::World;

#[cfg(test)]
mod tests;
