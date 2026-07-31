//! Runtime, terrain, diagnostics, project, input-map, and script requests.

use super::*;

impl EditorApp {
    pub(super) fn dispatch_project_request(
        &mut self,
        request: EditorRequest,
    ) -> Result<DispatchOutcome, BridgeError> {
        match request {
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
                        super::super::super::project_cli::create_project(
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
                super::super::super::project_input::validate_input_map(&params.map)
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
                super::super::super::project_input::save_project_input_map(&self.project, map)
                    .map_err(io_error)?;
                self.build_status = Some("Input actions saved.".into());
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::CreateScript(params) => {
                self.require_editing()?;
                let path = super::super::super::project_scripts::create_project_script_in(
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
            _ => unreachable!("request routed to the wrong editor IPC domain"),
        }
    }
}
