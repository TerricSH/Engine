use super::*;

impl EditorApp {
    pub(super) fn record_scene_document_error(&mut self, message: impl Into<String>) {
        let message = message.into();
        tracing::error!(%message, "editor scene document operation failed");
        self.scene_document_status = Some(message.clone());
        if let Some(editor_scene) = self.editor_scene.as_mut() {
            editor_scene.diagnostics.push(Diagnostic::new(
                "EDSCENE_DOCUMENT_FAILED",
                DiagnosticSeverity::Error,
                "editor.scene-document",
                message,
            ));
        }
        self.request_ui_open_panel(protocol::UiPanel::Console, protocol::UiDockZone::Bottom);
    }

    pub(super) fn reload_project_manifest(&mut self) -> Result<(), String> {
        let reloaded = GameProject::load(&self.project.manifest_path)
            .map_err(|error| format!("Could not reload project scene catalog: {error}"))?;
        let current_scene_path = reloaded.scene_path(&self.current_scene_id).ok_or_else(|| {
            format!(
                "Reloaded scene catalog no longer contains the open scene '{}'",
                self.current_scene_id
            )
        })?;
        self.current_scene_path = current_scene_path;
        self.project = reloaded;
        Ok(())
    }

    pub(super) fn save_current_scene_document(&mut self) -> Result<(), String> {
        let editor_scene = self
            .editor_scene
            .as_mut()
            .ok_or_else(|| "No editor scene is open".to_string())?;
        save_scene_atomically(&editor_scene.scene, &self.current_scene_path)?;
        editor_scene.history.mark_clean();
        let recovery = scene_recovery_path(&self.project, &self.current_scene_id);
        match std::fs::remove_file(&recovery) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                tracing::warn!(path = %recovery.display(), %error, "could not remove saved recovery snapshot")
            }
        }
        self.pending_recovery = None;
        tracing::info!(
            scene_id = self.current_scene_id,
            scene = %self.current_scene_path.display(),
            "editor scene saved"
        );
        self.scene_document_status = Some(format!("Saved '{}'.", self.current_scene_id));
        Ok(())
    }

    pub(super) fn maybe_write_recovery_snapshot(&mut self) {
        const RECOVERY_INTERVAL_SECONDS: u64 = 30;
        if self.last_recovery_snapshot.elapsed().as_secs() < RECOVERY_INTERVAL_SECONDS {
            return;
        }
        self.last_recovery_snapshot = Instant::now();
        if !self.play_session.is_editing()
            || !self
                .editor_scene
                .as_ref()
                .is_some_and(EditorScene::is_dirty)
        {
            return;
        }
        let Some(scene) = self.editor_scene.as_ref().map(|scene| &scene.scene) else {
            return;
        };
        let recovery = scene_recovery_path(&self.project, &self.current_scene_id);
        match save_scene_atomically(scene, &recovery) {
            Ok(()) => {
                self.build_status = Some(format!(
                    "Recovery snapshot updated for '{}'.",
                    self.current_scene_id
                ));
            }
            Err(error) => self.record_scene_document_error(format!(
                "Could not write recovery snapshot {}: {error}",
                recovery.display()
            )),
        }
    }

    pub(super) fn restore_recovery_snapshot(&mut self) -> Result<(), String> {
        let recovery = self
            .pending_recovery
            .clone()
            .ok_or_else(|| "No recovery snapshot is pending".to_string())?;
        let scene = Scene::load_from_file(&recovery).map_err(|error| {
            format!(
                "Could not load recovery snapshot {}: {error}",
                recovery.display()
            )
        })?;
        super::super::project_scripts::validate_runtime_script_references(&self.project, &scene)?;
        let game_loop = self
            .game_loop
            .as_mut()
            .ok_or_else(|| "Editor runtime is not initialized".to_string())?;
        let (preview_scene, diagnostics) = editor_preview_scene(&game_loop.runtime, &scene);
        game_loop.load_scene(preview_scene).map_err(|diagnostics| {
            format!(
                "Recovery snapshot could not be restored into the editor runtime: {}",
                summarize_scene_diagnostics(&diagnostics)
            )
        })?;
        game_loop.init_physics();
        let mut editor_scene = EditorScene::new_with_component_registry(
            scene,
            std::sync::Arc::clone(game_loop.runtime.component_registry()),
        )
        .map_err(|error| format!("Recovery snapshot is not authorable: {error}"))?;
        editor_scene.history.mark_dirty();
        editor_scene.diagnostics.push_many(diagnostics);
        self.scene_settings_draft = editor_scene.scene.scene_settings.clone();
        self.editor_scene = Some(editor_scene);
        self.selected_entity_ids.clear();
        self.pending_recovery = None;
        self.scene_document_status = Some(format!(
            "Recovered unsaved changes for '{}'; save to keep them.",
            self.current_scene_id
        ));
        Ok(())
    }

    pub(super) fn discard_recovery_snapshot(&mut self) -> Result<(), String> {
        let Some(recovery) = self.pending_recovery.take() else {
            return Ok(());
        };
        match std::fs::remove_file(&recovery) {
            Ok(()) => {
                self.scene_document_status = Some("Recovery snapshot discarded.".to_string());
                Ok(())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!(
                "Could not discard recovery snapshot {}: {error}",
                recovery.display()
            )),
        }
    }

    /// Load and validate a catalog scene before replacing the active document.
    /// `GameLoop::load_scene` is transactional, so any strict ECS failure leaves
    /// the currently rendered authoring preview untouched.
    pub(super) fn switch_scene_document(&mut self, scene_id: &str) -> Result<bool, String> {
        if !self.play_session.is_editing() {
            return Err("Stop Play before opening another scene.".to_string());
        }
        if scene_id == self.current_scene_id {
            self.pending_scene_switch = None;
            self.pending_document_action = None;
            self.scene_document_status = Some(format!("Scene '{scene_id}' is already open."));
            return Ok(false);
        }

        let scene_path = self.project.scene_path(scene_id).ok_or_else(|| {
            format!("Unknown project scene '{scene_id}'; refresh the project scene catalog.")
        })?;
        let scene = Scene::load_from_file(&scene_path).map_err(|error| {
            format!(
                "Could not load scene '{}' from {}: {error}",
                scene_id,
                scene_path.display()
            )
        })?;
        super::super::project_scripts::validate_runtime_script_references(&self.project, &scene)
            .map_err(|error| {
                format!("Scene '{scene_id}' has invalid script references: {error}")
            })?;

        let game_loop = self
            .game_loop
            .as_mut()
            .ok_or_else(|| "Editor runtime is not initialized".to_string())?;
        let (preview_scene, preview_diagnostics) = editor_preview_scene(&game_loop.runtime, &scene);
        game_loop.load_scene(preview_scene).map_err(|diagnostics| {
            format!(
                "Scene '{scene_id}' could not be restored into the editor runtime: {}",
                summarize_scene_diagnostics(&diagnostics)
            )
        })?;
        game_loop.init_physics();

        let mut editor_scene = EditorScene::new_with_component_registry(
            scene,
            std::sync::Arc::clone(game_loop.runtime.component_registry()),
        )
        .map_err(|error| format!("Scene '{scene_id}' is not authorable: {error}"))?;
        editor_scene.diagnostics.push_many(preview_diagnostics);
        self.scene_settings_draft = editor_scene.scene.scene_settings.clone();
        self.editor_scene = Some(editor_scene);
        self.selected_entity_ids.clear();
        self.current_scene_id = scene_id.to_string();
        self.current_scene_path = scene_path;
        self.scene_browser_selection = scene_id.to_string();
        self.pending_scene_switch = None;
        self.pending_document_action = None;
        self.gizmo.cancel_drag();
        self.gizmo_pointer_events.clear();
        self.viewport_tab = ViewportTab::Scene;
        self.material_editor.reset();
        self.material_editor_selection = None;
        self.last_frame_time = Instant::now();
        self.scene_document_status = Some(format!("Opened scene '{scene_id}'."));
        self.pending_recovery = newer_recovery_snapshot(
            &self.project,
            &self.current_scene_id,
            &self.current_scene_path,
        );
        tracing::info!(scene_id, scene = %self.current_scene_path.display(), "editor scene opened");
        Ok(true)
    }

    pub(super) fn request_scene_switch(&mut self, scene_id: String) -> Result<bool, String> {
        if scene_id == self.current_scene_id {
            self.pending_scene_switch = None;
            self.pending_document_action = None;
            self.scene_document_status = Some(format!("Scene '{scene_id}' is already open."));
            return Ok(false);
        }
        if self
            .editor_scene
            .as_ref()
            .is_some_and(EditorScene::is_dirty)
        {
            self.pending_scene_switch = Some(format!("opening scene '{scene_id}'"));
            self.pending_document_action = Some(SceneDocumentAction::Open(scene_id.clone()));
            self.scene_document_status = Some(format!(
                "Unsaved changes: choose Save & Switch, Discard & Switch, or Cancel for '{scene_id}'."
            ));
            return Ok(false);
        }
        self.switch_scene_document(&scene_id)
    }

    pub(super) fn defer_document_action_if_dirty(
        &mut self,
        action: SceneDocumentAction,
        target_label: String,
    ) -> bool {
        if !self
            .editor_scene
            .as_ref()
            .is_some_and(EditorScene::is_dirty)
        {
            return false;
        }
        self.pending_scene_switch = Some(target_label.clone());
        self.pending_document_action = Some(action);
        self.scene_document_status = Some(format!(
            "Unsaved changes must be saved or discarded before {target_label}."
        ));
        true
    }

    pub(super) fn rename_scene_document(
        &mut self,
        old_id: &str,
        new_id: &str,
    ) -> Result<bool, String> {
        let old_id = old_id.trim();
        let new_id = new_id.trim();
        if old_id.is_empty() || new_id.is_empty() {
            return Err("Scene rename requires both the current and new scene IDs.".to_string());
        }
        super::super::project_cli::rename_project_scene(
            &self.project.manifest_path,
            old_id,
            new_id,
        )?;
        let reloaded = GameProject::load(&self.project.manifest_path)
            .map_err(|error| format!("Could not reload renamed scene catalog: {error}"))?;

        if old_id == self.current_scene_id {
            self.project = reloaded;
            self.switch_scene_document(new_id)?;
        } else {
            self.current_scene_path =
                reloaded.scene_path(&self.current_scene_id).ok_or_else(|| {
                    format!(
                        "Renaming '{old_id}' removed the current scene '{}' from the catalog",
                        self.current_scene_id
                    )
                })?;
            self.project = reloaded;
        }
        self.scene_browser_selection = new_id.to_string();
        self.scene_operation_id.clear();
        self.new_scene_id.clear();
        self.scene_document_status = Some(format!("Renamed scene '{old_id}' to '{new_id}'."));
        Ok(true)
    }

    pub(super) fn delete_scene_document(
        &mut self,
        scene_id: &str,
        replacement_startup: Option<&str>,
    ) -> Result<bool, String> {
        let scene_id = scene_id.trim();
        if scene_id.is_empty() {
            return Err("No scene was selected for deletion.".to_string());
        }
        let deleting_current = scene_id == self.current_scene_id;
        let deleted = super::super::project_cli::delete_project_scene(
            &self.project.manifest_path,
            scene_id,
            replacement_startup,
        )?;
        let reloaded = GameProject::load(&self.project.manifest_path)
            .map_err(|error| format!("Could not reload scene catalog after deletion: {error}"))?;

        if deleting_current {
            let next_scene = deleted
                .replacement_startup
                .clone()
                .unwrap_or_else(|| reloaded.startup_scene_id().to_string());
            self.project = reloaded;
            self.switch_scene_document(&next_scene)?;
            self.scene_browser_selection = next_scene;
        } else {
            self.current_scene_path =
                reloaded.scene_path(&self.current_scene_id).ok_or_else(|| {
                    format!(
                        "Deleting '{scene_id}' removed the current scene '{}' from the catalog",
                        self.current_scene_id
                    )
                })?;
            self.project = reloaded;
            self.scene_browser_selection = self.current_scene_id.clone();
        }
        self.scene_operation_id.clear();
        self.scene_replacement_id.clear();
        self.scene_document_status = Some(format!(
            "Moved scene '{}' to project trash at {} (metadata: {}).",
            deleted.scene_id,
            deleted.trash_directory.display(),
            deleted.metadata_path.display()
        ));
        Ok(true)
    }

    pub(super) fn apply_scene_document_action(
        &mut self,
        action: SceneDocumentAction,
    ) -> Result<bool, String> {
        if self.pending_document_action.is_some()
            && !matches!(&action, SceneDocumentAction::CancelSwitch)
        {
            return Err(
                "Resolve or cancel the pending scene document operation before starting another"
                    .to_string(),
            );
        }
        self.apply_scene_document_action_after_confirmation(action, false)
    }

    /// Applies a scene-document action after the caller has explicitly resolved
    /// the dirty-document prompt. A discarded document must stay dirty until a
    /// successful switch replaces it; marking it clean up front would turn a
    /// failed open/create/rename/delete into a false save checkpoint.
    pub(super) fn apply_scene_document_action_after_confirmation(
        &mut self,
        action: SceneDocumentAction,
        dirty_prompt_resolved: bool,
    ) -> Result<bool, String> {
        match action {
            SceneDocumentAction::Open(scene_id) => {
                if dirty_prompt_resolved {
                    self.switch_scene_document(&scene_id)
                } else {
                    self.request_scene_switch(scene_id)
                }
            }
            SceneDocumentAction::Create { scene_id, folder } => {
                let scene_id = scene_id.trim();
                if scene_id.is_empty() {
                    return Err("New scene ID must not be empty.".to_string());
                }
                if !dirty_prompt_resolved
                    && self.defer_document_action_if_dirty(
                        SceneDocumentAction::Create {
                            scene_id: scene_id.to_string(),
                            folder: folder.clone(),
                        },
                        format!("creating and opening scene '{scene_id}'"),
                    )
                {
                    return Ok(false);
                }
                super::super::project_cli::create_project_scene_in_folder(
                    &self.project.manifest_path,
                    scene_id,
                    None,
                    &folder,
                )?;
                self.reload_project_manifest()?;
                self.scene_browser_selection = scene_id.to_string();
                self.new_scene_id.clear();
                self.new_scene_folder.clear();
                self.scene_document_status = Some(format!("Created scene '{scene_id}'."));
                self.switch_scene_document(scene_id)
            }
            SceneDocumentAction::SaveAs(scene_id) => {
                let scene_id = scene_id.trim();
                if scene_id.is_empty() {
                    return Err("Save As scene ID must not be empty.".to_string());
                }
                let source = self
                    .editor_scene
                    .as_ref()
                    .ok_or_else(|| "No editor scene is open".to_string())?
                    .scene
                    .clone();
                super::super::project_cli::duplicate_project_scene(
                    &self.project.manifest_path,
                    scene_id,
                    &source,
                )?;
                self.reload_project_manifest()?;
                self.scene_browser_selection = scene_id.to_string();
                self.new_scene_id.clear();
                self.scene_document_status = Some(format!("Saved scene as '{scene_id}'."));
                self.switch_scene_document(scene_id)
            }
            SceneDocumentAction::Duplicate { source_id, new_id } => {
                let source_id = source_id.trim();
                let new_id = new_id.trim();
                if source_id.is_empty() || new_id.is_empty() {
                    return Err(
                        "Scene duplication requires both source and destination IDs.".to_string(),
                    );
                }
                if !dirty_prompt_resolved
                    && self.defer_document_action_if_dirty(
                        SceneDocumentAction::Duplicate {
                            source_id: source_id.to_string(),
                            new_id: new_id.to_string(),
                        },
                        format!("duplicating and opening scene '{new_id}'"),
                    )
                {
                    return Ok(false);
                }
                let source_path = self.project.scene_path(source_id).ok_or_else(|| {
                    format!("Unknown project scene '{source_id}' cannot be duplicated.")
                })?;
                let source = Scene::load_from_file(&source_path).map_err(|error| {
                    format!(
                        "Could not load scene '{source_id}' from {}: {error}",
                        source_path.display()
                    )
                })?;
                super::super::project_cli::duplicate_project_scene(
                    &self.project.manifest_path,
                    new_id,
                    &source,
                )?;
                self.reload_project_manifest()?;
                self.scene_browser_selection = new_id.to_string();
                self.scene_operation_id.clear();
                self.new_scene_id.clear();
                self.scene_document_status =
                    Some(format!("Duplicated scene '{source_id}' as '{new_id}'."));
                self.switch_scene_document(new_id)
            }
            SceneDocumentAction::SetStartup(scene_id) => {
                super::super::project_cli::set_project_startup_scene(
                    &self.project.manifest_path,
                    &scene_id,
                )?;
                self.reload_project_manifest()?;
                self.scene_document_status =
                    Some(format!("Scene '{scene_id}' is now the startup scene."));
                Ok(false)
            }
            SceneDocumentAction::Rename { old_id, new_id } => {
                if !dirty_prompt_resolved
                    && old_id == self.current_scene_id
                    && self.defer_document_action_if_dirty(
                        SceneDocumentAction::Rename {
                            old_id: old_id.clone(),
                            new_id: new_id.clone(),
                        },
                        format!("renaming scene '{old_id}' to '{new_id}'"),
                    )
                {
                    return Ok(false);
                }
                self.rename_scene_document(&old_id, &new_id)
            }
            SceneDocumentAction::Delete {
                scene_id,
                replacement_startup,
            } => {
                if !dirty_prompt_resolved
                    && scene_id == self.current_scene_id
                    && self.defer_document_action_if_dirty(
                        SceneDocumentAction::Delete {
                            scene_id: scene_id.clone(),
                            replacement_startup: replacement_startup.clone(),
                        },
                        format!("deleting scene '{scene_id}'"),
                    )
                {
                    return Ok(false);
                }
                self.delete_scene_document(&scene_id, replacement_startup.as_deref())
            }
            SceneDocumentAction::CancelSwitch => {
                self.pending_scene_switch = None;
                self.pending_document_action = None;
                self.scene_document_status = Some("Scene switch cancelled.".to_string());
                Ok(false)
            }
        }
    }

    pub(super) fn apply_close_document_action(
        &mut self,
        action: CloseDocumentAction,
    ) -> Result<(), String> {
        match action {
            CloseDocumentAction::SaveAndClose => {
                self.save_current_scene_document()?;
                self.pending_scene_switch = None;
                self.pending_document_action = None;
                self.close_confirmation_pending = false;
                self.exit_after_frame = true;
            }
            CloseDocumentAction::DiscardAndClose => {
                self.pending_scene_switch = None;
                self.pending_document_action = None;
                self.close_confirmation_pending = false;
                self.exit_after_frame = true;
            }
            CloseDocumentAction::Cancel => {
                self.close_confirmation_pending = false;
                self.scene_document_status = Some("Editor close cancelled.".to_string());
            }
        }
        Ok(())
    }
}
