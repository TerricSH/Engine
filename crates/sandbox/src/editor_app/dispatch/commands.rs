//! Shared authoring-command and entity-selection operations.

use super::*;

impl EditorApp {
    pub(super) fn require_editing(&self) -> Result<(), BridgeError> {
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

    pub(super) fn reject_current_scene_asset_reference(
        &self,
        asset_id: &str,
    ) -> Result<(), BridgeError> {
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

    pub(super) fn set_runtime_mode(
        &mut self,
        requested: RuntimeModeDto,
    ) -> Result<(), BridgeError> {
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

    pub(super) fn execute_command(&mut self, command: Box<dyn Command>) -> Result<(), BridgeError> {
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

    pub(super) fn undo_or_redo(&mut self, undo: bool) -> Result<(), BridgeError> {
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

    pub(super) fn select_entities(
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

    pub(super) fn param_or_selected(&self, entity_id: &str) -> Result<String, BridgeError> {
        if !entity_id.is_empty() {
            return Ok(entity_id.to_string());
        }
        self.editor_scene
            .as_ref()
            .and_then(|scene| scene.selected_entity.clone())
            .ok_or_else(selection_error)
    }

    pub(super) fn command_entity_ids(
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

    pub(super) fn root_entity_ids(&self, requested: &[String]) -> Result<Vec<String>, BridgeError> {
        let scene = self.editor_scene.as_ref().ok_or_else(runtime_unavailable)?;
        let clipboard = engine_editor::EntityClipboard::capture(&scene.scene, requested)
            .map_err(|error| validation_error(error.to_string()))?;
        Ok(clipboard.root_ids().to_vec())
    }

    pub(super) fn clear_removed_selection(&mut self, removed_roots: &[String]) {
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

    pub(super) fn capture_entities(&mut self, requested: &[String]) -> Result<(), BridgeError> {
        let roots = self.command_entity_ids("", requested)?;
        let scene = self.editor_scene.as_ref().ok_or_else(runtime_unavailable)?;
        self.entity_clipboard = Some(
            engine_editor::EntityClipboard::capture(&scene.scene, &roots)
                .map_err(|error| validation_error(error.to_string()))?,
        );
        Ok(())
    }

    pub(super) fn add_component_command(
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

    pub(super) fn reset_component_command(
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

    pub(super) fn entity(&self, entity_id: &str) -> Result<&EntityRecord, BridgeError> {
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
}
