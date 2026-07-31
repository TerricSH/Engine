use super::*;

impl EditorApp {
    pub(super) fn start_editor_job(
        &mut self,
        label: impl Into<String>,
        reload_assets: bool,
        operation: impl FnOnce() -> Result<EditorJobOutput, String> + Send + 'static,
    ) -> Result<u64, String> {
        let label = label.into();
        if let Some(active) = self.editor_build_task.as_ref() {
            return Err(format!(
                "{} is already running; wait before starting {label}.",
                active.operation().display_name()
            ));
        }
        if let Some(active) = self.background_job.as_ref() {
            return Err(format!(
                "{} is already running; wait for it to finish.",
                active.label
            ));
        }
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(operation());
        });
        let id = self.next_editor_operation_id;
        self.next_editor_operation_id = self.next_editor_operation_id.wrapping_add(1);
        self.build_status = Some(format!("{label} in progress..."));
        self.set_editor_operation_status(EditorOperationStatus {
            id,
            label: label.clone(),
            state: EditorOperationState::Running,
        });
        self.background_job = Some(EditorBackgroundJob {
            id,
            label,
            receiver,
            reload_assets,
        });
        Ok(id)
    }

    pub(super) fn poll_editor_job(&mut self) -> bool {
        let result = self
            .background_job
            .as_ref()
            .and_then(|job| match job.receiver.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => Some(Err(format!(
                    "{} worker terminated without a result",
                    job.label
                ))),
            });
        let Some(result) = result else {
            return false;
        };
        let job = self
            .background_job
            .take()
            .expect("completed editor job must still be present");
        match result {
            Ok(output) => {
                let refresh_result = (|| {
                    if job.reload_assets {
                        let game_loop = self
                            .game_loop
                            .as_mut()
                            .ok_or_else(|| "Editor runtime is not initialized".to_string())?;
                        super::super::project_app::load_project_assets(
                            &mut game_loop.runtime,
                            &self.project,
                        )?;
                        self.material_editor_selection = None;
                    }
                    self.refresh_asset_catalog()?;
                    self.apply_editor_job_output(output)
                })();
                match refresh_result {
                    Ok(()) => {
                        self.set_editor_operation_status(EditorOperationStatus {
                            id: job.id,
                            label: job.label.clone(),
                            state: EditorOperationState::Succeeded,
                        });
                        self.build_status = Some(format!("{} completed successfully.", job.label));
                    }
                    Err(error) => {
                        let warning = format!(
                            "{} committed its project files, but the editor could not refresh the result: {error}. Do not retry the mutation; use Refresh after resolving the reported error.",
                            job.label
                        );
                        self.set_editor_operation_status(EditorOperationStatus {
                            id: job.id,
                            label: job.label.clone(),
                            state: EditorOperationState::CommittedWithWarning(warning.clone()),
                        });
                        self.record_build_error(&format!("{} refresh", job.label), warning);
                    }
                }
            }
            Err(error) => {
                self.set_editor_operation_status(EditorOperationStatus {
                    id: job.id,
                    label: job.label.clone(),
                    state: EditorOperationState::Failed(error.clone()),
                });
                self.record_build_error(&job.label, error);
            }
        }
        true
    }

    pub(super) fn set_editor_operation_status(&mut self, status: EditorOperationStatus) {
        if let Some(existing) = self
            .recent_editor_operations
            .iter_mut()
            .find(|existing| existing.id == status.id)
        {
            *existing = status.clone();
        } else {
            self.recent_editor_operations.push_back(status.clone());
            while self.recent_editor_operations.len() > 16 {
                self.recent_editor_operations.pop_front();
            }
        }
        self.last_editor_operation = Some(status);
    }

    pub(super) fn apply_editor_job_output(
        &mut self,
        output: EditorJobOutput,
    ) -> Result<(), String> {
        match output {
            EditorJobOutput::None => {}
            EditorJobOutput::SelectAsset(asset_id) => {
                if !self.asset_browser.reveal_asset(&asset_id) {
                    return Err(format!(
                        "asset '{asset_id}' was committed but is missing from the refreshed catalog"
                    ));
                }
            }
            EditorJobOutput::SelectFolder(folder) => {
                let normalized = folder.trim().replace('\\', "/");
                let requested = if normalized.trim_matches('/').is_empty() {
                    "/".to_string()
                } else {
                    format!("/{}", normalized.trim_matches('/'))
                };
                self.asset_browser.set_current_folder(&requested);
                if !self
                    .asset_browser
                    .current_folder()
                    .eq_ignore_ascii_case(&requested)
                {
                    return Err(format!(
                        "asset folder '{requested}' was committed but is missing from the refreshed catalog"
                    ));
                }
                self.asset_browser.select_asset(None);
            }
            EditorJobOutput::ClearAssetSelection => {
                self.asset_browser.select_asset(None);
            }
        }
        self.workspace_preferences.project_asset_folder =
            self.asset_browser.current_folder().to_string();
        Ok(())
    }

    pub(super) fn start_editor_build(
        &mut self,
        operation: super::super::editor_build_ops::EditorBuildOperation,
    ) {
        let label = operation.kind().display_name();
        if let Some(job) = self.background_job.as_ref() {
            self.build_status = Some(format!(
                "{} is already running; wait before starting {label}.",
                job.label
            ));
            return;
        }
        if let Some(task) = self.editor_build_task.as_ref() {
            self.build_status = Some(format!(
                "{} is already running; cancel it or wait for completion.",
                task.operation().display_name()
            ));
            return;
        }
        let task = match self.editor_build_service.as_ref() {
            Ok(service) => service.start(&self.project.manifest_path, operation),
            Err(error) => {
                self.record_build_error(label, error.clone());
                return;
            }
        };
        match task {
            Ok(task) => {
                self.build_output.clear();
                self.build_status = Some(format!("{label} in progress..."));
                self.request_ui_open_panel(protocol::UiPanel::Build, protocol::UiDockZone::Bottom);
                self.editor_build_task = Some(task);
            }
            Err(error) => self.record_build_error(label, error.to_string()),
        }
    }

    pub(super) fn poll_editor_build(&mut self) -> bool {
        let Some(task) = self.editor_build_task.as_mut() else {
            return false;
        };
        let output = task.output_snapshot();
        self.build_output = match (output.stdout.trim(), output.stderr.trim()) {
            ("", "") => String::new(),
            (stdout, "") => stdout.to_string(),
            ("", stderr) => stderr.to_string(),
            (stdout, stderr) => format!("{stdout}\n\n--- stderr ---\n{stderr}"),
        };
        let Some(result) = task.try_complete() else {
            return false;
        };
        self.editor_build_task = None;
        match result {
            Ok(super::super::editor_build_ops::EditorBuildResult::Validated(result)) => {
                self.build_status = Some(format!(
                    "Validated '{}': {} scenes, {} entities, {} declared / {} cooked assets in {:.2}s.",
                    result.project,
                    result.scenes,
                    result.entities,
                    result.declared_assets,
                    result.cooked_assets,
                    result.elapsed.as_secs_f32()
                ));
            }
            Ok(super::super::editor_build_ops::EditorBuildResult::CookedAndCompiled(result)) => {
                self.build_status = Some(format!(
                    "Cooked and compiled '{}' in {:.2}s{}.",
                    result.project,
                    result.elapsed.as_secs_f32(),
                    if result.scripts_configured {
                        " including project scripts"
                    } else {
                        ""
                    }
                ));
                let reload = self
                    .game_loop
                    .as_mut()
                    .ok_or_else(|| "Editor runtime is not initialized".to_string())
                    .and_then(|game_loop| {
                        super::super::project_app::load_project_assets(
                            &mut game_loop.runtime,
                            &self.project,
                        )
                        .map(|_| ())
                    });
                if let Err(error) = reload {
                    self.run_after_build = false;
                    self.record_build_error("Cook & Compile asset reload", error);
                } else if let Err(error) = self.refresh_asset_catalog() {
                    self.run_after_build = false;
                    self.record_build_error("Refresh project asset catalog", error);
                } else if self.run_after_build {
                    self.run_after_build = false;
                    match self.launch_project_player() {
                        Ok(pid) => {
                            self.build_status = Some(format!(
                                "Cooked, validated, and started project player ({pid})."
                            ));
                        }
                        Err(error) => self.record_build_error("Run project", error),
                    }
                }
            }
            Ok(super::super::editor_build_ops::EditorBuildResult::PackagedWindows(result)) => {
                self.build_status = Some(format!(
                    "Packaged Windows player {} in {:.2}s: {} (SHA-256 {}).",
                    result.version,
                    result.elapsed.as_secs_f32(),
                    result.archive_path.display(),
                    result.archive_sha256
                ));
                self.build_output.push_str(&format!(
                    "\n\nRelease root: {}\nArchive: {}\nArchive SHA-256: {}\nSymbols: {}\nSymbols SHA-256: {}\nManifest: {}\nDirty worktree: {}",
                    result.release_root.display(),
                    result.archive_path.display(),
                    result.archive_sha256,
                    result.symbols_archive_path.display(),
                    result.symbols_sha256,
                    result.release_manifest_path.display(),
                    result.dirty
                ));
            }
            Err(error) => {
                self.run_after_build = false;
                self.record_build_error(error.operation.display_name(), error.to_string())
            }
        }
        true
    }

    pub(super) fn request_run_project(&mut self) {
        if self.editor_build_task.is_some() || self.background_job.is_some() {
            self.record_build_error(
                "Run project",
                "Wait for the active project operation to finish".to_string(),
            );
            return;
        }
        if !self.play_session.is_editing() {
            self.record_build_error(
                "Run project",
                "Stop the in-editor Play session before launching the player".to_string(),
            );
            return;
        }
        if let Err(error) = self.save_current_scene_document() {
            self.record_build_error("Run project", error);
            return;
        }
        let input_save = self
            .game_loop
            .as_ref()
            .ok_or_else(|| "Editor runtime is not initialized".to_string())
            .and_then(|game_loop| {
                super::super::project_input::save_project_input_map(
                    &self.project,
                    &game_loop.input_map,
                )
            });
        if let Err(error) = input_save {
            self.record_build_error("Run project input settings", error);
            return;
        }
        self.run_after_build = true;
        self.start_editor_build(
            super::super::editor_build_ops::EditorBuildOperation::CookAndCompile,
        );
        if self.editor_build_task.is_none() {
            self.run_after_build = false;
        } else {
            self.build_status =
                Some("Saving, validating, cooking, and compiling before Run...".to_string());
        }
    }

    pub(super) fn launch_project_player(&self) -> Result<u32, String> {
        let executable =
            match crate::engine_installation::EngineInstallation::discover_from_current_executable(
            )? {
                Some(installation) => installation.windows_runtime,
                None => std::env::current_exe()
                    .map_err(|error| format!("could not resolve editor executable: {error}"))?,
            };
        std::process::Command::new(&executable)
            .arg("project")
            .arg("run")
            .arg(&self.project.manifest_path)
            .arg("--scripts-already-built")
            .current_dir(&self.project.root)
            .spawn()
            .map(|child| child.id())
            .map_err(|error| {
                format!(
                    "could not launch project player {}: {error}",
                    executable.display()
                )
            })
    }

    pub(super) fn cancel_editor_build(&mut self) {
        let Some(task) = self.editor_build_task.as_ref() else {
            self.build_status = Some("No cancellable build operation is running.".to_string());
            return;
        };
        match task.cancel() {
            Ok(true) => {
                self.build_status =
                    Some(format!("Cancelling {}...", task.operation().display_name()));
            }
            Ok(false) => {
                self.build_status =
                    Some("The build process has already finished; collecting result.".to_string());
            }
            Err(error) => self.record_build_error("Cancel build", error.to_string()),
        }
    }

    pub(super) fn rebuild_and_reload_scripts(&mut self) {
        if self.background_job.is_some() || self.editor_build_task.is_some() {
            self.build_status = Some(
                "Wait for the active project operation before rebuilding scripts.".to_string(),
            );
            return;
        }
        let Some(game_loop) = self.game_loop.as_mut() else {
            self.record_build_error(
                "Rebuild & Reload Scripts",
                "Editor runtime is not initialized".to_string(),
            );
            return;
        };
        self.build_status = Some("Rebuilding project scripts...".to_string());
        match super::super::project_scripts::rebuild_and_reload_project_scripts(
            &mut game_loop.runtime,
            &self.project,
        ) {
            Ok(result) => {
                let verified_classes = game_loop.runtime.verified_script_classes().len();
                self.build_status = Some(format!(
                    "Rebuilt and transactionally reloaded {} script assemblies; {} concrete EngineBehaviour classes verified.",
                    result.assemblies, verified_classes
                ));
                self.request_ui_open_panel(protocol::UiPanel::Build, protocol::UiDockZone::Bottom);
            }
            Err(error) => self.record_build_error("Rebuild & Reload Scripts", error),
        }
    }

    pub(super) fn verified_script_add_command(
        &self,
        assembly_id: &str,
        class_name: &str,
    ) -> Result<Box<dyn engine_editor::Command>, String> {
        if self.project.script_assembly.is_none() {
            return Err(
                "game.project.json does not configure a compiled script_assembly".to_string(),
            );
        }
        let runtime = &self
            .game_loop
            .as_ref()
            .ok_or_else(|| "Editor runtime is not initialized".to_string())?
            .runtime;
        if !runtime
            .verified_script_classes()
            .iter()
            .any(|class| class.assembly_id == assembly_id && class.class_name == class_name)
        {
            return Err(format!(
                "'{class_name}' is not in the reflection-verified class list for loaded assembly '{assembly_id}'; rebuild and reload scripts"
            ));
        }
        let editor_scene = self
            .editor_scene
            .as_ref()
            .ok_or_else(|| "No editor scene is open".to_string())?;
        let selected_id = editor_scene
            .selected_entity
            .as_ref()
            .ok_or_else(|| "Select an entity before adding a script".to_string())?;
        let entity = editor_scene
            .scene
            .entities
            .iter()
            .find(|entity| &entity.persistent_id == selected_id)
            .ok_or_else(|| format!("Selected entity '{selected_id}' no longer exists"))?;
        if entity.components.contains_key("engine.script") {
            return Err(format!(
                "Entity '{}' already has an engine.script component",
                entity.persistent_id
            ));
        }
        let component = ComponentRecord {
            schema_version: SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: std::collections::BTreeMap::from([
                ("assembly_id".into(), Value::Str(assembly_id.to_string())),
                ("class_name".into(), Value::Str(class_name.to_string())),
            ]),
        };
        Ok(Box::new(engine_editor::AddComponent::new(
            entity.persistent_id.clone(),
            "engine.script".to_string(),
            component,
        )))
    }
}
