use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::components::{
    validate_component_fields, validate_component_type_key, GameplayComponentQuery,
    GameplayComponentValue,
};
use super::events::{
    GameplayDamageKind, MAX_CHARACTER_CONTROL_SPEED, MAX_DAMAGE_AMOUNT,
    MAX_RAGDOLL_RECOVERY_SECONDS,
};
use super::physics_mutation::{GameplayPhysicsMutation, MAX_PHYSICS_MUTATION_COMPONENT};
use super::physics_query::GameplayPhysicsQuery;
use super::snapshots::{
    GameplayAnimationParameterValue, ScriptTransform, MAX_ANIMATION_SPEED,
    MAX_SCRIPT_SAVE_STATE_BYTES,
};
use super::ui::GameplayUiCommand;
use super::validation::{
    validate_entity_id, validate_prefab_id, validate_save_slot, validate_scene_id,
    validate_script_transform,
};
use super::{
    validate_runtime_asset_id, GameplayNetworkCommand, GameplayRuntimeMaterial,
    GameplayRuntimeMesh, GameplayRuntimePrefab, GameplayTerrainBrush,
};
/// Mutations a script may request after running a lifecycle method.
///
/// Every command is still bound to the instance's owning entity by the Rust
/// manager. Commands with an explicit target carry a *persistent entity id*,
/// which the runtime validates and resolves against the current World at the
/// frame boundary; a script can never forge an ECS handle.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GameplayCommand {
    SetTransform {
        transform: ScriptTransform,
    },
    /// Replace another persistent entity's Transform at the frame boundary.
    SetEntityTransform {
        entity_id: String,
        transform: ScriptTransform,
    },
    /// Create a persistent entity with a Transform at the frame boundary.
    CreateEntity {
        entity_id: String,
        transform: ScriptTransform,
    },
    /// Destroy the script's owning entity at the frame boundary.
    DestroySelf,
    /// Destroy another persistent entity at the frame boundary.
    DestroyEntity {
        entity_id: String,
    },
    /// Request a scene change after the current script update finishes.
    ///
    /// The runtime resolves `scene_id` against the project's named scene
    /// catalog. Process hosts validate this value before exposing the command
    /// to the engine, but other hosts can call [`Self::validate`] explicitly.
    LoadScene {
        scene_id: String,
    },
    /// Instantiate a cooked prefab asset at the frame boundary.
    ///
    /// The runtime resolves `prefab_id` against the prefab assets loaded from
    /// the project's cooked asset batch. The spawned instance root receives
    /// the first free persistent id from `prefab_id`, `prefab_id-2`, and so
    /// on; every other prefab entity receives `<rootId>.<prefab-local id>`
    /// (with the same `-N` conflict suffix). `translation`, when present,
    /// overrides the root entity's translation while keeping the prefab's
    /// rotation and scale.
    SpawnPrefab {
        prefab_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        translation: Option<[f32; 3]>,
    },
    /// Apply bounded damage to a persistent Destructible entity.
    ApplyDamage {
        entity_id: String,
        amount: f32,
        #[serde(default)]
        damage_kind: GameplayDamageKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hit_position: Option<[f32; 3]>,
        #[serde(default)]
        impulse: [f32; 3],
    },
    /// Switch an authored ragdoll between animation and physics ownership.
    SetRagdoll {
        entity_id: String,
        active: bool,
        #[serde(default)]
        recovery_duration: f32,
        #[serde(default)]
        impulse: [f32; 3],
    },
    /// Queue movement intent for a persistent CharacterController.
    ///
    /// The controller consumes this command on its next simulation update;
    /// scripts express movement intent rather than writing transforms.
    CharacterControl {
        entity_id: String,
        direction: [f32; 3],
        #[serde(default)]
        jump: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        speed: Option<f32>,
    },
    /// Mutate retained runtime UI through the managed class API.
    Ui {
        command: GameplayUiCommand,
    },
    /// Request an active physics query against the current physics world.
    ///
    /// The query is validated and executed at the frame boundary; the result
    /// arrives in the next frame's [`GameplayContext::physics_query_results`].
    PhysicsQuery {
        query: GameplayPhysicsQuery,
    },
    /// Queue a force or impulse for a persistent rigid body.
    ///
    /// The mutation is resolved after script callbacks complete and is
    /// executed safely at the start of the next physics step.
    PhysicsMutation {
        mutation: GameplayPhysicsMutation,
    },
    /// Request a snapshot of one script-accessible component on a persistent
    /// entity.
    ///
    /// The query is validated at the frame boundary; the snapshot (or an
    /// explicit miss) arrives in the next frame's
    /// [`GameplayContext::component_query_results`].
    ComponentQuery {
        query: GameplayComponentQuery,
    },
    /// Merge script-provided fields into one script-accessible component on a
    /// persistent entity at the frame boundary.
    ///
    /// Fields not present in `fields` keep their current values (or the
    /// component's authored defaults when the entity does not carry the
    /// component yet). The engine re-validates every field through the
    /// component's registered scene serde hooks before committing, so unknown
    /// field names, mismatched value types, and unsupported enum cases are
    /// rejected with a script diagnostic and never partially applied.
    SetComponent {
        entity_id: String,
        component_type: String,
        fields: BTreeMap<String, GameplayComponentValue>,
    },
    /// Play a direct animation clip through the target's AnimationPlayer.
    PlayAnimation {
        entity_id: String,
        clip_asset: String,
        #[serde(default = "default_true")]
        looping: bool,
        #[serde(default = "default_animation_speed")]
        speed: f32,
        #[serde(default = "default_true")]
        restart: bool,
    },
    /// Set a parameter on an authored animation state-machine instance.
    SetAnimationParameter {
        entity_id: String,
        name: String,
        value: GameplayAnimationParameterValue,
    },
    /// Force an authored animation state machine to a named state.
    TransitionAnimationState {
        entity_id: String,
        state: String,
    },
    /// Pause or resume the target AnimationPlayer without changing its clip.
    SetAnimationPlaying {
        entity_id: String,
        playing: bool,
    },
    /// Update the bounded morph target weights on the target Skeleton.
    SetMorphWeights {
        entity_id: String,
        weights: Vec<f32>,
    },
    /// Capture the live engine state plus one project-owned JSON document.
    SaveCheckpoint {
        slot: String,
        state_json: String,
    },
    /// Restore a checkpoint from a configured save slot.
    LoadCheckpoint {
        slot: String,
    },
    /// Query a cooked behavior/state/skill/quest logic asset by ID.
    QueryLogicAsset {
        query_id: u32,
        asset_id: String,
    },
    RegisterRuntimeMesh {
        request_id: u32,
        asset_id: String,
        mesh: GameplayRuntimeMesh,
    },
    RegisterRuntimeMaterial {
        request_id: u32,
        asset_id: String,
        material: GameplayRuntimeMaterial,
    },
    RegisterRuntimePrefab {
        request_id: u32,
        asset_id: String,
        prefab: GameplayRuntimePrefab,
    },
    TerrainApplyBrush {
        request_id: u32,
        terrain_entity_id: String,
        brush: GameplayTerrainBrush,
    },
    Network {
        request_id: u32,
        network: GameplayNetworkCommand,
    },
}

