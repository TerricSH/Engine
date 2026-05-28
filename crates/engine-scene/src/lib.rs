#![forbid(unsafe_code)]

mod scene;
mod validation;
mod extraction;
mod entity;
mod component;
pub mod components;
mod world;

pub use scene::*;
pub use validation::validate_scene;
pub use extraction::extract_renderer_input;
pub use entity::{Entity, EntityManager};
pub use component::{Component, SparseSet, ComponentStorageDyn};
pub use world::World;

#[cfg(test)]
mod tests;
