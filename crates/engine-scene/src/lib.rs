#![forbid(unsafe_code)]

pub mod archetype;
pub mod camera_utils;
mod component;
pub mod components;
mod entity;
mod extraction;
pub mod pool;
pub mod prefab;
pub mod prefab_instance;
pub mod prefab_override;
pub mod registry;
mod scene;
pub mod third_person_camera;
mod validation;
mod world;
mod world_slot;

pub use archetype::{Archetype, ArchetypeRegistry};
pub use component::{Component, ComponentStorageDyn, SparseSet};
pub use entity::{Entity, EntityManager};
pub use extraction::{
    aabb_in_frustum, extract_frustum_planes, extract_renderer_input,
    extract_renderer_input_from_world,
};
pub use pool::{ObjectPool, ObjectPoolError};
pub use prefab::{
    detect_prefab_cycles, prefab_cooker, prefab_loader, register_prefab_asset_type,
    validate_prefab, Prefab, PrefabChildRef, PREFAB_CONTRACT, PREFAB_SCHEMA_VERSION,
};
pub use prefab_instance::{PrefabInstanceRef, PrefabInstantiateResult, PrefabLoad, PrefabRegistry};
pub use prefab_override::{OverrideRecord, OverrideSet};
pub use registry::{
    AssetTypeExtension, AssetTypeMeta, AssetTypeRegistry, ComponentExtension, ComponentMeta,
    ComponentRegistry,
};
pub use scene::*;
pub use validation::validate_scene;
pub use world::{PersistentEntityCreateError, World};
pub use world_slot::{WeakWorldSlot, WorldSlot};

#[cfg(test)]
mod tests;