impl GameplayCommand {
    /// Validate untrusted command data received from a script host.
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::SetTransform { transform } => validate_script_transform(transform),
            Self::SetEntityTransform {
                entity_id,
                transform,
            }
            | Self::CreateEntity {
                entity_id,
                transform,
            } => {
                validate_entity_id(entity_id)?;
                validate_script_transform(transform)
            }
            Self::DestroySelf => Ok(()),
            Self::DestroyEntity { entity_id } => validate_entity_id(entity_id),
            Self::LoadScene { scene_id } => validate_scene_id(scene_id),
            Self::SpawnPrefab {
                prefab_id,
                translation,
            } => {
                validate_prefab_id(prefab_id)?;
                if let Some(translation) = translation {
                    if !translation.iter().all(|value| value.is_finite()) {
                        return Err("spawn translation must contain only finite values".into());
                    }
                }
                Ok(())
            }
            Self::ApplyDamage {
                entity_id,
                amount,
                hit_position,
                impulse,
                ..
            } => {
                validate_entity_id(entity_id)?;
                if !amount.is_finite() || *amount <= 0.0 || *amount > MAX_DAMAGE_AMOUNT {
                    return Err(format!(
                        "damage amount must be finite, greater than zero, and at most {MAX_DAMAGE_AMOUNT}"
                    ));
                }
                if !hit_position
                    .iter()
                    .flatten()
                    .chain(impulse.iter())
                    .all(|value| value.is_finite() && value.abs() <= MAX_PHYSICS_MUTATION_COMPONENT)
                {
                    return Err(
                        "damage position/impulse must be finite and within the physics mutation bound"
                            .into(),
                    );
                }
                Ok(())
            }
            Self::SetRagdoll {
                entity_id,
                recovery_duration,
                impulse,
                ..
            } => {
                validate_entity_id(entity_id)?;
                if !recovery_duration.is_finite()
                    || *recovery_duration < 0.0
                    || *recovery_duration > MAX_RAGDOLL_RECOVERY_SECONDS
                {
                    return Err(format!(
                        "ragdoll recovery duration must be finite, non-negative, and at most {MAX_RAGDOLL_RECOVERY_SECONDS} seconds"
                    ));
                }
                if !impulse
                    .iter()
                    .all(|value| value.is_finite() && value.abs() <= MAX_PHYSICS_MUTATION_COMPONENT)
                {
                    return Err(
                        "ragdoll impulse must be finite and within the physics mutation bound"
                            .into(),
                    );
                }
                Ok(())
            }
            Self::CharacterControl {
                entity_id,
                direction,
                speed,
                ..
            } => {
                validate_entity_id(entity_id)?;
                if !direction.iter().all(|value| value.is_finite()) {
                    return Err("character direction must contain only finite values".into());
                }
                let horizontal_length_squared =
                    direction[0] * direction[0] + direction[2] * direction[2];
                if horizontal_length_squared > 1.000_2 {
                    return Err(
                        "character direction must have horizontal length no greater than one"
                            .into(),
                    );
                }
                if direction[1].abs() > f32::EPSILON {
                    return Err("character direction must be horizontal (Y must be zero)".into());
                }
                if let Some(speed) = speed {
                    if !speed.is_finite() || *speed <= 0.0 || *speed > MAX_CHARACTER_CONTROL_SPEED {
                        return Err(format!(
                            "character speed must be finite, greater than zero, and at most {MAX_CHARACTER_CONTROL_SPEED}"
                        ));
                    }
                }
                Ok(())
            }
            Self::Ui { command } => command.validate(),
            Self::PhysicsQuery { query } => query.validate(),
            Self::PhysicsMutation { mutation } => mutation.validate(),
            Self::ComponentQuery { query } => query.validate(),
            Self::SetComponent {
                entity_id,
                component_type,
                fields,
            } => {
                validate_entity_id(entity_id)?;
                validate_component_type_key(component_type)?;
                validate_component_fields(fields)
            }
            Self::PlayAnimation {
                entity_id,
                clip_asset,
                speed,
                ..
            } => {
                validate_entity_id(entity_id)?;
                validate_entity_id(clip_asset)?;
                if !speed.is_finite() || *speed <= 0.0 || *speed > MAX_ANIMATION_SPEED {
                    return Err(format!(
                        "animation speed must be finite, greater than zero, and at most {MAX_ANIMATION_SPEED}"
                    ));
                }
                Ok(())
            }
            Self::SetAnimationParameter {
                entity_id,
                name,
                value,
            } => {
                validate_entity_id(entity_id)?;
                validate_component_type_key(name)?;
                if matches!(value, GameplayAnimationParameterValue::Float(value) if !value.is_finite())
                {
                    return Err("animation float parameter must be finite".into());
                }
                Ok(())
            }
            Self::TransitionAnimationState { entity_id, state } => {
                validate_entity_id(entity_id)?;
                validate_component_type_key(state)
            }
            Self::SetAnimationPlaying { entity_id, .. } => validate_entity_id(entity_id),
            Self::SetMorphWeights { entity_id, weights } => {
                validate_entity_id(entity_id)?;
                if weights.is_empty()
                    || weights.len() > 8
                    || weights
                        .iter()
                        .any(|weight| !weight.is_finite() || !(-1.0..=1.0).contains(weight))
                {
                    return Err(
                        "morph weights require 1 to 8 finite values in the range [-1, 1]".into(),
                    );
                }
                Ok(())
            }
            Self::SaveCheckpoint { slot, state_json } => {
                validate_save_slot(slot)?;
                if state_json.len() > MAX_SCRIPT_SAVE_STATE_BYTES {
                    return Err(format!(
                        "save state JSON exceeds the {MAX_SCRIPT_SAVE_STATE_BYTES}-byte limit"
                    ));
                }
                serde_json::from_str::<serde_json::Value>(state_json)
                    .map(|_| ())
                    .map_err(|error| format!("save state must be valid JSON: {error}"))
            }
            Self::LoadCheckpoint { slot } => validate_save_slot(slot),
            Self::QueryLogicAsset { asset_id, .. } => validate_entity_id(asset_id),
            Self::RegisterRuntimeMesh { asset_id, mesh, .. } => {
                validate_runtime_asset_id(asset_id)?;
                mesh.validate()
            }
            Self::RegisterRuntimeMaterial {
                asset_id, material, ..
            } => {
                validate_runtime_asset_id(asset_id)?;
                material.validate()
            }
            Self::RegisterRuntimePrefab {
                asset_id, prefab, ..
            } => {
                validate_runtime_asset_id(asset_id)?;
                prefab.validate()
            }
            Self::TerrainApplyBrush {
                terrain_entity_id,
                brush,
                ..
            } => {
                validate_entity_id(terrain_entity_id)?;
                brush.validate()
            }
            Self::Network { network, .. } => network.validate(),
        }
    }
}

