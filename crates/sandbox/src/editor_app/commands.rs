use super::*;

impl EditorApp {
    pub(super) fn execute_editor_command(
        &mut self,
        command: Box<dyn engine_editor::Command>,
    ) -> bool {
        let label = command.name().to_string();
        if !self.play_session.is_editing() {
            self.record_editor_command_error(&label, "Stop Play mode before editing the scene");
            return false;
        }
        let (Some(game_loop), Some(editor_scene)) =
            (self.game_loop.as_mut(), self.editor_scene.as_mut())
        else {
            self.record_editor_command_error(&label, "Editor scene runtime is not initialized");
            return false;
        };
        let result = match editor_scene.execute(command) {
            Ok(()) => {
                let existing_ids = editor_scene
                    .scene
                    .entities
                    .iter()
                    .map(|entity| entity.persistent_id.clone())
                    .collect::<std::collections::BTreeSet<_>>();
                self.selected_entity_ids
                    .retain(|entity_id| existing_ids.contains(entity_id));
                let selection_exists = editor_scene
                    .selected_entity
                    .as_ref()
                    .is_some_and(|id| existing_ids.contains(id));
                if !selection_exists {
                    editor_scene.selected_entity = self.selected_entity_ids.first().cloned();
                } else if let Some(active) = editor_scene.selected_entity.as_ref() {
                    if !self.selected_entity_ids.contains(active) {
                        self.selected_entity_ids.push(active.clone());
                    }
                }
                synchronize_authoring_view(
                    game_loop,
                    editor_scene,
                    &self.scene_view,
                    self.viewport_tab,
                );
                Ok((label == "Set Scene Settings")
                    .then(|| editor_scene.scene.scene_settings.clone()))
            }
            Err(error) => Err(error.to_string()),
        };
        match result {
            Ok(scene_settings) => {
                if let Some(scene_settings) = scene_settings {
                    self.scene_settings_draft = scene_settings;
                }
                true
            }
            Err(error) => {
                self.record_editor_command_error(&label, error);
                false
            }
        }
    }

    pub(super) fn record_build_error(&mut self, label: &str, error: String) {
        self.build_status = Some(format!("{label} failed: {error}"));
        if let Some(editor_scene) = self.editor_scene.as_mut() {
            editor_scene.diagnostics.push(Diagnostic::new(
                "EDBUILD_FAILED",
                DiagnosticSeverity::Error,
                "editor.build",
                format!("{label} failed: {error}"),
            ));
        }
        self.request_ui_open_panel(protocol::UiPanel::Console, protocol::UiDockZone::Bottom);
    }

    pub(super) fn record_editor_command_error(&mut self, label: &str, error: impl Into<String>) {
        let error = error.into();
        tracing::error!(label, %error, "editor authoring command failed");
        self.build_status = Some(format!("{label} failed: {error}"));
        if let Some(editor_scene) = self.editor_scene.as_mut() {
            editor_scene.diagnostics.push(Diagnostic::new(
                "EDCOMMAND_FAILED",
                DiagnosticSeverity::Error,
                "editor.command",
                format!("{label} failed: {error}"),
            ));
        }
        self.request_ui_open_panel(protocol::UiPanel::Console, protocol::UiDockZone::Bottom);
    }

    pub(super) fn create_prefab_from_selection(
        &mut self,
        asset_id: String,
        relative_source_path: PathBuf,
        manifest_name: PathBuf,
    ) -> Result<(), String> {
        if !self.play_session.is_editing() {
            return Err("Stop Play before authoring a prefab".to_string());
        }
        if self.background_job.is_some() || self.editor_build_task.is_some() {
            return Err("Wait for the active project operation to finish".to_string());
        }
        let created = (|| {
            let editor_scene = self
                .editor_scene
                .as_ref()
                .ok_or_else(|| "No editor scene is open".to_string())?;
            let selected = editor_scene
                .selected_entity
                .as_ref()
                .ok_or_else(|| "Select an entity hierarchy to create a prefab".to_string())?;
            create_prefab_asset_from_scene(
                &editor_scene.scene,
                selected,
                PrefabAssetCreateRequest {
                    source_root: &self.project.asset_source,
                    manifest_path: &manifest_name,
                    relative_source_path: &relative_source_path,
                    asset_id: AssetId::new(asset_id),
                },
            )
            .map_err(|error| error.to_string())
        })()?;

        self.refresh_asset_catalog()?;
        self.asset_browser
            .select_asset(Some(created.asset_id.clone()));
        let source_path = created.source_path.clone();
        let asset_id = created.asset_id.id.clone();
        self.start_editor_build(
            super::super::editor_build_ops::EditorBuildOperation::CookAndCompile,
        );
        if self.editor_build_task.is_some() {
            self.build_status = Some(format!(
                "Created prefab '{asset_id}' at {}; cooking and compiling it through the project build pipeline...",
                source_path.display()
            ));
        }
        Ok(())
    }

