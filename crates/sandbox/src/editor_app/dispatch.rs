//! The single typed command router for the React editor shell.

use std::collections::BTreeSet;
use std::path::PathBuf;

use engine_editor::asset_browser::AssetKindFilter;
use engine_editor::component_catalog::ComponentCatalog;
use engine_editor::material_editor::ShaderParamType;
use engine_editor::{
    AddComponent, Command, CommandBatch, EntityPasteParent, MoveEntitySibling, RemoveComponent,
    ReplaceComponent, SetComponentEnabled, SetComponentField, SetEntityEnabled, SetEntityName,
    SetEntityParent, SetSceneSettings, SiblingMove,
};
use platform::PlatformEvent;
use serde_json::{json, Value as JsonValue};

use super::protocol::*;
use super::*;

pub(super) struct DispatchMessages {
    pub json_messages: Vec<String>,
}

struct DispatchOutcome {
    result: JsonValue,
    state_changed: bool,
}

impl DispatchOutcome {
    fn accepted(state_changed: bool) -> Self {
        Self {
            result: json!({ "accepted": true }),
            state_changed,
        }
    }

    fn result(result: JsonValue, state_changed: bool) -> Self {
        Self {
            result,
            state_changed,
        }
    }

    fn job(job_id: u64) -> Self {
        Self::result(json!({ "accepted": true, "jobId": job_id }), true)
    }
}

impl EditorApp {
    pub(super) fn dispatch_ipc_json(&mut self, raw: &str) -> DispatchMessages {
        let parsed = serde_json::from_str::<BridgeRequest>(raw);
        let request = match parsed {
            Ok(request) => request,
            Err(error) => {
                let id = serde_json::from_str::<JsonValue>(raw)
                    .ok()
                    .and_then(|value| {
                        value
                            .get("id")
                            .and_then(JsonValue::as_str)
                            .map(str::to_string)
                    })
                    .unwrap_or_default();
                return DispatchMessages {
                    json_messages: vec![self.error_response(
                        id,
                        BridgeError::new(
                            EditorErrorCode::InvalidRequest,
                            format!("Malformed editor IPC JSON: {error}"),
                        ),
                    )],
                };
            }
        };

        let decoded = match EditorRequest::decode(&request) {
            Ok(decoded) => decoded,
            Err(error) => {
                return DispatchMessages {
                    json_messages: vec![self.error_response(request.id, error)],
                };
            }
        };

        if !matches!(decoded, EditorRequest::Ready(_)) {
            if request.protocol.as_deref() != Some(EDITOR_PROTOCOL) {
                return DispatchMessages {
                    json_messages: vec![self.error_response(
                        request.id,
                        BridgeError::new(
                            EditorErrorCode::ProtocolMismatch,
                            format!("Expected protocol '{EDITOR_PROTOCOL}'"),
                        ),
                    )],
                };
            }
            if request.session_id.as_deref() != Some(self.session_id.as_str()) {
                return DispatchMessages {
                    json_messages: vec![self.error_response(
                        request.id,
                        BridgeError::new(
                            EditorErrorCode::ProtocolMismatch,
                            "The editor session changed; bootstrap again",
                        ),
                    )],
                };
            }
            if request_requires_revision(&decoded)
                && request.base_revision != Some(self.editor_revision)
            {
                let mut error = BridgeError::new(
                    EditorErrorCode::StaleRevision,
                    "The editor changed since this command was created",
                );
                error.current_revision = Some(self.editor_revision);
                return DispatchMessages {
                    json_messages: vec![self.error_response(request.id, error)],
                };
            }
        }

        match self.dispatch_editor_request(decoded) {
            Ok(outcome) => {
                if outcome.state_changed {
                    self.editor_revision = self.editor_revision.wrapping_add(1);
                }
                let response = BridgeResponse {
                    protocol: EDITOR_PROTOCOL,
                    id: request.id,
                    session_id: self.session_id.clone(),
                    revision: self.editor_revision,
                    result: Some(outcome.result),
                    error: None,
                };
                let mut json_messages = vec![serialize_message(&response)];
                if outcome.state_changed {
                    self.editor_event_sequence = self.editor_event_sequence.wrapping_add(1);
                    let event = BridgeEvent {
                        protocol: EDITOR_PROTOCOL,
                        session_id: self.session_id.clone(),
                        sequence: self.editor_event_sequence,
                        revision: self.editor_revision,
                        event: PROJECT_CHANGED_EVENT,
                        params: self.editor_snapshot(),
                    };
                    json_messages.push(serialize_message(&event));
                }
                json_messages.extend(self.take_ui_open_panel_events_json());
                DispatchMessages { json_messages }
            }
            Err(error) => {
                let mut json_messages = vec![self.error_response(request.id, error)];
                json_messages.extend(self.take_ui_open_panel_events_json());
                DispatchMessages { json_messages }
            }
        }
    }

    fn error_response(&self, id: String, error: BridgeError) -> String {
        serialize_message(&BridgeResponse::<JsonValue> {
            protocol: EDITOR_PROTOCOL,
            id,
            session_id: self.session_id.clone(),
            revision: self.editor_revision,
            result: None,
            error: Some(error),
        })
    }

