use crate::*;

use super::world::*;

impl EngineRuntime {
    pub(super) fn apply_script_extended_command(
        &mut self,
        entity_id: String,
        command: GameplayCommand,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        match command {
            GameplayCommand::PlayAnimation {
                entity_id: target_id,
                clip_asset,
                looping,
                speed,
                restart,
            } => {
                if !script_command_owner_exists(&self.world_slot, &entity_id) {
                    diagnostics.push(script_owner_missing_diagnostic(
                        &entity_id,
                        "play an animation",
                    ));
                    return;
                }
                let command = GameplayCommand::PlayAnimation {
                    entity_id: target_id.clone(),
                    clip_asset: clip_asset.clone(),
                    looping,
                    speed,
                    restart,
                };
                if let Err(reason) = command.validate() {
                    diagnostics.push(script_component_diagnostic(
                            "SCRIPT_ANIMATION_INVALID",
                            &entity_id,
                            format!(
                                "script entity '{entity_id}' produced an invalid animation request: {reason}"
                            ),
                        ));
                    return;
                }
                apply_script_animation_command(
                    &self.world_slot,
                    &entity_id,
                    &target_id,
                    ScriptAnimationCommand::PlayClip {
                        clip_asset,
                        looping,
                        speed,
                        restart,
                    },
                    diagnostics,
                );
            }
            GameplayCommand::SetAnimationParameter {
                entity_id: target_id,
                name,
                value,
            } => {
                if !script_command_owner_exists(&self.world_slot, &entity_id) {
                    diagnostics.push(script_owner_missing_diagnostic(
                        &entity_id,
                        "set an animation parameter",
                    ));
                    return;
                }
                let command = GameplayCommand::SetAnimationParameter {
                    entity_id: target_id.clone(),
                    name: name.clone(),
                    value: value.clone(),
                };
                if let Err(reason) = command.validate() {
                    diagnostics.push(script_component_diagnostic(
                            "SCRIPT_ANIMATION_INVALID",
                            &entity_id,
                            format!(
                                "script entity '{entity_id}' produced an invalid animation parameter: {reason}"
                            ),
                        ));
                    return;
                }
                apply_script_animation_command(
                    &self.world_slot,
                    &entity_id,
                    &target_id,
                    ScriptAnimationCommand::SetParameter { name, value },
                    diagnostics,
                );
            }
            GameplayCommand::TransitionAnimationState {
                entity_id: target_id,
                state,
            } => {
                if !script_command_owner_exists(&self.world_slot, &entity_id) {
                    diagnostics.push(script_owner_missing_diagnostic(
                        &entity_id,
                        "transition an animation state",
                    ));
                    return;
                }
                let command = GameplayCommand::TransitionAnimationState {
                    entity_id: target_id.clone(),
                    state: state.clone(),
                };
                if let Err(reason) = command.validate() {
                    diagnostics.push(script_component_diagnostic(
                            "SCRIPT_ANIMATION_INVALID",
                            &entity_id,
                            format!(
                                "script entity '{entity_id}' produced an invalid animation transition: {reason}"
                            ),
                        ));
                    return;
                }
                apply_script_animation_command(
                    &self.world_slot,
                    &entity_id,
                    &target_id,
                    ScriptAnimationCommand::Transition { state },
                    diagnostics,
                );
            }
            GameplayCommand::SetAnimationPlaying {
                entity_id: target_id,
                playing,
            } => {
                if !script_command_owner_exists(&self.world_slot, &entity_id) {
                    diagnostics.push(script_owner_missing_diagnostic(
                        &entity_id,
                        "change animation playback",
                    ));
                    return;
                }
                if let Err(reason) = (GameplayCommand::SetAnimationPlaying {
                    entity_id: target_id.clone(),
                    playing,
                })
                .validate()
                {
                    diagnostics.push(script_component_diagnostic(
                            "SCRIPT_ANIMATION_INVALID",
                            &entity_id,
                            format!(
                                "script entity '{entity_id}' produced an invalid animation request: {reason}"
                            ),
                        ));
                    return;
                }
                apply_script_animation_command(
                    &self.world_slot,
                    &entity_id,
                    &target_id,
                    ScriptAnimationCommand::SetPlaying { playing },
                    diagnostics,
                );
            }
            GameplayCommand::SetMorphWeights {
                entity_id: target_id,
                weights,
            } => {
                if !script_command_owner_exists(&self.world_slot, &entity_id) {
                    diagnostics.push(script_owner_missing_diagnostic(
                        &entity_id,
                        "change morph weights",
                    ));
                    return;
                }
                let command = GameplayCommand::SetMorphWeights {
                    entity_id: target_id.clone(),
                    weights: weights.clone(),
                };
                if let Err(reason) = command.validate() {
                    diagnostics.push(script_component_diagnostic(
                        "SCRIPT_ANIMATION_INVALID",
                        &entity_id,
                        format!(
                            "script entity '{entity_id}' produced invalid morph weights: {reason}"
                        ),
                    ));
                    return;
                }
                apply_script_morph_weights(
                    &self.world_slot,
                    &entity_id,
                    &target_id,
                    weights,
                    diagnostics,
                );
            }
            GameplayCommand::SaveCheckpoint { slot, state_json } => {
                if !script_command_owner_exists(&self.world_slot, &entity_id) {
                    diagnostics.push(script_owner_missing_diagnostic(
                        &entity_id,
                        "save a checkpoint",
                    ));
                    return;
                }
                let command = GameplayCommand::SaveCheckpoint {
                    slot: slot.clone(),
                    state_json: state_json.clone(),
                };
                if let Err(reason) = command.validate() {
                    diagnostics.push(script_component_diagnostic(
                            "SCRIPT_SAVE_INVALID",
                            &entity_id,
                            format!(
                                "script entity '{entity_id}' produced an invalid save request: {reason}"
                            ),
                        ));
                    return;
                }
                if self.scripting.pending_save_requests.len()
                    >= engine_script::MAX_PENDING_SAVE_REQUESTS
                {
                    diagnostics.push(script_component_diagnostic(
                            "SCRIPT_SAVE_OVERFLOW",
                            &entity_id,
                            format!(
                                "script entity '{entity_id}' exceeded the pending save request budget of {} per frame",
                                engine_script::MAX_PENDING_SAVE_REQUESTS
                            ),
                        ));
                    return;
                }
                self.scripting.pending_save_requests.push(
                    engine_script::OwnedGameplaySaveRequest {
                        owner_entity_id: entity_id,
                        slot,
                        operation: engine_script::GameplaySaveOperation::Save { state_json },
                    },
                );
            }
            GameplayCommand::LoadCheckpoint { slot } => {
                if !script_command_owner_exists(&self.world_slot, &entity_id) {
                    diagnostics.push(script_owner_missing_diagnostic(
                        &entity_id,
                        "load a checkpoint",
                    ));
                    return;
                }
                if let Err(reason) = engine_script::validate_save_slot(&slot) {
                    diagnostics.push(script_component_diagnostic(
                            "SCRIPT_SAVE_INVALID",
                            &entity_id,
                            format!(
                                "script entity '{entity_id}' produced an invalid load request: {reason}"
                            ),
                        ));
                    return;
                }
                if self.scripting.pending_save_requests.len()
                    >= engine_script::MAX_PENDING_SAVE_REQUESTS
                {
                    diagnostics.push(script_component_diagnostic(
                            "SCRIPT_SAVE_OVERFLOW",
                            &entity_id,
                            format!(
                                "script entity '{entity_id}' exceeded the pending save request budget of {} per frame",
                                engine_script::MAX_PENDING_SAVE_REQUESTS
                            ),
                        ));
                    return;
                }
                self.scripting.pending_save_requests.push(
                    engine_script::OwnedGameplaySaveRequest {
                        owner_entity_id: entity_id,
                        slot,
                        operation: engine_script::GameplaySaveOperation::Load,
                    },
                );
            }
            GameplayCommand::QueryLogicAsset { query_id, asset_id } => {
                if !script_command_owner_exists(&self.world_slot, &entity_id) {
                    diagnostics.push(script_owner_missing_diagnostic(
                        &entity_id,
                        "query a logic asset",
                    ));
                    return;
                }
                let query_count = self
                    .scripting
                    .logic_asset_results
                    .values()
                    .map(Vec::len)
                    .sum::<usize>();
                if query_count >= engine_script::MAX_PENDING_LOGIC_ASSET_QUERIES {
                    diagnostics.push(script_component_diagnostic(
                            "SCRIPT_LOGIC_ASSET_QUERY_OVERFLOW",
                            &entity_id,
                            format!(
                                "script entity '{entity_id}' exceeded the logic asset query budget of {} per frame",
                                engine_script::MAX_PENDING_LOGIC_ASSET_QUERIES
                            ),
                        ));
                    return;
                }
                let result = if let Err(error) = engine_script::validate_entity_id(&asset_id) {
                    engine_script::GameplayLogicAssetResult {
                        query_id,
                        asset_id,
                        json: None,
                        error: Some(error),
                    }
                } else {
                    let id = AssetId::new(asset_id.clone());
                    match self
                        .asset_registry
                        .get::<engine_asset::cook::LogicAsset>(&id)
                    {
                        Some(asset) => match serde_json::to_string(asset.get()) {
                            Ok(json)
                                if json.len()
                                    <= engine_script::MAX_SCRIPT_LOGIC_ASSET_JSON_BYTES =>
                            {
                                engine_script::GameplayLogicAssetResult {
                                    query_id,
                                    asset_id,
                                    json: Some(json),
                                    error: None,
                                }
                            }
                            Ok(_) => engine_script::GameplayLogicAssetResult {
                                query_id,
                                asset_id,
                                json: None,
                                error: Some(format!(
                                    "logic asset JSON exceeds the {}-byte script limit",
                                    engine_script::MAX_SCRIPT_LOGIC_ASSET_JSON_BYTES
                                )),
                            },
                            Err(error) => engine_script::GameplayLogicAssetResult {
                                query_id,
                                asset_id,
                                json: None,
                                error: Some(format!(
                                    "logic asset could not be serialized: {error}"
                                )),
                            },
                        },
                        None => engine_script::GameplayLogicAssetResult {
                            query_id,
                            asset_id,
                            json: None,
                            error: Some("logic asset is not loaded".into()),
                        },
                    }
                };
                self.scripting
                    .logic_asset_results
                    .entry(entity_id)
                    .or_default()
                    .push(result);
            }
            _ => unreachable!("non-extended gameplay command reached extended dispatcher"),
        }
    }
}