    pub(super) fn instantiate_prefab_asset(
        &mut self,
        asset_id: AssetId,
        parent_id: Option<PersistentId>,
    ) -> Result<(), String> {
        let prepared = (|| {
            let editor_scene = self
                .editor_scene
                .as_ref()
                .ok_or_else(|| "No editor scene is open".to_string())?;
            let game_loop = self
                .game_loop
                .as_ref()
                .ok_or_else(|| "Editor runtime is not initialized".to_string())?;
            if game_loop
                .runtime
                .asset_registry()
                .get::<engine_scene::Prefab>(&asset_id)
                .is_none()
            {
                return Err(format!(
                    "Prefab '{}' is not loaded. Run Cook & Compile Project, then instantiate it.",
                    asset_id.id
                ));
            }
            prepare_prefab_instantiation_from_registry(
                &editor_scene.scene,
                game_loop.runtime.asset_registry(),
                &asset_id,
                parent_id
                    .clone()
                    .map(engine_editor::EntityPasteParent::Entity)
                    .unwrap_or(engine_editor::EntityPasteParent::SceneRoot),
            )
            .map_err(|error| match error {
                PrefabAuthoringError::AssetNotLoaded(missing) => format!(
                    "Prefab '{missing}' is not loaded. Run Cook & Compile Project, then instantiate it."
                ),
                error => error.to_string(),
            })
        })();
        let plan = prepared?;
        let root = plan.root_entity_id().clone();
        let count = plan.entity_ids().len();
        if !self.execute_editor_command(plan.into_command()) {
            self.request_ui_open_panel(protocol::UiPanel::Console, protocol::UiDockZone::Bottom);
            return Err("The prefab instantiation command was rejected".to_string());
        }
        self.selected_entity_ids = vec![root.clone()];
        if let Some(editor_scene) = self.editor_scene.as_mut() {
            editor_scene.selected_entity = Some(root.clone());
        }
        self.build_status = Some(format!(
            "Instantiated prefab '{}' as '{}' ({} entities).",
            asset_id.id, root, count
        ));
        Ok(())
    }

    pub(super) fn unpack_prefab_instance(
        &mut self,
        entity_id: PersistentId,
        mode: PrefabUnpackMode,
    ) -> Result<(), String> {
        let plan = self
            .editor_scene
            .as_ref()
            .ok_or_else(|| "No editor scene is open".to_string())
            .and_then(|editor_scene| {
                prepare_unpack_prefab(&editor_scene.scene, &entity_id, mode)
                    .map_err(|error| error.to_string())
            });
        let plan = plan?;
        let count = plan.entity_ids().len();
        if !self.execute_editor_command(plan.into_command()) {
            self.request_ui_open_panel(protocol::UiPanel::Console, protocol::UiDockZone::Bottom);
            return Err("The prefab unpack command was rejected".to_string());
        }
        self.selected_entity_ids = vec![entity_id.clone()];
        if let Some(editor_scene) = self.editor_scene.as_mut() {
            editor_scene.selected_entity = Some(entity_id.clone());
        }
        let scope = match mode {
            PrefabUnpackMode::Instance => "instance",
            PrefabUnpackMode::Completely => "instance and nested prefab links",
        };
        self.build_status = Some(format!(
            "Unpacked {scope} at '{entity_id}' ({count} prefab link records removed)."
        ));
        Ok(())
    }

    pub(super) fn refresh_asset_catalog(
        &mut self,
    ) -> Result<engine_editor::asset_browser::AssetRefreshSummary, String> {
        let game_loop = self
            .game_loop
            .as_ref()
            .ok_or_else(|| "Editor runtime is not initialized".to_string())?;
        let requested_folder = self.workspace_preferences.project_asset_folder.clone();
        let summary = refresh_project_asset_list(
            &mut self.asset_browser,
            game_loop.runtime.asset_registry(),
            &self.project.asset_source,
        )
        .map_err(|error| error.to_string())?;
        self.asset_browser.set_current_folder(requested_folder);
        self.workspace_preferences.project_asset_folder =
            self.asset_browser.current_folder().to_string();
        Ok(summary)
    }

    pub(super) fn copy_component_to_clipboard(
        &mut self,
        entity_id: &PersistentId,
        component_type: &str,
    ) -> Result<(), String> {
        let editor_scene = self
            .editor_scene
            .as_ref()
            .ok_or_else(|| "No editor scene is open".to_string())?;
        let clipboard = engine_editor::ComponentClipboard::capture(
            &editor_scene.scene,
            entity_id,
            &component_type.to_string(),
        )
        .map_err(|error| error.to_string())?;
        self.component_clipboard = Some(clipboard);
        self.build_status = Some(format!("Copied component '{component_type}'."));
        Ok(())
    }

