use crate::*;

use super::world::*;

impl EngineRuntime {
    pub(crate) fn take_pending_terrain_brushes(
        &mut self,
    ) -> Vec<engine_script::OwnedGameplayTerrainBrushRequest> {
        std::mem::take(&mut self.scripting.pending_terrain_brushes)
    }

    pub(crate) fn push_runtime_asset_result(
        &mut self,
        owner_entity_id: String,
        result: engine_script::GameplayRuntimeAssetResult,
    ) {
        self.scripting
            .runtime_asset_results
            .entry(owner_entity_id)
            .or_default()
            .push(result);
    }

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
            GameplayCommand::RegisterRuntimeMesh {
                request_id,
                asset_id,
                mesh,
            } => {
                if !script_command_owner_exists(&self.world_slot, &entity_id) {
                    diagnostics.push(script_owner_missing_diagnostic(
                        &entity_id,
                        "register a runtime mesh",
                    ));
                    return;
                }
                let result = mesh.validate().and_then(|()| {
                    let descriptor = crate::RuntimeMeshDescriptor {
                        positions: mesh
                            .positions
                            .into_iter()
                            .map(glam::Vec3::from_array)
                            .collect(),
                        normals: mesh
                            .normals
                            .into_iter()
                            .map(glam::Vec3::from_array)
                            .collect(),
                        uvs: mesh.uvs.into_iter().map(glam::Vec2::from_array).collect(),
                        indices: mesh.indices,
                        bounds: None,
                    };
                    self.create_runtime_mesh(&asset_id, descriptor)
                        .map_err(|error| error.to_string())
                        .and_then(|handle| {
                            self.runtime_mesh_asset_id(handle)
                                .map(|id| id.id)
                                .ok_or_else(|| "runtime mesh registration lost its asset ID".into())
                        })
                });
                self.record_runtime_asset_result(entity_id, request_id, asset_id, result);
            }
            GameplayCommand::RegisterRuntimeMaterial {
                request_id,
                asset_id,
                material,
            } => {
                if !script_command_owner_exists(&self.world_slot, &entity_id) {
                    diagnostics.push(script_owner_missing_diagnostic(
                        &entity_id,
                        "register a runtime material",
                    ));
                    return;
                }
                let result = material.validate().and_then(|()| {
                    let id = AssetId::new(asset_id.clone());
                    if self.asset_registry.contains(&id) {
                        return Err(format!(
                            "runtime material '{asset_id}' conflicts with an existing asset"
                        ));
                    }
                    let encoded =
                        serde_json::to_vec(&material).map_err(|error| error.to_string())?;
                    let upload = engine_renderer::MaterialUpload {
                        material_id: id,
                        base_color: material.base_color,
                        metallic: material.metallic,
                        roughness: material.roughness,
                        ambient_occlusion: material.ambient_occlusion,
                        emissive: material.emissive,
                        base_color_texture: material.base_color_texture.map(AssetId::new),
                        normal_texture: material.normal_texture.map(AssetId::new),
                        metallic_roughness_texture: material
                            .metallic_roughness_texture
                            .map(AssetId::new),
                        occlusion_texture: material.occlusion_texture.map(AssetId::new),
                        emissive_texture: material.emissive_texture.map(AssetId::new),
                        advanced: engine_renderer::AdvancedMaterialParameters::default(),
                        transparency: if material.blend {
                            engine_renderer::Transparency::Blend
                        } else if let Some(cutoff) = material.alpha_cutoff {
                            engine_renderer::Transparency::Masked { cutoff }
                        } else {
                            engine_renderer::Transparency::Opaque
                        },
                        double_sided: material.double_sided,
                        content_hash: engine_asset::compute_content_hash(&[encoded.as_slice()]),
                    };
                    self.register_material_asset(upload);
                    Ok(asset_id.clone())
                });
                self.record_runtime_asset_result(entity_id, request_id, asset_id, result);
            }
            GameplayCommand::RegisterRuntimePrefab {
                request_id,
                asset_id,
                prefab,
            } => {
                if !script_command_owner_exists(&self.world_slot, &entity_id) {
                    diagnostics.push(script_owner_missing_diagnostic(
                        &entity_id,
                        "register a runtime prefab",
                    ));
                    return;
                }
                let result = self.register_script_runtime_prefab(&asset_id, prefab);
                self.record_runtime_asset_result(entity_id, request_id, asset_id, result);
            }
            GameplayCommand::TerrainApplyBrush {
                request_id,
                terrain_entity_id,
                brush,
            } => {
                if !script_command_owner_exists(&self.world_slot, &entity_id) {
                    diagnostics.push(script_owner_missing_diagnostic(&entity_id, "edit terrain"));
                    return;
                }
                if let Err(error) = brush.validate() {
                    self.record_runtime_asset_result(
                        entity_id,
                        request_id,
                        terrain_entity_id,
                        Err(error),
                    );
                    return;
                }
                self.scripting.pending_terrain_brushes.push(
                    engine_script::OwnedGameplayTerrainBrushRequest {
                        owner_entity_id: entity_id,
                        request_id,
                        terrain_entity_id,
                        brush,
                    },
                );
            }
            _ => unreachable!("non-extended gameplay command reached extended dispatcher"),
        }
    }

    fn record_runtime_asset_result(
        &mut self,
        owner_entity_id: String,
        request_id: u32,
        requested_asset_id: String,
        result: Result<String, String>,
    ) {
        let (asset_id, success, error) = match result {
            Ok(asset_id) => (asset_id, true, None),
            Err(error) => (requested_asset_id, false, Some(error)),
        };
        self.push_runtime_asset_result(
            owner_entity_id,
            engine_script::GameplayRuntimeAssetResult {
                request_id,
                asset_id,
                success,
                error,
            },
        );
    }

    fn register_script_runtime_prefab(
        &mut self,
        asset_id: &str,
        descriptor: engine_script::GameplayRuntimePrefab,
    ) -> Result<String, String> {
        descriptor.validate()?;
        let id = AssetId::new(asset_id.to_string());
        if self.asset_registry.contains(&id) {
            return Err(format!(
                "runtime prefab '{asset_id}' conflicts with an existing asset"
            ));
        }
        let mut prefab = engine_scene::Prefab::new(id.clone());
        prefab.hierarchy = descriptor
            .entities
            .into_iter()
            .map(|entity| engine_scene::EntityRecord {
                persistent_id: entity.entity_id,
                parent: entity.parent,
                name: entity.name,
                enabled: entity.enabled,
                components: entity
                    .components
                    .into_iter()
                    .map(|(component_type, fields)| {
                        (
                            component_type,
                            engine_scene::ComponentRecord {
                                schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
                                enabled: true,
                                fields: fields
                                    .into_iter()
                                    .map(|(name, value)| (name, value.to_scene_value()))
                                    .collect(),
                            },
                        )
                    })
                    .collect(),
            })
            .collect();
        engine_scene::validate_prefab_structure(&prefab).map_err(|errors| {
            errors
                .into_iter()
                .map(|error| error.to_string())
                .collect::<Vec<_>>()
                .join("; ")
        })?;
        self.asset_registry.insert_typed(id.clone(), prefab);
        self.loaded_extension_asset_ids
            .entry("prefab".into())
            .or_default()
            .insert(id);
        Ok(asset_id.to_string())
    }
}
