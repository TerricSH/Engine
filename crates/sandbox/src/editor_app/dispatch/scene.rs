//! Scene hierarchy, entity, component, and settings request handling.

use super::*;

impl EditorApp {
    pub(super) fn dispatch_scene_request(
        &mut self,
        request: EditorRequest,
    ) -> Result<DispatchOutcome, BridgeError> {
        match request {
            EditorRequest::Undo => {
                if !self
                    .editor_scene
                    .as_ref()
                    .is_some_and(|scene| scene.history.can_undo())
                {
                    return Ok(DispatchOutcome::accepted(false));
                }
                self.undo_or_redo(true)?;
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::Redo => {
                if !self
                    .editor_scene
                    .as_ref()
                    .is_some_and(|scene| scene.history.can_redo())
                {
                    return Ok(DispatchOutcome::accepted(false));
                }
                self.undo_or_redo(false)?;
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::SelectEntity(params) => {
                self.select_entities(params.entity_ids, params.entity_id)?;
                Ok(DispatchOutcome::result(
                    serde_json::to_value(&self.editor_snapshot().selection)
                        .map_err(internal_error)?,
                    true,
                ))
            }
            EditorRequest::CreateEntity(params) => {
                self.require_editing()?;
                let entity_id = params.entity_id.unwrap_or_else(|| allocate_entity_id(self));
                let entity = ComponentCatalog::instantiate_template(
                    params.template_id.as_deref().unwrap_or("empty"),
                    entity_id.clone(),
                    params.parent_id,
                )
                .map_err(|error| validation_error(error.to_string()))?;
                self.execute_command(Box::new(engine_editor::AddEntity::new(entity)))?;
                if let Some(scene) = self.editor_scene.as_mut() {
                    scene.selected_entity = Some(entity_id.clone());
                }
                self.selected_entity_ids = vec![entity_id.clone()];
                Ok(DispatchOutcome::result(
                    json!({ "entityId": entity_id }),
                    true,
                ))
            }
            EditorRequest::SetEntityEnabled(params) => {
                let ids = self.command_entity_ids(&params.entity_id, &params.entity_ids)?;
                let commands: Vec<Box<dyn Command>> = ids
                    .into_iter()
                    .filter(|entity_id| {
                        self.entity(entity_id)
                            .is_ok_and(|entity| entity.enabled != params.enabled)
                    })
                    .map(|entity_id| {
                        Box::new(SetEntityEnabled::new(entity_id, params.enabled))
                            as Box<dyn Command>
                    })
                    .collect();
                if commands.is_empty() {
                    return Ok(DispatchOutcome::accepted(false));
                }
                self.execute_command(Box::new(CommandBatch::new("Set Entity Enabled", commands)))?;
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::SetEntityName(params) => {
                let name = params.name.and_then(|name| {
                    let trimmed = name.trim();
                    (!trimmed.is_empty()).then(|| trimmed.to_string())
                });
                if name.is_none() {
                    return Err(BridgeError::new(
                        EditorErrorCode::ValidationFailed,
                        "GameObject name cannot be empty",
                    ));
                }
                if self.entity(&params.entity_id)?.name == name {
                    return Ok(DispatchOutcome::accepted(false));
                }
                self.execute_command(Box::new(SetEntityName::new(params.entity_id, name)))?;
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::SetEntityParent(params) => {
                let ids = self.command_entity_ids(&params.entity_id, &params.entity_ids)?;
                let roots = self.root_entity_ids(&ids)?;
                let commands = roots
                    .into_iter()
                    .map(|entity_id| {
                        Box::new(SetEntityParent::new(entity_id, params.parent.clone()))
                            as Box<dyn Command>
                    })
                    .collect();
                self.execute_command(Box::new(CommandBatch::new("Set Entity Parent", commands)))?;
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::MoveEntity(params) => {
                self.execute_command(Box::new(MoveEntitySibling::new(
                    params.entity_id,
                    match params.movement {
                        SiblingMoveDto::Up => SiblingMove::Up,
                        SiblingMoveDto::Down => SiblingMove::Down,
                        SiblingMoveDto::First => SiblingMove::First,
                        SiblingMoveDto::Last => SiblingMove::Last,
                    },
                )))?;
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::CopyEntities(params) => {
                self.capture_entities(&params.entity_ids)?;
                Ok(DispatchOutcome::accepted(false))
            }
            EditorRequest::CutEntities(params) => {
                let ids = self.command_entity_ids("", &params.entity_ids)?;
                self.capture_entities(&ids)?;
                let roots = self.root_entity_ids(&ids)?;
                let commands = roots
                    .iter()
                    .cloned()
                    .map(|entity_id| {
                        Box::new(engine_editor::RemoveEntity::new(entity_id)) as Box<dyn Command>
                    })
                    .collect();
                self.execute_command(Box::new(CommandBatch::new("Cut Entities", commands)))?;
                self.clear_removed_selection(&roots);
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::PasteEntities(params) => {
                let parent = params
                    .parent_id
                    .map(EntityPasteParent::Entity)
                    .unwrap_or(EntityPasteParent::SceneRoot);
                self.paste_entity_clipboard(parent)
                    .map_err(validation_error)?;
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::DuplicateEntity(params) => {
                let ids = self.command_entity_ids(&params.entity_id, &params.entity_ids)?;
                self.duplicate_entities(&ids).map_err(validation_error)?;
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::DeleteEntity(params) => {
                let ids = self.command_entity_ids(&params.entity_id, &params.entity_ids)?;
                let roots = self.root_entity_ids(&ids)?;
                let commands = roots
                    .iter()
                    .cloned()
                    .map(|entity_id| {
                        Box::new(engine_editor::RemoveEntity::new(entity_id)) as Box<dyn Command>
                    })
                    .collect();
                self.execute_command(Box::new(CommandBatch::new("Delete Entities", commands)))?;
                self.clear_removed_selection(&roots);
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::SetComponentEnabled(params) => {
                let ids = self.command_entity_ids(&params.entity_id, &params.entity_ids)?;
                let commands: Vec<Box<dyn Command>> = ids
                    .into_iter()
                    .filter(|entity_id| {
                        self.entity(entity_id)
                            .ok()
                            .and_then(|entity| entity.components.get(&params.component_type))
                            .is_some_and(|component| component.enabled != params.enabled)
                    })
                    .map(|entity_id| {
                        Box::new(SetComponentEnabled::new(
                            entity_id,
                            params.component_type.clone(),
                            params.enabled,
                        )) as Box<dyn Command>
                    })
                    .collect();
                if commands.is_empty() {
                    return Ok(DispatchOutcome::accepted(false));
                }
                self.execute_command(Box::new(CommandBatch::new(
                    "Set Component Enabled",
                    commands,
                )))?;
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::SetComponentField(params) => {
                let ids = self.command_entity_ids(&params.entity_id, &params.entity_ids)?;
                let commands: Vec<Box<dyn Command>> = ids
                    .into_iter()
                    .filter(|entity_id| {
                        self.entity(entity_id)
                            .ok()
                            .and_then(|entity| entity.components.get(&params.component_type))
                            .and_then(|component| component.fields.get(&params.field_name))
                            != Some(&params.value)
                    })
                    .map(|entity_id| {
                        Box::new(SetComponentField::new(
                            entity_id,
                            params.component_type.clone(),
                            params.field_name.clone(),
                            params.value.clone(),
                        )) as Box<dyn Command>
                    })
                    .collect();
                if commands.is_empty() {
                    return Ok(DispatchOutcome::accepted(false));
                }
                self.execute_command(Box::new(CommandBatch::new("Set Component Field", commands)))?;
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::AddComponent(params) => {
                let ids = self.command_entity_ids(&params.entity_id, &params.entity_ids)?;
                let commands = ids
                    .iter()
                    .filter_map(|entity_id| match self.entity(entity_id) {
                        Ok(entity) if entity.components.contains_key(&params.component_type) => {
                            None
                        }
                        Ok(_) => {
                            Some(self.add_component_command(entity_id, &params.component_type))
                        }
                        Err(error) => Some(Err(error)),
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                if commands.is_empty() {
                    return Err(BridgeError::new(
                        EditorErrorCode::Conflict,
                        format!(
                            "{} is already attached to every selected entity",
                            params.component_type
                        ),
                    ));
                }
                self.execute_command(Box::new(CommandBatch::new("Add Component", commands)))?;
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::ResetComponent(params) => {
                let ids = self.command_entity_ids(&params.entity_id, &params.entity_ids)?;
                let commands = ids
                    .iter()
                    .map(|entity_id| {
                        self.reset_component_command(entity_id, &params.component_type)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                self.execute_command(Box::new(CommandBatch::new("Reset Component", commands)))?;
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::RemoveComponent(params) => {
                let ids = self.command_entity_ids(&params.entity_id, &params.entity_ids)?;
                if ComponentCatalog::descriptor(&params.component_type)
                    .is_some_and(|descriptor| !descriptor.removable)
                {
                    return Err(BridgeError::new(
                        EditorErrorCode::ValidationFailed,
                        format!(
                            "{} is required and cannot be removed",
                            ComponentCatalog::descriptor(&params.component_type)
                                .map_or(params.component_type.as_str(), |descriptor| {
                                    descriptor.display_name
                                })
                        ),
                    ));
                }
                for entity_id in &ids {
                    let entity = self.entity(entity_id)?;
                    if !entity.components.contains_key(&params.component_type) {
                        return Err(not_found("component", &params.component_type));
                    }
                    if let Some(dependent) = entity.components.keys().find_map(|type_id| {
                        let descriptor = ComponentCatalog::descriptor(type_id)?;
                        descriptor
                            .required_components
                            .contains(&params.component_type.as_str())
                            .then_some(descriptor.display_name)
                    }) {
                        return Err(BridgeError::new(
                            EditorErrorCode::ValidationFailed,
                            format!(
                                "Cannot remove {} because {dependent} requires it",
                                params.component_type
                            ),
                        ));
                    }
                }
                let commands = ids
                    .into_iter()
                    .map(|entity_id| {
                        Box::new(RemoveComponent::new(
                            entity_id,
                            params.component_type.clone(),
                        )) as Box<dyn Command>
                    })
                    .collect();
                self.execute_command(Box::new(CommandBatch::new("Remove Component", commands)))?;
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::CopyComponent(params) => {
                let entity_id = self
                    .command_entity_ids(&params.entity_id, &params.entity_ids)?
                    .into_iter()
                    .next()
                    .ok_or_else(selection_error)?;
                self.copy_component_to_clipboard(&entity_id, &params.component_type)
                    .map_err(validation_error)?;
                Ok(DispatchOutcome::accepted(false))
            }
            EditorRequest::PasteComponent(params) => {
                let ids = self.command_entity_ids(&params.entity_id, &params.entity_ids)?;
                self.paste_component_to_entities(ids, params.component_type)
                    .map_err(validation_error)?;
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::ApplySceneSettings(params) => {
                let command = SetSceneSettings::prepare(
                    &self
                        .editor_scene
                        .as_ref()
                        .ok_or_else(runtime_unavailable)?
                        .scene,
                    params.settings.clone(),
                );
                self.execute_command(Box::new(command))?;
                self.scene_settings_draft = params.settings;
                Ok(DispatchOutcome::accepted(true))
            }
            _ => unreachable!("request routed to the wrong editor IPC domain"),
        }
    }
}
