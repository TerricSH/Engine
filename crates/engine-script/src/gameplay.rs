//! Data-only gameplay bridge shared by script hosts and the engine runtime.
//!
//! Process-based hosts cannot call the engine's in-process FFI directly. The
//! runtime therefore sends each script a frame snapshot and applies the
//! commands returned after the lifecycle call completes.

mod commands;
mod components;
mod context;
mod events;
mod physics_mutation;
mod physics_query;
mod runtime_assets;
mod snapshots;
mod ui;
mod validation;

pub use commands::{
    GameplayCommand, GameplaySaveOperation, OwnedGameplayCommand, OwnedGameplayDamageRequest,
    OwnedGameplayPhysicsMutation, OwnedGameplayPhysicsQuery, OwnedGameplayRagdollRequest,
    OwnedGameplaySaveRequest,
};
#[cfg(test)]
pub use components::validate_component_field_name;
pub use components::{
    validate_component_fields, validate_component_type_key, GameplayComponentQuery,
    GameplayComponentQueryResult, GameplayComponentValue, OwnedGameplayComponentQuery,
    MAX_COMPONENT_FIELDS, MAX_COMPONENT_LIST_ITEMS, MAX_COMPONENT_VALUE_DEPTH,
    MAX_COMPONENT_VALUE_STRING_BYTES, MAX_PENDING_COMPONENT_QUERIES,
};
pub use context::GameplayContext;
pub use events::{
    GameplayDamageEvent, GameplayDamageKind, GameplayPhysicsEvent, GameplayPhysicsEventKind,
    GameplayRagdollEvent, MAX_CHARACTER_CONTROL_SPEED, MAX_DAMAGE_AMOUNT,
    MAX_PENDING_DAMAGE_REQUESTS, MAX_PENDING_RAGDOLL_REQUESTS, MAX_RAGDOLL_RECOVERY_SECONDS,
};
pub use physics_mutation::{
    GameplayJointLimits, GameplayJointMotor, GameplayJointType, GameplayPhysicsMutation,
    MAX_PENDING_PHYSICS_MUTATIONS, MAX_PENDING_PHYSICS_QUERIES, MAX_PHYSICS_MUTATION_COMPONENT,
    MAX_PHYSICS_OVERLAP_RESULTS, MAX_PHYSICS_QUERY_DISTANCE,
};
pub use physics_query::{
    GameplayInteractionSnapshot, GameplayPhysicsQuery, GameplayPhysicsQueryFilter,
    GameplayPhysicsQueryResult,
};
pub use runtime_assets::*;
pub use snapshots::{
    GameplayAnimationParameterValue, GameplayCameraSnapshot, GameplayEntitySnapshot,
    GameplayInputTransitions, GameplayInputValue, GameplayLogicAssetResult,
    GameplayPointerSnapshot, GameplaySaveEvent, GameplaySaveEventKind, ScriptTransform,
    MAX_ANIMATION_SPEED, MAX_PENDING_LOGIC_ASSET_QUERIES, MAX_PENDING_SAVE_REQUESTS,
    MAX_SCRIPT_LOGIC_ASSET_JSON_BYTES, MAX_SCRIPT_SAVE_STATE_BYTES,
};
pub use ui::{
    GameplayUiColor, GameplayUiCommand, GameplayUiElement, GameplayUiEvent, GameplayUiLayout,
    GameplayUiScaleMode, GameplayUiValue,
};
pub use validation::{
    validate_entity_id, validate_prefab_id, validate_save_slot, validate_scene_id,
    validate_script_transform,
};

#[cfg(test)]
mod tests;
