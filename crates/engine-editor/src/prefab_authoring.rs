//! Transactional prefab asset authoring and undoable scene integration.

mod error;
mod instantiation;
mod source;
mod unpack;
mod util;

pub use error::PrefabAuthoringError;
pub use instantiation::{
    prepare_prefab_instantiation, prepare_prefab_instantiation_from_registry,
    prepare_prefab_instantiation_from_source, PrefabInstantiationPlan,
};
pub use source::{
    create_prefab_asset_from_scene, load_prefab_source, prefab_from_scene_subtree,
    CreatedPrefabAsset, PrefabAssetCreateRequest, PREFAB_SOURCE_SUFFIX,
};
pub use unpack::{prepare_unpack_prefab, PrefabUnpackMode, PrefabUnpackPlan};

#[cfg(test)]
mod tests;