    fn dispatch_editor_request(
        &mut self,
        request: EditorRequest,
    ) -> Result<DispatchOutcome, BridgeError> {
        if self.pending_document_action.is_some()
            && matches!(
                &request,
                EditorRequest::OpenDocument(_)
                    | EditorRequest::CreateDocument(_)
                    | EditorRequest::SaveDocumentAs(_)
                    | EditorRequest::DuplicateDocument(_)
                    | EditorRequest::RenameDocument(_)
                    | EditorRequest::DeleteDocument(_)
                    | EditorRequest::SetStartupDocument(_)
            )
        {
            return Err(BridgeError::new(
                EditorErrorCode::Conflict,
                "Resolve or cancel the pending scene document operation before starting another",
            ));
        }
        match request {
            EditorRequest::Ready(params) => {
                if params.protocol_version != EDITOR_PROTOCOL_VERSION {
                    return Err(BridgeError::new(
                        EditorErrorCode::ProtocolMismatch,
                        format!(
                            "React editor protocol {} is incompatible with host protocol {}",
                            params.protocol_version, EDITOR_PROTOCOL_VERSION
                        ),
                    ));
                }
                tracing::info!(
                    client_version = params.client_version.as_deref().unwrap_or("unspecified"),
                    protocol_version = params.protocol_version,
                    "React editor IPC session established"
                );
                Ok(DispatchOutcome::result(
                    serde_json::to_value(self.editor_snapshot()).map_err(internal_error)?,
                    false,
                ))
            }
            EditorRequest::GetSnapshot => Ok(DispatchOutcome::result(
                serde_json::to_value(self.editor_snapshot()).map_err(internal_error)?,
                false,
            )),
            EditorRequest::RequestExit => {
                self.request_editor_exit();
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::SaveDocument => {
                self.require_editing()?;
                self.save_current_scene_document().map_err(io_error)?;
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::OpenDocument(params) => {
                self.require_editing()?;
                self.apply_scene_document_action(SceneDocumentAction::Open(params.scene_id))
                    .map_err(validation_error)?;
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::CreateDocument(params) => {
                self.require_editing()?;
                self.apply_scene_document_action(SceneDocumentAction::Create {
                    scene_id: params.scene_id,
                    folder: PathBuf::from(params.folder),
                })
                .map_err(validation_error)?;
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::SaveDocumentAs(params) => {
                self.require_editing()?;
                self.apply_scene_document_action(SceneDocumentAction::SaveAs(params.scene_id))
                    .map_err(validation_error)?;
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::DuplicateDocument(params) => {
                self.require_editing()?;
                self.apply_scene_document_action(SceneDocumentAction::Duplicate {
                    source_id: params.source_id,
                    new_id: params.new_id,
                })
                .map_err(validation_error)?;
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::RenameDocument(params) => {
                self.require_editing()?;
                self.apply_scene_document_action(SceneDocumentAction::Rename {
                    old_id: params.old_id,
                    new_id: params.new_id,
                })
                .map_err(validation_error)?;
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::DeleteDocument(params) => {
                self.require_editing()?;
                self.apply_scene_document_action(SceneDocumentAction::Delete {
                    scene_id: params.scene_id,
                    replacement_startup: params.replacement_startup,
                })
                .map_err(validation_error)?;
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::SetStartupDocument(params) => {
                self.require_editing()?;
                self.apply_scene_document_action(SceneDocumentAction::SetStartup(params.scene_id))
                    .map_err(validation_error)?;
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::ResolvePendingSwitch(params) => {
                self.require_editing()?;
                self.pending_scene_switch.as_ref().ok_or_else(|| {
                    BridgeError::new(EditorErrorCode::Conflict, "No scene switch is pending")
                })?;
                let pending = self.pending_document_action.clone().ok_or_else(|| {
                    BridgeError::new(
                        EditorErrorCode::Conflict,
                        "The pending scene action was lost; cancel and retry the operation",
                    )
                })?;
                match params.decision {
                    SaveDiscardCancel::Save => {
                        self.save_current_scene_document().map_err(io_error)?;
                        self.apply_scene_document_action_after_confirmation(pending, true)
                            .map_err(validation_error)?;
                    }
                    SaveDiscardCancel::Discard => {
                        self.apply_scene_document_action_after_confirmation(pending, true)
                            .map_err(validation_error)?;
                    }
                    SaveDiscardCancel::Cancel => {
                        self.apply_scene_document_action(SceneDocumentAction::CancelSwitch)
                            .map_err(validation_error)?;
                    }
                }
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::ResolveClose(params) => {
                if !self.close_confirmation_pending {
                    return Err(BridgeError::new(
                        EditorErrorCode::Conflict,
                        "No editor close confirmation is pending",
                    ));
                }
                let action = match params.decision {
                    SaveDiscardCancel::Save => CloseDocumentAction::SaveAndClose,
                    SaveDiscardCancel::Discard => CloseDocumentAction::DiscardAndClose,
                    SaveDiscardCancel::Cancel => CloseDocumentAction::Cancel,
                };
                self.apply_close_document_action(action).map_err(io_error)?;
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::ResolveRecovery(params) => {
                match params.decision {
                    RecoveryDecision::Restore => self.restore_recovery_snapshot(),
                    RecoveryDecision::Discard => self.discard_recovery_snapshot(),
                }
                .map_err(io_error)?;
                Ok(DispatchOutcome::accepted(true))
            }
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
            EditorRequest::SetRuntimeMode(params) => {
                self.set_runtime_mode(params.mode)?;
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::StepRuntime => {
                if self.play_session.mode() != EditorPlayMode::Paused {
                    return Err(BridgeError::new(
                        EditorErrorCode::Conflict,
                        "Runtime stepping requires Play mode to be paused",
                    ));
                }
                self.step_play();
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::SetViewportBounds(params) => {
                if !matches!(params.viewport.as_str(), "scene" | "game") {
                    return Err(BridgeError::new(
                        EditorErrorCode::InvalidRequest,
                        format!("Unknown viewport '{}'", params.viewport),
                    ));
                }
                let next_viewport = if params.viewport == "game" {
                    ViewportTab::Game
                } else {
                    ViewportTab::Scene
                };
                if !params.visible {
                    self.cancel_web_viewport_input();
                    if self.viewport_tab == next_viewport {
                        self.web_viewport_rect = ScreenRect::default();
                    }
                    return Ok(DispatchOutcome::accepted(false));
                }
                if self.viewport_tab != next_viewport {
                    self.cancel_web_viewport_input();
                }
                self.web_viewport_rect = params.rect;
                self.viewport_tab = next_viewport;
                Ok(DispatchOutcome::accepted(false))
            }
            EditorRequest::ViewportInput(params) => {
                self.handle_web_viewport_input(params)?;
                Ok(DispatchOutcome::accepted(false))
            }
            EditorRequest::SetGizmoMode(params) => {
                self.gizmo.mode = match params.mode {
                    GizmoModeDto::Move => GizmoMode::Translate,
                    GizmoModeDto::Rotate => GizmoMode::Rotate,
                    GizmoModeDto::Scale => GizmoMode::Scale,
                };
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::SetGizmoSpace(params) => {
                self.gizmo.space = match params.mode {
                    GizmoSpaceDto::Global => GizmoSpace::Global,
                    GizmoSpaceDto::Local => GizmoSpace::Local,
                };
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::SetSnapping(params) => {
                self.gizmo.snapping = params.enabled;
                self.workspace_preferences.snapping_enabled = params.enabled;
                self.persist_workspace_preferences_if_changed();
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::FrameSelected => {
                self.frame_selected()?;
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::SetCamera(params) => {
                if !camera_params_are_finite(&params) {
                    return Err(validation_error(
                        "Scene camera parameters must contain only finite numbers",
                    ));
                }
                self.scene_view
                    .set_camera_orbit(params.pitch, params.yaw, params.distance);
                self.scene_view.set_target(params.target);
                self.scene_view.set_orthographic(params.orthographic);
                self.scene_view.set_camera_speed(params.speed);
                self.persist_workspace_preferences_if_changed();
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::SetGizmos(params) => {
                self.workspace_preferences.gizmos_visible = params.visible;
                self.persist_workspace_preferences_if_changed();
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::SelectAsset(params) => {
                if !self.asset_browser.select_asset(params.asset_id) {
                    return Err(BridgeError::new(
                        EditorErrorCode::NotFound,
                        "The selected asset is not in the current project catalog",
                    ));
                }
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::SetAssetBrowser(params) => {
                if params.query.is_none()
                    && params.folder.is_none()
                    && params.kind.is_none()
                    && params.page.is_none()
                    && params.view.is_none()
                {
                    return Err(BridgeError::new(
                        EditorErrorCode::InvalidRequest,
                        "Asset browser update must include at least one field",
                    ));
                }
                self.set_asset_browser_state(params)?;
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::RefreshAssets => {
                self.require_editing()?;
                let project = self.project.manifest_path.clone();
                let job_id = self
                    .start_editor_job("Asset reimport", true, move || {
                        super::super::project_cli::cook_project(&project)
                            .map(|_| EditorJobOutput::None)
                    })
                    .map_err(job_conflict)?;
                Ok(DispatchOutcome::job(job_id))
            }
            EditorRequest::RevealProject => {
                reveal_in_file_manager(&self.project.root).map_err(io_error)?;
                Ok(DispatchOutcome::accepted(false))
            }
            EditorRequest::RevealAssetFolder(params) => {
                let relative = PathBuf::from(
                    params
                        .folder
                        .trim()
                        .trim_matches(['/', '\\'])
                        .replace('\\', "/"),
                );
                if relative
                    .components()
                    .any(|component| !matches!(component, std::path::Component::Normal(_)))
                    && !relative.as_os_str().is_empty()
                {
                    return Err(BridgeError::new(
                        EditorErrorCode::ValidationFailed,
                        "Asset folder must be a normalized project-relative path",
                    ));
                }
                let source_root = self
                    .project
                    .asset_source
                    .canonicalize()
                    .map_err(|error| io_error(error.to_string()))?;
                let folder = self
                    .project
                    .asset_source
                    .join(relative)
                    .canonicalize()
                    .map_err(|error| io_error(error.to_string()))?;
                if !folder.starts_with(&source_root) || !folder.is_dir() {
                    return Err(BridgeError::new(
                        EditorErrorCode::ValidationFailed,
                        "Asset folder is outside the project source tree or is not a directory",
                    ));
                }
                reveal_in_file_manager(&folder).map_err(io_error)?;
                Ok(DispatchOutcome::accepted(false))
            }
            EditorRequest::RevealAsset(params) => {
                let path = self.asset_path(&params.asset_id)?;
                reveal_in_file_manager(&path).map_err(io_error)?;
                Ok(DispatchOutcome::accepted(false))
            }
            EditorRequest::OpenAsset(params) => {
                let entry = self
                    .asset_browser
                    .catalog_assets()
                    .iter()
                    .find(|entry| entry.id.id == params.asset_id)
                    .cloned()
                    .ok_or_else(|| not_found("asset", &params.asset_id))?;
                if entry.kind == engine_editor::asset_browser::AssetKind::Material {
                    self.open_material(params.asset_id);
                    Ok(DispatchOutcome::accepted(true))
                } else {
                    reveal_in_file_manager(&self.asset_path(&params.asset_id)?)
                        .map_err(io_error)?;
                    Ok(DispatchOutcome::accepted(false))
                }
            }
            EditorRequest::ImportAsset(params) => {
                self.require_editing()?;
                let project = self.project.manifest_path.clone();
                let imported_id = params.asset_id.clone();
                let job_id = self
                    .start_editor_job("Asset import", true, move || {
                        super::super::project_cli::import_project_asset_from(
                            project,
                            PathBuf::from(params.source),
                            params.asset_id,
                            params.asset_type,
                            PathBuf::from(params.folder),
                        )?;
                        Ok(EditorJobOutput::SelectAsset(imported_id))
                    })
                    .map_err(job_conflict)?;
                Ok(DispatchOutcome::job(job_id))
            }
            EditorRequest::CreateAssetFolder(params) => {
                self.require_editing()?;
                let project = self.project.manifest_path.clone();
                let folder_name = params.folder;
                let folder = PathBuf::from(&folder_name);
                let job_id = self
                    .start_editor_job("Create asset folder", false, move || {
                        super::super::editor_asset_ops::create_asset_folder(&project, &folder)?;
                        Ok(EditorJobOutput::SelectFolder(folder_name))
                    })
                    .map_err(job_conflict)?;
                Ok(DispatchOutcome::job(job_id))
            }
            EditorRequest::RenameAssetFolder(params) => {
                self.require_editing()?;
                let project = self.project.manifest_path.clone();
                let folder = PathBuf::from(params.folder);
                let new_folder_name = params.new_folder;
                let new_folder = PathBuf::from(&new_folder_name);
                let job_id = self
                    .start_editor_job("Rename asset folder", false, move || {
                        super::super::editor_asset_ops::rename_asset_folder(
                            &project,
                            &folder,
                            &new_folder,
                        )?;
                        Ok(EditorJobOutput::SelectFolder(new_folder_name))
                    })
                    .map_err(job_conflict)?;
                Ok(DispatchOutcome::job(job_id))
            }
            EditorRequest::DeleteAssetFolder(params) => {
                self.require_editing()?;
                let project = self.project.manifest_path.clone();
                let folder = PathBuf::from(params.folder);
                let parent_folder = folder
                    .parent()
                    .unwrap_or_else(|| std::path::Path::new(""))
                    .to_string_lossy()
                    .replace('\\', "/");
                let job_id = self
                    .start_editor_job("Delete asset folder", false, move || {
                        super::super::editor_asset_ops::delete_asset_folder(&project, &folder)?;
                        Ok(EditorJobOutput::SelectFolder(parent_folder))
                    })
                    .map_err(job_conflict)?;
                Ok(DispatchOutcome::job(job_id))
            }
            EditorRequest::CreateMaterial(params) => {
                self.require_editing()?;
                let project = self.project.manifest_path.clone();
                let folder = PathBuf::from(params.folder);
                let job_id = self
                    .start_editor_job("Create material", true, move || {
                        super::super::editor_asset_ops::create_material_asset(
                            &project,
                            &folder,
                            &params.name,
                            &super::super::editor_asset_ops::MaterialTemplate::default(),
                        )
                        .map(|mutation| EditorJobOutput::SelectAsset(mutation.asset_id.id))
                    })
                    .map_err(job_conflict)?;
                Ok(DispatchOutcome::job(job_id))
            }
            EditorRequest::CreatePrefab(params) => {
                self.require_editing()?;
                self.create_prefab_from_selection(
                    params.asset_id,
                    PathBuf::from(params.relative_source_path),
                    PathBuf::from(params.manifest_name),
                )
                .map_err(validation_error)?;
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::InstantiatePrefab(params) => {
                self.require_editing()?;
                self.instantiate_prefab_asset(params.asset_id, params.parent_id)
                    .map_err(validation_error)?;
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::UnpackPrefab(params) => {
                self.require_editing()?;
                self.unpack_prefab_instance(
                    params.entity_id,
                    match params.mode {
                        UnpackModeDto::Instance => PrefabUnpackMode::Instance,
                        UnpackModeDto::Completely => PrefabUnpackMode::Completely,
                    },
                )
                .map_err(validation_error)?;
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::DuplicateAsset(params) => {
                self.require_editing()?;
                let project = self.project.manifest_path.clone();
                let asset = AssetId::new(params.asset_id);
                let job_id = self
                    .start_editor_job("Duplicate asset", true, move || {
                        super::super::editor_asset_ops::duplicate_project_asset(&project, &asset)
                            .map(|mutation| EditorJobOutput::SelectAsset(mutation.asset_id.id))
                    })
                    .map_err(job_conflict)?;
                Ok(DispatchOutcome::job(job_id))
            }
            EditorRequest::MoveAsset(params) => {
                self.require_editing()?;
                let project = self.project.manifest_path.clone();
                let asset = AssetId::new(params.asset_id);
                let new_path = PathBuf::from(params.new_source_path);
                let job_id = self
                    .start_editor_job("Move asset", true, move || {
                        super::super::editor_asset_ops::move_project_asset(
                            &project, &asset, &new_path,
                        )
                        .map(|mutation| EditorJobOutput::SelectAsset(mutation.asset_id.id))
                    })
                    .map_err(job_conflict)?;
                Ok(DispatchOutcome::job(job_id))
            }
            EditorRequest::DeleteAsset(params) => {
                self.require_editing()?;
                self.reject_current_scene_asset_reference(&params.asset_id)?;
                let project = self.project.manifest_path.clone();
                let asset = AssetId::new(params.asset_id);
                let job_id = self
                    .start_editor_job("Delete asset", true, move || {
                        super::super::editor_asset_ops::delete_project_asset(&project, &asset)?;
                        Ok(EditorJobOutput::ClearAssetSelection)
                    })
                    .map_err(job_conflict)?;
                Ok(DispatchOutcome::job(job_id))
            }
            EditorRequest::AssignAsset(params) => {
                self.asset_browser
                    .select_asset(Some(AssetId::new(params.asset_id)));
                if let Some(scene) = self.editor_scene.as_mut() {
                    scene.selected_entity = Some(params.entity_id);
                }
                let command = self
                    .editor_scene
                    .as_ref()
                    .and_then(|scene| scene.selected_entity.clone())
                    .and_then(|entity| self.asset_browser.selected_assignment_command(entity))
                    .ok_or_else(|| {
                        BridgeError::new(
                            EditorErrorCode::ValidationFailed,
                            "Select a Mesh or Material and an entity with a Renderable component",
                        )
                    })?;
                self.execute_command(Box::new(command))?;
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::OpenMaterial(params) => {
                self.open_material(params.asset_id);
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::SetMaterialParameter(params) => {
                self.require_editing()?;
                self.set_material_parameter(params)?;
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::SaveMaterial => {
                self.require_editing()?;
                self.material_editor.request_save();
                self.process_material_save();
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::AssignOpenMaterial => {
                self.require_editing()?;
                self.assign_open_material()?;
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::SetAnimation(params) => {
                if params.skeleton.is_none()
                    && params.clip.is_none()
                    && params.playing.is_none()
                    && params.looping.is_none()
                    && params.speed.is_none()
                    && params.time.is_none()
                {
                    return Err(BridgeError::new(
                        EditorErrorCode::InvalidRequest,
                        "Animation update must include at least one field",
                    ));
                }
                self.set_animation_state(params);
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::ReplayTerrainSeed(params) => {
                self.require_editing()?;
                let seed = params.seed.trim().parse::<u64>().map_err(|_| {
                    BridgeError::new(
                        EditorErrorCode::InvalidRequest,
                        "Terrain seed must be a decimal integer in 0..=18446744073709551615",
                    )
                })?;
                let entity_id = self
                    .editor_scene
                    .as_ref()
                    .and_then(|scene| {
                        scene.scene.entities.iter().find_map(|entity| {
                            entity
                                .components
                                .contains_key("engine.terrain_volume")
                                .then(|| entity.persistent_id.clone())
                        })
                    })
                    .ok_or_else(|| not_found("component", "engine.terrain_volume"))?;
                self.execute_command(Box::new(SetComponentField::new(
                    entity_id,
                    "engine.terrain_volume".to_string(),
                    "seed".to_string(),
                    Value::UInt(seed),
                )))?;
                if let Some(game_loop) = self.game_loop.as_mut() {
                    game_loop.terrain_force_regenerate();
                }
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::RegenerateTerrain => {
                self.game_loop
                    .as_mut()
                    .ok_or_else(runtime_unavailable)?
                    .terrain_force_regenerate();
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::RetryTerrain => {
                self.game_loop
                    .as_mut()
                    .ok_or_else(runtime_unavailable)?
                    .terrain
                    .retry_failed();
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::ClearDiagnostics => {
                self.editor_scene
                    .as_mut()
                    .ok_or_else(runtime_unavailable)?
                    .diagnostics
                    .clear();
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::ExportDiagnostics => {
                self.export_diagnostics()?;
                Ok(DispatchOutcome::accepted(false))
            }
            EditorRequest::StartBuild(params) => {
                self.start_build_request(params)?;
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::CancelBuild => {
                self.cancel_editor_build();
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::RunProject => {
                self.request_run_project();
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::CreateProject(params) => {
                let root = PathBuf::from(params.root);
                let job_id = self
                    .start_editor_job("Create project", false, move || {
                        super::super::project_cli::create_project(
                            &root,
                            params.name.as_deref(),
                            params.with_csharp,
                        )?;
                        launch_editor_window(&root)?;
                        Ok(EditorJobOutput::None)
                    })
                    .map_err(job_conflict)?;
                Ok(DispatchOutcome::job(job_id))
            }
            EditorRequest::OpenProject(params) => {
                let pid = launch_editor_window(&PathBuf::from(params.path)).map_err(io_error)?;
                self.build_status = Some(format!("Opened project editor ({pid})."));
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::SaveProjectSettings(params) => {
                self.require_editing()?;
                let mut manifest = self.project.manifest.clone();
                manifest.window.title = params.title;
                manifest.window.width = params.width;
                manifest.window.height = params.height;
                manifest
                    .write_to_root(&self.project.root)
                    .map_err(|error| {
                        io_error(format!("Could not save project settings: {error}"))
                    })?;
                self.reload_project_manifest().map_err(io_error)?;
                self.build_status = Some("Project settings saved.".into());
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::ReplaceInputMap(params) => {
                self.require_editing()?;
                super::super::project_input::validate_input_map(&params.map)
                    .map_err(validation_error)?;
                self.game_loop
                    .as_mut()
                    .ok_or_else(runtime_unavailable)?
                    .input_map = params.map;
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::SaveInputMap => {
                self.require_editing()?;
                let map = &self
                    .game_loop
                    .as_ref()
                    .ok_or_else(runtime_unavailable)?
                    .input_map;
                super::super::project_input::save_project_input_map(&self.project, map)
                    .map_err(io_error)?;
                self.build_status = Some("Input actions saved.".into());
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::CreateScript(params) => {
                self.require_editing()?;
                let path = super::super::project_scripts::create_project_script_in(
                    &self.project,
                    &PathBuf::from(params.folder),
                    &params.class_name,
                )
                .map_err(|error| BridgeError::new(EditorErrorCode::ScriptFailed, error))?;
                self.build_status = Some(format!("Created script {}.", path.display()));
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::RebuildScripts => {
                self.rebuild_and_reload_scripts();
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::AttachScript(params) => {
                if let Some(scene) = self.editor_scene.as_mut() {
                    scene.selected_entity = Some(params.entity_id);
                }
                let command = self
                    .verified_script_add_command(&params.assembly_id, &params.class_name)
                    .map_err(validation_error)?;
                self.execute_command(command)?;
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::PersistLayout(params) => {
                validate_react_layout(&params.serialized_layout)?;
                self.workspace_preferences.react_layout = Some(params.serialized_layout);
                self.persist_workspace_preferences_if_changed();
                Ok(DispatchOutcome::accepted(false))
            }
        }
    }

    fn require_editing(&self) -> Result<(), BridgeError> {
        if !self.play_session.is_editing() {
            return Err(BridgeError::new(
                EditorErrorCode::EditingRequired,
                "Stop Play mode before editing authoring data",
            ));
        }
        if self.background_job.is_some() || self.editor_build_task.is_some() {
            return Err(BridgeError::new(
                EditorErrorCode::Conflict,
                "Wait for the active project operation before editing authoring data",
            ));
        }
        Ok(())
    }

    fn reject_current_scene_asset_reference(&self, asset_id: &str) -> Result<(), BridgeError> {
        let Some(scene) = self.editor_scene.as_ref().map(|editor| &editor.scene) else {
            return Ok(());
        };
        let referenced = scene
            .collect_asset_dependencies()
            .iter()
            .chain(scene.dependencies.iter())
            .any(|dependency| dependency.id == asset_id)
            || scene
                .scene_settings
                .environment_map
                .as_ref()
                .is_some_and(|dependency| dependency.id == asset_id);
        if referenced {
            Err(BridgeError::new(
                EditorErrorCode::Conflict,
                format!(
                    "Asset '{asset_id}' is referenced by the open authoring scene; remove the reference before deleting it"
                ),
            ))
        } else {
            Ok(())
        }
    }

    fn set_runtime_mode(&mut self, requested: RuntimeModeDto) -> Result<(), BridgeError> {
        let current = self.play_session.mode();
        if requested == RuntimeModeDto::Play
            && current == EditorPlayMode::Editing
            && self.pending_document_action.is_some()
        {
            return Err(BridgeError::new(
                EditorErrorCode::Conflict,
                "Resolve or cancel the pending scene document operation before entering Play mode",
            ));
        }
        let expected = match (requested, current) {
            (RuntimeModeDto::Edit, EditorPlayMode::Editing)
            | (RuntimeModeDto::Play, EditorPlayMode::Playing)
            | (RuntimeModeDto::Paused, EditorPlayMode::Paused) => {
                return Err(BridgeError::new(
                    EditorErrorCode::Conflict,
                    "The runtime is already in the requested mode",
                ));
            }
            (RuntimeModeDto::Edit, _) => {
                self.stop_play();
                EditorPlayMode::Editing
            }
            (RuntimeModeDto::Play, EditorPlayMode::Paused) => {
                self.resume_play();
                EditorPlayMode::Playing
            }
            (RuntimeModeDto::Play, EditorPlayMode::Editing) => {
                self.start_play();
                EditorPlayMode::Playing
            }
            (RuntimeModeDto::Paused, EditorPlayMode::Playing) => {
                self.pause_play();
                EditorPlayMode::Paused
            }
            (RuntimeModeDto::Paused, EditorPlayMode::Editing) => {
                return Err(BridgeError::new(
                    EditorErrorCode::Conflict,
                    "Start Play mode before pausing the runtime",
                ));
            }
        };
        if self.play_session.mode() != expected {
            return Err(BridgeError::new(
                EditorErrorCode::RuntimeUnavailable,
                "The requested runtime transition failed; review Console diagnostics",
            ));
        }
        Ok(())
    }

    fn execute_command(&mut self, command: Box<dyn Command>) -> Result<(), BridgeError> {
        self.require_editing()?;
        if self.execute_editor_command(command) {
            Ok(())
        } else {
            Err(BridgeError::new(
                EditorErrorCode::ValidationFailed,
                "The editor command was rejected; see Console for details",
            ))
        }
    }

    fn undo_or_redo(&mut self, undo: bool) -> Result<(), BridgeError> {
        self.require_editing()?;
        let (game_loop, editor_scene) = match (self.game_loop.as_mut(), self.editor_scene.as_mut())
        {
            (Some(game_loop), Some(editor_scene)) => (game_loop, editor_scene),
            _ => return Err(runtime_unavailable()),
        };
        if undo {
            editor_scene.undo()
        } else {
            editor_scene.redo()
        }
        .map_err(|error| validation_error(error.to_string()))?;
        let existing = editor_scene
            .scene
            .entities
            .iter()
            .map(|entity| entity.persistent_id.clone())
            .collect::<BTreeSet<_>>();
        self.selected_entity_ids
            .retain(|entity_id| existing.contains(entity_id));
        if editor_scene
            .selected_entity
            .as_ref()
            .is_none_or(|entity_id| !existing.contains(entity_id))
        {
            editor_scene.selected_entity = self.selected_entity_ids.first().cloned();
        }
        synchronize_authoring_view(game_loop, editor_scene, &self.scene_view, self.viewport_tab);
        self.scene_settings_draft = editor_scene.scene.scene_settings.clone();
        Ok(())
    }

    fn select_entities(
        &mut self,
        requested: Vec<String>,
        active: Option<String>,
    ) -> Result<(), BridgeError> {
        let mut seen = BTreeSet::new();
        let mut selection = requested
            .into_iter()
            .filter(|entity_id| !entity_id.is_empty() && seen.insert(entity_id.clone()))
            .collect::<Vec<_>>();
        if let Some(active) = active.as_ref() {
            if !active.is_empty() && seen.insert(active.clone()) {
                selection.push(active.clone());
            }
        }
        let active = active
            .filter(|entity_id| !entity_id.is_empty())
            .or_else(|| selection.first().cloned());
        for entity_id in &selection {
            self.entity(entity_id)?;
        }
        if let Some(active) = active.as_ref() {
            self.entity(active)?;
        }
        self.selected_entity_ids = selection;
        if let Some(scene) = self.editor_scene.as_mut() {
            scene.selected_entity = active;
        }
        let material = self.editor_scene.as_ref().and_then(|scene| {
            selected_material_asset(&scene.scene, scene.selected_entity.as_ref())
        });
        if let Some(material) = material {
            self.open_material(material);
        }
        Ok(())
    }

    fn param_or_selected(&self, entity_id: &str) -> Result<String, BridgeError> {
        if !entity_id.is_empty() {
            return Ok(entity_id.to_string());
        }
        self.editor_scene
            .as_ref()
            .and_then(|scene| scene.selected_entity.clone())
            .ok_or_else(selection_error)
    }

    fn command_entity_ids(
        &self,
        primary: &str,
        requested: &[String],
    ) -> Result<Vec<String>, BridgeError> {
        let source = if !requested.is_empty() {
            requested.to_vec()
        } else if !primary.is_empty() {
            vec![primary.to_string()]
        } else if !self.selected_entity_ids.is_empty() {
            self.selected_entity_ids.clone()
        } else {
            vec![self.param_or_selected("")?]
        };
        let mut seen = BTreeSet::new();
        let ids = source
            .into_iter()
            .filter(|entity_id| !entity_id.is_empty() && seen.insert(entity_id.clone()))
            .collect::<Vec<_>>();
        if ids.is_empty() {
            return Err(selection_error());
        }
        for entity_id in &ids {
            self.entity(entity_id)?;
        }
        Ok(ids)
    }

    fn root_entity_ids(&self, requested: &[String]) -> Result<Vec<String>, BridgeError> {
        let scene = self.editor_scene.as_ref().ok_or_else(runtime_unavailable)?;
        let clipboard = engine_editor::EntityClipboard::capture(&scene.scene, requested)
            .map_err(|error| validation_error(error.to_string()))?;
        Ok(clipboard.root_ids().to_vec())
    }

    fn clear_removed_selection(&mut self, removed_roots: &[String]) {
        let removed = removed_roots.iter().cloned().collect::<BTreeSet<_>>();
        let existing = self
            .editor_scene
            .as_ref()
            .map(|scene| {
                scene
                    .scene
                    .entities
                    .iter()
                    .map(|entity| entity.persistent_id.clone())
                    .collect::<BTreeSet<_>>()
            })
            .unwrap_or_default();
        self.selected_entity_ids
            .retain(|entity_id| !removed.contains(entity_id) && existing.contains(entity_id));
        if let Some(scene) = self.editor_scene.as_mut() {
            if scene
                .selected_entity
                .as_ref()
                .is_none_or(|entity_id| !existing.contains(entity_id))
            {
                scene.selected_entity = self.selected_entity_ids.first().cloned();
            }
        }
    }

    fn capture_entities(&mut self, requested: &[String]) -> Result<(), BridgeError> {
        let roots = self.command_entity_ids("", requested)?;
        let scene = self.editor_scene.as_ref().ok_or_else(runtime_unavailable)?;
        self.entity_clipboard = Some(
            engine_editor::EntityClipboard::capture(&scene.scene, &roots)
                .map_err(|error| validation_error(error.to_string()))?,
        );
        Ok(())
    }

    fn add_component_command(
        &self,
        entity_id: &str,
        component_type: &str,
    ) -> Result<Box<dyn Command>, BridgeError> {
        let descriptor = ComponentCatalog::descriptor(component_type)
            .ok_or_else(|| not_found("component", component_type))?;
        let entity = self.entity(entity_id)?.clone();
        if entity.components.contains_key(descriptor.type_id) {
            return Err(BridgeError::new(
                EditorErrorCode::Conflict,
                format!("{} is already attached", descriptor.display_name),
            ));
        }
        let mut projected = entity.clone();
        let mut commands: Vec<Box<dyn Command>> = Vec::new();
        for type_id in descriptor
            .required_components
            .iter()
            .copied()
            .chain(std::iter::once(descriptor.type_id))
        {
            if projected.components.contains_key(type_id) {
                continue;
            }
            let component = ComponentCatalog::create_component(type_id, &projected)
                .map_err(|error| validation_error(error.to_string()))?;
            projected
                .components
                .insert(type_id.to_string(), component.clone());
            commands.push(Box::new(AddComponent::new(
                entity_id.to_string(),
                type_id.to_string(),
                component,
            )));
        }
        Ok(Box::new(CommandBatch::new(
            format!("Add {}", descriptor.display_name),
            commands,
        )))
    }

    fn reset_component_command(
        &self,
        entity_id: &str,
        component_type: &str,
    ) -> Result<Box<dyn Command>, BridgeError> {
        let descriptor = ComponentCatalog::descriptor(component_type)
            .ok_or_else(|| not_found("component", component_type))?;
        let mut projected = self.entity(entity_id)?.clone();
        projected.components.remove(component_type);
        let replacement = ComponentCatalog::create_component(component_type, &projected)
            .map_err(|error| validation_error(error.to_string()))?;
        Ok(Box::new(ReplaceComponent::new(
            entity_id.to_string(),
            descriptor.type_id.to_string(),
            replacement,
        )))
    }

    fn entity(&self, entity_id: &str) -> Result<&EntityRecord, BridgeError> {
        self.editor_scene
            .as_ref()
            .and_then(|scene| {
                scene
                    .scene
                    .entities
                    .iter()
                    .find(|entity| entity.persistent_id == entity_id)
            })
            .ok_or_else(|| not_found("entity", entity_id))
    }

    fn frame_selected(&mut self) -> Result<(), BridgeError> {
        let target = self
            .editor_scene
            .as_ref()
            .and_then(|scene| {
                let selected = scene.selected_entity.as_ref()?;
                let entity = scene
                    .scene
                    .entities
                    .iter()
                    .find(|entity| &entity.persistent_id == selected)?;
                match entity
                    .components
                    .get("engine.transform")?
                    .fields
                    .get("translation")?
                {
                    Value::Vec3(value) => Some(*value),
                    _ => None,
                }
            })
            .ok_or_else(selection_error)?;
        let (pitch, yaw, _) = self.scene_view.camera_orbit();
        self.scene_view.set_target(target);
        self.scene_view.set_camera_orbit(pitch, yaw, 6.0);
        Ok(())
    }

    fn set_asset_browser_state(&mut self, params: AssetBrowserParams) -> Result<(), BridgeError> {
        if let Some(query) = params.query {
            self.asset_browser.set_search_query(query);
        }
        if let Some(folder) = params.folder {
            self.asset_browser.set_current_folder(folder);
        }
        if let Some(kind) = params.kind {
            let filter = match kind.to_ascii_lowercase().as_str() {
                "all" => AssetKindFilter::All,
                "mesh" | "model" => AssetKindFilter::Mesh,
                "texture" => AssetKindFilter::Texture,
                "shader" => AssetKindFilter::Shader,
                "scene" => AssetKindFilter::Scene,
                "material" => AssetKindFilter::Material,
                "pipeline" => AssetKindFilter::Pipeline,
                "script" => AssetKindFilter::Script,
                "audio" => AssetKindFilter::Audio,
                "font" => AssetKindFilter::Font,
                "animation" => AssetKindFilter::Animation,
                "skeleton" => AssetKindFilter::Skeleton,
                "navmesh" => AssetKindFilter::NavMesh,
                "logic" => AssetKindFilter::Logic,
                "prefab" => AssetKindFilter::Prefab,
                "unknown" | "other" => AssetKindFilter::Unknown,
                _ => {
                    return Err(BridgeError::new(
                        EditorErrorCode::InvalidRequest,
                        format!("Unknown asset kind filter '{kind}'"),
                    ));
                }
            };
            self.asset_browser.set_kind_filter(filter);
        }
        if let Some(page) = params.page {
            while self.asset_browser.page() < page && self.asset_browser.next_page() {}
            while self.asset_browser.page() > page && self.asset_browser.previous_page() {}
        }
        if let Some(view) = params.view {
            self.workspace_preferences.project_asset_view = match view.as_str() {
                "grid" => ProjectAssetView::Grid,
                "list" => ProjectAssetView::List,
                _ => {
                    return Err(BridgeError::new(
                        EditorErrorCode::InvalidRequest,
                        format!("Unknown asset browser view '{view}'"),
                    ));
                }
            };
        }
        self.workspace_preferences.project_asset_folder =
            self.asset_browser.current_folder().to_string();
        Ok(())
    }

    fn asset_path(&self, asset_id: &str) -> Result<PathBuf, BridgeError> {
        let entry = self
            .asset_browser
            .catalog_assets()
            .iter()
            .find(|entry| entry.id.id == asset_id)
            .ok_or_else(|| not_found("asset", asset_id))?;
        if let Some(source) = entry.source_path.as_deref() {
            return Ok(self.project.asset_source.join(source));
        }
        Ok(self.project.root.clone())
    }

    fn open_material(&mut self, material: String) {
        if let Some(game_loop) = self.game_loop.as_ref() {
            load_material(
                &mut self.material_editor,
                &material,
                game_loop.runtime.asset_registry(),
            );
        }
        self.material_editor
            .set_save_access(project_material_save_access(&self.project, &material));
        self.material_editor_selection = Some(material);
        self.request_ui_open_panel(UiPanel::Material, UiDockZone::Bottom);
    }

    fn set_material_parameter(
        &mut self,
        params: MaterialParameterParams,
    ) -> Result<(), BridgeError> {
        let parameter = self
            .material_editor
            .shader_params
            .iter_mut()
            .find(|parameter| parameter.name == params.name)
            .ok_or_else(|| not_found("material parameter", &params.name))?;
        match parameter.param_type {
            ShaderParamType::Float => {
                parameter.float_value = serde_json::from_value(params.value)
                    .map_err(|error| validation_error(error.to_string()))?;
            }
            ShaderParamType::Color => {
                parameter.color_value = serde_json::from_value(params.value)
                    .map_err(|error| validation_error(error.to_string()))?;
            }
            ShaderParamType::Texture => {
                parameter.texture_value = serde_json::from_value(params.value)
                    .map_err(|error| validation_error(error.to_string()))?;
            }
        }
        Ok(())
    }

    fn assign_open_material(&mut self) -> Result<(), BridgeError> {
        let material = self
            .material_editor
            .selected_material
            .clone()
            .ok_or_else(|| not_found("open material", "selection"))?;
        let loaded = self.game_loop.as_ref().is_some_and(|game_loop| {
            game_loop
                .runtime
                .asset_registry()
                .get::<engine_renderer::MaterialUpload>(&AssetId::new(&material))
                .is_some()
        });
        if !loaded {
            return Err(BridgeError::new(
                EditorErrorCode::ValidationFailed,
                format!("Material '{material}' is not loaded; reimport it before assignment"),
            ));
        }
        let editor_scene = self.editor_scene.as_ref().ok_or_else(runtime_unavailable)?;
        let command = assign_material_to_selected_command(editor_scene, &material)
            .map_err(validation_error)?;
        self.execute_command(command)
    }

    fn set_animation_state(&mut self, params: AnimationParams) {
        if let Some(skeleton) = params.skeleton {
            self.animation_preview.selected_skeleton = skeleton;
        }
        if let Some(clip) = params.clip {
            self.animation_preview.selected_clip = clip;
        }
        if let Some(playing) = params.playing {
            self.animation_preview.playing = playing;
        }
        if let Some(looping) = params.looping {
            self.animation_preview.looping = looping;
        }
        if let Some(speed) = params.speed {
            self.animation_preview.speed = speed.clamp(0.05, 4.0);
        }
        if let Some(time) = params.time {
            let duration = self
                .animation_preview
                .clip_info()
                .map_or(f32::MAX, |info| info.duration);
            self.animation_preview.playback_time = time.clamp(0.0, duration);
        }
    }

    fn export_diagnostics(&mut self) -> Result<(), BridgeError> {
        let output = self.project.root.join(".engine/logs/editor-console.txt");
        let contents = self
            .editor_scene
            .as_ref()
            .map(|scene| {
                scene
                    .diagnostics
                    .all_entries()
                    .iter()
                    .map(|entry| {
                        format!(
                            "{:?} [{}] {}: {}",
                            entry.diagnostic.severity,
                            entry.diagnostic.code,
                            entry.diagnostic.system,
                            entry.diagnostic.message
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        atomic_write_file(&output, contents.as_bytes()).map_err(io_error)?;
        self.build_status = Some(format!("Console exported to {}.", output.display()));
        Ok(())
    }

    fn start_build_request(&mut self, params: BuildParams) -> Result<(), BridgeError> {
        let operation = params
            .operation
            .unwrap_or_else(|| match params.target_id.as_deref() {
                Some("validate") => BuildOperation::Validate,
                Some("windows-x64") if params.version.is_some() => BuildOperation::PackageWindows,
                _ => BuildOperation::CookAndCompile,
            });
        self.run_after_build = params.run_after_build;
        match operation {
            BuildOperation::Validate => self
                .start_editor_build(super::super::editor_build_ops::EditorBuildOperation::Validate),
            BuildOperation::CookAndCompile => self.start_editor_build(
                super::super::editor_build_ops::EditorBuildOperation::CookAndCompile,
            ),
            BuildOperation::PackageWindows => {
                let version = params
                    .version
                    .unwrap_or_else(|| self.package_version.clone());
                let output = params
                    .output_root
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from(&self.package_output_root));
                self.start_editor_build(
                    super::super::editor_build_ops::EditorBuildOperation::PackageWindows(
                        super::super::editor_build_ops::PackageWindowsOptions::new(version, output),
                    ),
                );
            }
        }
        Ok(())
    }

    fn handle_web_viewport_input(
        &mut self,
        params: ViewportInputParams,
    ) -> Result<(), BridgeError> {
        if !matches!(params.viewport.as_str(), "scene" | "game") {
            return Err(BridgeError::new(
                EditorErrorCode::InvalidRequest,
                format!("Unknown viewport '{}'", params.viewport),
            ));
        }
        let next_viewport = if params.viewport == "game" {
            ViewportTab::Game
        } else {
            ViewportTab::Scene
        };
        if self.viewport_tab != next_viewport {
            self.cancel_web_viewport_input();
            self.viewport_tab = next_viewport;
        }
        match params.event {
            ViewportInput::PointerDown {
                pointer_id,
                x,
                y,
                button,
                buttons,
                modifiers,
            } => {
                let pointer = self.web_pointer_to_physical(x, y);
                self.web_viewport_input.pointer_id = Some(pointer_id);
                self.web_viewport_input.pointer = Some(pointer);
                self.web_viewport_input.buttons = buttons;
                self.web_viewport_input.modifiers = modifiers;
                if button == 0
                    && self.viewport_tab == ViewportTab::Scene
                    && self.play_session.is_editing()
                {
                    self.gizmo_pointer_events
                        .push(GizmoPointerEvent::Press(pointer));
                }
                #[cfg(feature = "runtime-subsystems")]
                if button == 0 && self.viewport_tab == ViewportTab::Game {
                    let ui_pointer = self.web_viewport_pointer_to_physical(x, y);
                    if let Some(game_loop) = self.game_loop.as_mut() {
                        game_loop.ui_pointer_move(ui_pointer.x, ui_pointer.y);
                        game_loop.ui_pointer_left_press();
                    }
                }
            }
            ViewportInput::PointerUp {
                pointer_id,
                x,
                y,
                button,
                buttons,
                modifiers,
            } => {
                if self
                    .web_viewport_input
                    .pointer_id
                    .is_some_and(|active| active != pointer_id)
                {
                    return Ok(());
                }
                let pointer = self.web_pointer_to_physical(x, y);
                self.web_viewport_input.pointer = Some(pointer);
                self.web_viewport_input.buttons = buttons;
                self.web_viewport_input.modifiers = modifiers;
                if buttons == 0 {
                    self.web_viewport_input.pointer_id = None;
                }
                if button == 0
                    && self.viewport_tab == ViewportTab::Scene
                    && self.play_session.is_editing()
                {
                    self.gizmo_pointer_events
                        .push(GizmoPointerEvent::Release(pointer));
                }
                #[cfg(feature = "runtime-subsystems")]
                if button == 0 && self.viewport_tab == ViewportTab::Game {
                    let ui_pointer = self.web_viewport_pointer_to_physical(x, y);
                    if let Some(game_loop) = self.game_loop.as_mut() {
                        game_loop.ui_pointer_move(ui_pointer.x, ui_pointer.y);
                        game_loop.ui_pointer_left_release();
                    }
                }
            }
            ViewportInput::PointerMove {
                pointer_id,
                x,
                y,
                button,
                buttons,
                modifiers,
            } => {
                if self
                    .web_viewport_input
                    .pointer_id
                    .is_some_and(|active| active != pointer_id)
                {
                    return Ok(());
                }
                let pointer = self.web_pointer_to_physical(x, y);
                if let Some(previous) = self.web_viewport_input.pointer {
                    let scale = self.window_scale_factor as f32;
                    let delta = (pointer - previous) / scale.max(f32::EPSILON);
                    let (pitch, yaw, distance) = self.scene_view.camera_orbit();
                    let orbit_sensitivity = if modifiers.shift { 0.05 } else { 0.2 };
                    if buttons & 2 != 0 && self.viewport_tab == ViewportTab::Scene {
                        self.scene_view.set_camera_orbit(
                            pitch + delta.y * orbit_sensitivity,
                            yaw + delta.x * orbit_sensitivity,
                            distance,
                        );
                    }
                    if buttons & 4 != 0 && self.viewport_tab == ViewportTab::Scene {
                        let yaw_radians = yaw.to_radians();
                        let right = Vec3::new(-yaw_radians.sin(), 0.0, yaw_radians.cos());
                        let pan_scale = distance.max(0.1) * 0.0025;
                        let target = Vec3::from_array(*self.scene_view.target())
                            - right * delta.x * pan_scale
                            + Vec3::Y * delta.y * pan_scale;
                        self.scene_view.set_target(target.to_array());
                    }
                }
                self.web_viewport_input.pointer = Some(pointer);
                self.web_viewport_input.buttons = buttons;
                self.web_viewport_input.modifiers = modifiers;
                if (buttons & 1 != 0 || button == 0)
                    && self.viewport_tab == ViewportTab::Scene
                    && self.play_session.is_editing()
                {
                    self.gizmo_pointer_events
                        .push(GizmoPointerEvent::Move(pointer));
                }
                #[cfg(feature = "runtime-subsystems")]
                if self.viewport_tab == ViewportTab::Game {
                    let ui_pointer = self.web_viewport_pointer_to_physical(x, y);
                    if let Some(game_loop) = self.game_loop.as_mut() {
                        game_loop.ui_pointer_move(ui_pointer.x, ui_pointer.y);
                    }
                }
            }
            ViewportInput::PointerCancel { pointer_id } => {
                if self
                    .web_viewport_input
                    .pointer_id
                    .is_none_or(|active| active == pointer_id)
                {
                    self.cancel_web_viewport_input();
                }
            }
            ViewportInput::Wheel {
                x,
                y,
                delta_x,
                delta_y,
                delta_mode,
                modifiers,
            } => {
                self.web_viewport_input.modifiers = modifiers;
                if self.viewport_tab == ViewportTab::Scene {
                    let (pitch, yaw, distance) = self.scene_view.camera_orbit();
                    let unit_scale = match delta_mode {
                        0 => 1.0,
                        1 => 16.0,
                        _ => self.web_viewport_rect.height.max(1.0),
                    };
                    self.scene_view.set_camera_orbit(
                        pitch,
                        yaw + delta_x * unit_scale * 0.01,
                        distance * (-delta_y * unit_scale * 0.0025).exp(),
                    );
                }
                #[cfg(feature = "runtime-subsystems")]
                if self.viewport_tab == ViewportTab::Game {
                    let ui_pointer = self.web_viewport_pointer_to_physical(x, y);
                    if let Some(game_loop) = self.game_loop.as_mut() {
                        game_loop.ui_pointer_move(ui_pointer.x, ui_pointer.y);
                    }
                }
            }
            ViewportInput::KeyDown {
                key,
                code,
                repeat,
                modifiers,
            } => {
                if !repeat {
                    self.web_viewport_input.keys.insert(code.clone());
                }
                self.web_viewport_input.modifiers = modifiers;
                if key == "Escape" {
                    self.gizmo_pointer_events.push(GizmoPointerEvent::Cancel);
                }
                self.route_web_key(&code, true, modifiers);
            }
            ViewportInput::KeyUp {
                key,
                code,
                repeat,
                modifiers,
            } => {
                self.web_viewport_input.keys.remove(&code);
                self.web_viewport_input.modifiers = modifiers;
                if !repeat || key == "Escape" {
                    self.route_web_key(&code, false, modifiers);
                }
            }
            ViewportInput::Focus => self.web_viewport_input.focused = true,
            ViewportInput::Blur => self.cancel_web_viewport_input(),
        }
        Ok(())
    }

    fn web_pointer_to_physical(&self, x: f32, y: f32) -> Vec2 {
        css_pointer_to_physical(
            self.web_viewport_rect.x + x,
            self.web_viewport_rect.y + y,
            self.window_scale_factor,
        )
    }

    fn web_viewport_pointer_to_physical(&self, x: f32, y: f32) -> Vec2 {
        css_pointer_to_physical(x, y, self.window_scale_factor)
    }

    pub(super) fn cancel_web_viewport_input(&mut self) {
        self.web_viewport_input.pointer_id = None;
        self.web_viewport_input.pointer = None;
        self.web_viewport_input.buttons = 0;
        self.web_viewport_input.modifiers = InputModifiers::default();
        self.web_viewport_input.keys.clear();
        self.web_viewport_input.focused = false;
        self.gizmo_pointer_events.push(GizmoPointerEvent::Cancel);
        #[cfg(feature = "target-desktop")]
        if let Some(game_loop) = self.game_loop.as_mut() {
            self.input_state.reset(&mut game_loop.input_map);
        }
        #[cfg(feature = "runtime-subsystems")]
        if let Some(game_loop) = self.game_loop.as_mut() {
            game_loop.cancel_ui_pointer();
        }
    }

    fn route_web_key(&mut self, code: &str, pressed: bool, modifiers: InputModifiers) {
        if self.play_session.is_editing() {
            return;
        }
        let Some(key) = web_key_code(code) else {
            return;
        };
        #[cfg(feature = "target-desktop")]
        if let Some(game_loop) = self.game_loop.as_mut() {
            self.input_state.apply_platform_event(
                &mut game_loop.input_map,
                &if pressed {
                    PlatformEvent::KeyPressed {
                        key,
                        modifiers: web_modifiers(modifiers),
                    }
                } else {
                    PlatformEvent::KeyReleased {
                        key,
                        modifiers: web_modifiers(modifiers),
                    }
                },
            );
        }
    }

    pub(super) fn tick_web_viewport_camera(&mut self, delta_seconds: f32) {
        if self.viewport_tab != ViewportTab::Scene || self.web_viewport_input.buttons & 2 == 0 {
            return;
        }
        let mut movement = Vec3::ZERO;
        let keys = &self.web_viewport_input.keys;
        movement.z += if keys.contains("KeyW") { 1.0 } else { 0.0 };
        movement.z -= if keys.contains("KeyS") { 1.0 } else { 0.0 };
        movement.x += if keys.contains("KeyD") { 1.0 } else { 0.0 };
        movement.x -= if keys.contains("KeyA") { 1.0 } else { 0.0 };
        movement.y += if keys.contains("KeyE") { 1.0 } else { 0.0 };
        movement.y -= if keys.contains("KeyQ") { 1.0 } else { 0.0 };
        if movement.length_squared() <= f32::EPSILON {
            return;
        }
        let (_, yaw, _) = self.scene_view.camera_orbit();
        let yaw_radians = yaw.to_radians();
        let forward = Vec3::new(-yaw_radians.cos(), 0.0, -yaw_radians.sin());
        let right = Vec3::new(-yaw_radians.sin(), 0.0, yaw_radians.cos());
        let speed_multiplier = if self.web_viewport_input.modifiers.shift {
            3.0
        } else if self.web_viewport_input.modifiers.control {
            0.25
        } else {
            1.0
        };
        let world_delta = (right * movement.x + Vec3::Y * movement.y + forward * movement.z)
            .normalize_or_zero()
            * self.scene_view.camera_speed()
            * speed_multiplier
            * delta_seconds.min(0.1);
        let target = Vec3::from_array(*self.scene_view.target()) + world_delta;
        self.scene_view.set_target(target.to_array());
    }
}

fn request_requires_revision(request: &EditorRequest) -> bool {
    !matches!(
        request,
        EditorRequest::Ready(_)
            | EditorRequest::GetSnapshot
            | EditorRequest::SetViewportBounds(_)
            | EditorRequest::ViewportInput(_)
            | EditorRequest::PersistLayout(_)
    )
}

fn serialize_message(value: &impl serde::Serialize) -> String {
    serde_json::to_string(value).unwrap_or_else(|error| {
        format!(
            "{{\"error\":{{\"code\":\"internal\",\"message\":{}}}}}",
            serde_json::to_string(&error.to_string())
                .unwrap_or_else(|_| "\"serialization failed\"".into())
        )
    })
}

fn internal_error(error: serde_json::Error) -> BridgeError {
    BridgeError::new(EditorErrorCode::Internal, error.to_string())
}

fn validation_error(error: impl Into<String>) -> BridgeError {
    BridgeError::new(EditorErrorCode::ValidationFailed, error)
}

fn validate_react_layout(serialized: &str) -> Result<(), BridgeError> {
    const MAX_LAYOUT_BYTES: usize = 128 * 1024;
    const ZONES: [&str; 4] = ["left", "center", "right", "bottom"];
    const PANELS: [&str; 12] = [
        "hierarchy",
        "scene",
        "game",
        "inspector",
        "project",
        "console",
        "material",
        "animation",
        "profiler",
        "terrain",
        "build",
        "settings",
    ];
    if serialized.len() > MAX_LAYOUT_BYTES {
        return Err(validation_error("React layout exceeds 128 KiB"));
    }
    let layout: JsonValue = serde_json::from_str(serialized)
        .map_err(|error| validation_error(format!("React layout is not valid JSON: {error}")))?;
    let root = layout
        .as_object()
        .ok_or_else(|| validation_error("React layout must be a JSON object"))?;
    let zones = root
        .get("zones")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| validation_error("React layout must define dock zones"))?;
    for zone_id in ZONES {
        let zone = zones
            .get(zone_id)
            .and_then(JsonValue::as_object)
            .ok_or_else(|| validation_error(format!("React layout is missing '{zone_id}'")))?;
        let panels = zone
            .get("panels")
            .and_then(JsonValue::as_array)
            .ok_or_else(|| {
                validation_error(format!("React layout zone '{zone_id}' has no panel list"))
            })?;
        if !panels
            .iter()
            .all(|panel| panel.as_str().is_some_and(|panel| PANELS.contains(&panel)))
        {
            return Err(validation_error(format!(
                "React layout zone '{zone_id}' contains an unknown panel"
            )));
        }
        let active = zone
            .get("active")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| {
                validation_error(format!("React layout zone '{zone_id}' has no active panel"))
            })?;
        if !PANELS.contains(&active) {
            return Err(validation_error(format!(
                "React layout zone '{zone_id}' has an unknown active panel"
            )));
        }
        if zone.get("collapsed").and_then(JsonValue::as_bool).is_none() {
            return Err(validation_error(format!(
                "React layout zone '{zone_id}' has no collapsed state"
            )));
        }
    }
    for dimension in ["leftWidth", "rightWidth", "bottomHeight"] {
        if root.get(dimension).and_then(JsonValue::as_f64).is_none() {
            return Err(validation_error(format!(
                "React layout is missing numeric '{dimension}'"
            )));
        }
    }
    Ok(())
}

fn camera_params_are_finite(params: &CameraParams) -> bool {
    params.pitch.is_finite()
        && params.yaw.is_finite()
        && params.distance.is_finite()
        && params.target.iter().all(|value| value.is_finite())
        && params.speed.is_finite()
}

fn io_error(error: impl Into<String>) -> BridgeError {
    BridgeError::new(EditorErrorCode::IoFailed, error)
}

fn runtime_unavailable() -> BridgeError {
    BridgeError::new(
        EditorErrorCode::RuntimeUnavailable,
        "The editor runtime is not initialized",
    )
}

fn selection_error() -> BridgeError {
    BridgeError::new(EditorErrorCode::SelectionRequired, "Select an entity first")
}

fn job_conflict(message: String) -> BridgeError {
    BridgeError::new(EditorErrorCode::Conflict, message)
}

fn not_found(kind: &str, value: &str) -> BridgeError {
    BridgeError::new(
        EditorErrorCode::NotFound,
        format!("{kind} '{value}' was not found"),
    )
}

fn allocate_entity_id(app: &EditorApp) -> String {
    let existing = app
        .editor_scene
        .as_ref()
        .map(|scene| scene.scene.entities.as_slice());
    for sequence in 1_u64.. {
        let candidate = format!("entity-{sequence:04}");
        if existing.is_none_or(|entities| {
            entities
                .iter()
                .all(|entity| entity.persistent_id != candidate)
        }) {
            return candidate;
        }
    }
    unreachable!("u64 entity IDs cannot be exhausted")
}

fn web_key_code(code: &str) -> Option<platform::KeyCode> {
    use platform::KeyCode;
    Some(match code {
        "Escape" => KeyCode::Escape,
        "Space" => KeyCode::Space,
        "Enter" | "NumpadEnter" => KeyCode::Enter,
        "Backspace" => KeyCode::Backspace,
        "Tab" => KeyCode::Tab,
        "Delete" => KeyCode::Delete,
        "ArrowLeft" => KeyCode::Left,
        "ArrowRight" => KeyCode::Right,
        "ArrowUp" => KeyCode::Up,
        "ArrowDown" => KeyCode::Down,
        "KeyA" => KeyCode::A,
        "KeyB" => KeyCode::B,
        "KeyC" => KeyCode::C,
        "KeyD" => KeyCode::D,
        "KeyE" => KeyCode::E,
        "KeyF" => KeyCode::F,
        "KeyG" => KeyCode::G,
        "KeyH" => KeyCode::H,
        "KeyI" => KeyCode::I,
        "KeyJ" => KeyCode::J,
        "KeyK" => KeyCode::K,
        "KeyL" => KeyCode::L,
        "KeyM" => KeyCode::M,
        "KeyN" => KeyCode::N,
        "KeyO" => KeyCode::O,
        "KeyP" => KeyCode::P,
        "KeyQ" => KeyCode::Q,
        "KeyR" => KeyCode::R,
        "KeyS" => KeyCode::S,
        "KeyT" => KeyCode::T,
        "KeyU" => KeyCode::U,
        "KeyV" => KeyCode::V,
        "KeyW" => KeyCode::W,
        "KeyX" => KeyCode::X,
        "KeyY" => KeyCode::Y,
        "KeyZ" => KeyCode::Z,
        "Digit0" => KeyCode::Key0,
        "Digit1" => KeyCode::Key1,
        "Digit2" => KeyCode::Key2,
        "Digit3" => KeyCode::Key3,
        "Digit4" => KeyCode::Key4,
        "Digit5" => KeyCode::Key5,
        "Digit6" => KeyCode::Key6,
        "Digit7" => KeyCode::Key7,
        "Digit8" => KeyCode::Key8,
        "Digit9" => KeyCode::Key9,
        "F1" => KeyCode::F1,
        "F2" => KeyCode::F2,
        "F3" => KeyCode::F3,
        "F4" => KeyCode::F4,
        "F5" => KeyCode::F5,
        "F6" => KeyCode::F6,
        "F7" => KeyCode::F7,
        "F8" => KeyCode::F8,
        "F9" => KeyCode::F9,
        "F10" => KeyCode::F10,
        "F11" => KeyCode::F11,
        "F12" => KeyCode::F12,
        _ => return None,
    })
}

fn web_modifiers(modifiers: InputModifiers) -> platform::Modifiers {
    platform::Modifiers {
        ctrl: modifiers.control,
        shift: modifiers.shift,
        alt: modifiers.alt,
        logo: modifiers.meta,
    }
}

fn css_pointer_to_physical(x: f32, y: f32, scale_factor: f64) -> Vec2 {
    let scale = if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor as f32
    } else {
        1.0
    };
    Vec2::new(x * scale, y * scale)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_key_codes_map_to_the_existing_platform_contract() {
        assert_eq!(web_key_code("KeyW"), Some(platform::KeyCode::W));
        assert_eq!(web_key_code("ArrowLeft"), Some(platform::KeyCode::Left));
        assert_eq!(web_key_code("BrowserBack"), None);
    }

    #[test]
    fn viewport_traffic_does_not_require_scene_revision_round_trips() {
        assert!(!request_requires_revision(
            &EditorRequest::SetViewportBounds(ViewportBoundsParams {
                viewport: "scene".into(),
                rect: ScreenRect::default(),
                visible: true,
            })
        ));
        assert!(request_requires_revision(&EditorRequest::Undo));
    }

    #[test]
    fn entity_id_params_deserialize_the_canonical_batch_ids() {
        let params: EntityIdParams = serde_json::from_value(json!({
            "entityIds": ["root", "child"]
        }))
        .unwrap();

        assert!(params.entity_id.is_empty());
        assert_eq!(params.entity_ids, ["root", "child"]);

        for method in ["scene.duplicateEntity", "scene.deleteEntity"] {
            let request = BridgeRequest {
                id: "batch-entity-request".into(),
                protocol: Some(EDITOR_PROTOCOL.into()),
                session_id: Some("session".into()),
                base_revision: Some(0),
                method: method.into(),
                params: json!({ "entityIds": ["root", "child"] }),
            };
            let decoded = EditorRequest::decode(&request).unwrap();
            let decoded_params = match decoded {
                EditorRequest::DuplicateEntity(params) | EditorRequest::DeleteEntity(params) => {
                    params
                }
                _ => panic!("{method} decoded to the wrong request variant"),
            };
            assert!(decoded_params.entity_id.is_empty());
            assert_eq!(decoded_params.entity_ids, ["root", "child"]);
        }
    }

    #[test]
    fn css_viewport_coordinates_scale_to_physical_pixels_on_hidpi_surfaces() {
        assert_eq!(
            css_pointer_to_physical(120.0, 80.0, 1.5),
            Vec2::new(180.0, 120.0)
        );
        assert_eq!(
            css_pointer_to_physical(120.0, 80.0, 2.0),
            Vec2::new(240.0, 160.0)
        );
    }

    #[test]
    fn scene_camera_command_rejects_non_finite_state() {
        let valid = CameraParams {
            pitch: 20.0,
            yaw: 45.0,
            distance: 10.0,
            target: [0.0, 1.0, 2.0],
            orthographic: false,
            speed: 5.0,
        };
        assert!(camera_params_are_finite(&valid));
        assert!(!camera_params_are_finite(&CameraParams {
            target: [f32::NAN, 0.0, 0.0],
            ..valid
        }));
    }

    #[test]
    fn persisted_react_layout_requires_the_canonical_dock_shape() {
        assert!(validate_react_layout(DEFAULT_REACT_LAYOUT).is_ok());
        assert!(validate_react_layout(r#"{"zones":{}}"#).is_err());
        assert!(validate_react_layout("not json").is_err());
    }
}