    pub(super) fn paste_component_to_entities(
        &mut self,
        entity_ids: Vec<PersistentId>,
        component_type: String,
    ) -> Result<(), String> {
        let commands = {
            let editor_scene = self
                .editor_scene
                .as_ref()
                .ok_or_else(|| "No editor scene is open".to_string())?;
            let clipboard = self
                .component_clipboard
                .as_ref()
                .ok_or_else(|| "The component clipboard is empty".to_string())?;
            entity_ids
                .into_iter()
                .map(|entity_id| {
                    engine_editor::ReplaceComponent::prepare(
                        &editor_scene.scene,
                        entity_id,
                        component_type.clone(),
                        clipboard,
                    )
                    .map(|command| Box::new(command) as Box<dyn engine_editor::Command>)
                    .map_err(|error| error.to_string())
                })
                .collect::<Result<Vec<_>, _>>()?
        };
        if !self.execute_editor_command(Box::new(engine_editor::CommandBatch::new(
            "Paste Component Values",
            commands,
        ))) {
            return Err("The component changed before paste could be applied".to_string());
        }
        Ok(())
    }

    pub(super) fn paste_entity_clipboard(
        &mut self,
        parent: engine_editor::EntityPasteParent,
    ) -> Result<(), String> {
        let (command, selected) = {
            let editor_scene = self
                .editor_scene
                .as_ref()
                .ok_or_else(|| "No editor scene is open".to_string())?;
            let clipboard = self
                .entity_clipboard
                .as_ref()
                .ok_or_else(|| "The entity clipboard is empty".to_string())?;
            let command =
                engine_editor::PasteEntityRecords::prepare(&editor_scene.scene, clipboard, parent)
                    .map_err(|error| error.to_string())?;
            let selected = command.pasted_root_ids().to_vec();
            if selected.is_empty() {
                return Err("The prepared paste has no root entity".to_string());
            }
            (
                Box::new(command) as Box<dyn engine_editor::Command>,
                selected,
            )
        };
        if !self.execute_editor_command(command) {
            return Err("The scene changed before the paste could be applied".to_string());
        }
        self.selected_entity_ids = selected.clone();
        if let Some(editor_scene) = self.editor_scene.as_mut() {
            editor_scene.selected_entity = selected.first().cloned();
        }
        Ok(())
    }

    pub(super) fn duplicate_entities(&mut self, source_ids: &[PersistentId]) -> Result<(), String> {
        let (command, selected) = {
            let editor_scene = self
                .editor_scene
                .as_ref()
                .ok_or_else(|| "No editor scene is open".to_string())?;
            let clipboard =
                engine_editor::EntityClipboard::capture(&editor_scene.scene, source_ids)
                    .map_err(|error| error.to_string())?;
            let command = engine_editor::PasteEntityRecords::prepare(
                &editor_scene.scene,
                &clipboard,
                engine_editor::EntityPasteParent::PreserveOriginal,
            )
            .map_err(|error| error.to_string())?;
            let selected = command.pasted_root_ids().to_vec();
            (
                Box::new(command) as Box<dyn engine_editor::Command>,
                selected,
            )
        };
        if !self.execute_editor_command(command) {
            return Err("The scene changed before duplication could be applied".to_string());
        }
        self.selected_entity_ids = selected.clone();
        if let Some(editor_scene) = self.editor_scene.as_mut() {
            editor_scene.selected_entity = selected.first().cloned();
        }
        Ok(())
    }

    pub(super) fn process_material_save(&mut self) {
        let request = match self.material_editor.take_save_request() {
            Ok(request) => request,
            Err(error) => {
                self.material_editor.report_save_failure(error.clone());
                if let Some(editor_scene) = self.editor_scene.as_mut() {
                    editor_scene.diagnostics.push(Diagnostic::new(
                        "EDMATERIAL_SAVE_FAILED",
                        DiagnosticSeverity::Error,
                        "editor.material",
                        error,
                    ));
                }
                return;
            }
        };
        let Some(request) = request else {
            return;
        };
        let result = if !self.play_session.is_editing() {
            Err("Stop Play before saving project materials.".to_string())
        } else if let Some(game_loop) = self.game_loop.as_mut() {
            save_project_material(&mut game_loop.runtime, &self.project, &request)
        } else {
            Err("Editor runtime is not initialized".to_string())
        };
        match result {
            Ok(outcome) => {
                self.material_editor.report_save_success(format!(
                    "Saved {} and refreshed {}.",
                    outcome.source_path.display(),
                    outcome.cooked_path.display()
                ));
                if let Err(error) = self.refresh_asset_catalog() {
                    self.record_editor_command_error("Refresh project asset catalog", error);
                }
            }
            Err(error) => {
                self.material_editor.report_save_failure(error.clone());
                let mut diagnostic = Diagnostic::new(
                    "EDMATERIAL_SAVE_FAILED",
                    DiagnosticSeverity::Error,
                    "editor.material",
                    error.clone(),
                );
                diagnostic.asset = Some(AssetId::new(request.material_asset));
                if let Some(editor_scene) = self.editor_scene.as_mut() {
                    editor_scene.diagnostics.push(diagnostic);
                }
                tracing::error!(%error, "editor material save failed");
            }
        }
    }
}