fn default_animation_speed() -> f32 {
    1.0
}

fn default_true() -> bool {
    true
}

/// A validated command paired with the entity that owns the script instance.
#[derive(Clone, Debug, PartialEq)]
pub struct OwnedGameplayCommand {
    pub entity_id: String,
    pub command: GameplayCommand,
}

/// A validated physics query paired with the entity that owns the script
/// instance that issued it.
#[derive(Clone, Debug, PartialEq)]
pub struct OwnedGameplayPhysicsQuery {
    pub entity_id: String,
    pub query: GameplayPhysicsQuery,
}

/// A validated physics mutation paired with the entity that owns the script
/// instance that issued it.
#[derive(Clone, Debug, PartialEq)]
pub struct OwnedGameplayPhysicsMutation {
    pub owner_entity_id: String,
    pub mutation: GameplayPhysicsMutation,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OwnedGameplayDamageRequest {
    pub owner_entity_id: String,
    pub target_entity_id: String,
    pub amount: f32,
    pub damage_kind: GameplayDamageKind,
    pub hit_position: Option<[f32; 3]>,
    pub impulse: [f32; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct OwnedGameplayRagdollRequest {
    pub owner_entity_id: String,
    pub target_entity_id: String,
    pub active: bool,
    pub recovery_duration: f32,
    pub impulse: [f32; 3],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GameplaySaveOperation {
    Save { state_json: String },
    Load,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnedGameplaySaveRequest {
    pub owner_entity_id: String,
    pub slot: String,
    pub operation: GameplaySaveOperation,
}
