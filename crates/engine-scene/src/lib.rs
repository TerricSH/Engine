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
    aabb_in_frustum, active_camera_world_position, camera_relative_render_origin,
    entity_world_position, entity_world_transform, extract_frustum_planes,
    extract_renderer_input_from_world, extract_renderer_input_from_world_with_viewport,
    render_layer_bit, RenderViewportContext,
};
pub use pool::{ObjectPool, ObjectPoolError};
pub use prefab::{
    detect_prefab_cycles, parse_prefab_source, prefab_cooker, prefab_loader,
    register_prefab_asset_type, serialize_prefab_source, validate_prefab,
    validate_prefab_structure, Prefab, PrefabChildRef, PrefabValidationError, PREFAB_CONTRACT,
    PREFAB_SCHEMA_VERSION,
};
pub use prefab_instance::{
    instantiate_prefab, instantiate_prefab_from_asset, PrefabInstanceRef, PrefabInstantiateError,
    PrefabInstantiateResult, PrefabLoad, PrefabRegistry,
};
pub use registry::{
    AssetTypeExtension, AssetTypeMeta, AssetTypeRegistry, ComponentExtension, ComponentMeta,
    ComponentRegistry, ScriptAccess,
};
pub use scene::*;
pub use validation::{
    validate_scene, validate_scene_for_authoring, SceneAuthoringFailure,
    SceneAuthoringValidationError, SCENE_ONLY_COMPONENT_TYPES,
};
pub use world::merge::{DestroySubtreeError, DestroyedSubtree, MergeError};
pub use world::{PersistentEntityCreateError, World, WorldOriginRestoreError};
pub use world_slot::{WeakWorldSlot, WorldSlot};

#[cfg(test)]
mod tests;
