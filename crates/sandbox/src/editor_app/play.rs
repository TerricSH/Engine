use super::*;

impl EditorApp {
    pub(super) fn init_scene(&mut self) {
        let scene = match Scene::load_from_file(&self.current_scene_path) {
            Ok(scene) => scene,
            Err(error) => {
                tracing::error!(
                    scene_id = self.current_scene_id,
                    path = %self.current_scene_path.display(),
                    %error,
                    "editor: failed to load project startup scene"
                );
                std::process::exit(1);
            }
        };
        let mut editor_diagnostics = Vec::new();
        if let Some(ref mut game_loop) = self.game_loop {
            if let Err(error) = super::super::project_scripts::validate_runtime_script_references(
                &self.project,
                &scene,
            ) {
                tracing::error!(%error, "editor: invalid project script references");
                std::process::exit(1);
            }
            if let Err(error) = super::super::project_app::load_project_assets(
                &mut game_loop.runtime,
                &self.project,
            ) {
                tracing::error!(%error, "editor: failed to load project cooked assets");
                std::process::exit(1);
            }
            let requested_asset_folder = self.workspace_preferences.project_asset_folder.clone();
            if let Err(error) = refresh_project_asset_list(
                &mut self.asset_browser,
                game_loop.runtime.asset_registry(),
                &self.project.asset_source,
            ) {
                let message = format!("Could not load the project asset catalog: {error}");
                tracing::error!(%message);
                self.build_status = Some(message);
            } else {
                self.asset_browser
                    .set_current_folder(requested_asset_folder);
                self.workspace_preferences.project_asset_folder =
                    self.asset_browser.current_folder().to_string();
            }
            if self.project.script_project.is_some() {
                let message =
                    "Project scripts are not executed while opening a workspace. Use Rebuild \
                     Scripts, Play, or Build when you are ready to trust and compile this project's \
                     C# code.";
                tracing::info!("{message}");
                editor_diagnostics.push(Diagnostic::new(
                    "EDSCRIPT_BUILD_REQUIRED",
                    DiagnosticSeverity::Info,
                    "editor.workspace",
                    message,
                ));
            }
            let (preview_scene, missing_diagnostics) =
                editor_preview_scene(&game_loop.runtime, &scene);
            editor_diagnostics.extend(missing_diagnostics);
            if let Err(diagnostics) = game_loop.load_scene(preview_scene) {
                for diagnostic in diagnostics {
                    tracing::error!(
                        code = diagnostic.code,
                        entity = ?diagnostic.entity,
                        component_type = ?diagnostic.fields.get("component_type_id"),
                        message = diagnostic.message,
                        "editor: failed to load project scene"
                    );
                }
                std::process::exit(1);
            }
            if let Err(error) = super::super::project_scripts::fail_on_script_errors(
                &game_loop.runtime,
                "attachment/OnCreate",
            ) {
                tracing::error!(%error, "editor: script startup failed");
                std::process::exit(1);
            }
            game_loop.init_physics();
        }
        let component_registry = self
            .game_loop
            .as_ref()
            .map(|game_loop| std::sync::Arc::clone(game_loop.runtime.component_registry()))
            .unwrap_or_else(|| {
                tracing::error!("editor: runtime component registry is unavailable");
                std::process::exit(1);
            });
        let mut editor_scene =
            match EditorScene::new_with_component_registry(scene, component_registry) {
                Ok(editor_scene) => editor_scene,
                Err(error) => {
                    tracing::error!(%error, "editor: startup scene is not authorable");
                    std::process::exit(1);
                }
            };
        editor_scene.diagnostics.push_many(editor_diagnostics);
        self.scene_settings_draft = editor_scene.scene.scene_settings.clone();
        self.editor_scene = Some(editor_scene);
        self.selected_entity_ids.clear();
        self.last_frame_time = Instant::now();
        tracing::info!(
            project = self.project.manifest.name,
            scene_id = self.current_scene_id,
            scene = %self.current_scene_path.display(),
            "editor: project scene loaded"
        );
    }

    pub(super) fn start_play(&mut self) {
        let Some(authoring_scene) = self
            .editor_scene
            .as_ref()
            .map(|editor_scene| editor_scene.scene.clone())
        else {
            return;
        };
        let Some(game_loop) = self.game_loop.as_mut() else {
            return;
        };

        let refreshed_scripts =
            match super::super::project_scripts::rebuild_and_reload_project_scripts(
                &mut game_loop.runtime,
                &self.project,
            ) {
                Ok(prepared) => prepared,
                Err(error) => {
                    let diagnostic = Diagnostic::new(
                        "EDPLAY_SCRIPT_REBUILD_FAILED",
                        DiagnosticSeverity::Error,
                        "editor.play-mode",
                        format!(
                            "Play mode did not start because C# scripts failed to rebuild: {error}"
                        ),
                    );
                    tracing::error!(%error, "editor Play script rebuild failed");
                    if let Some(editor_scene) = self.editor_scene.as_mut() {
                        editor_scene.diagnostics.push(diagnostic);
                    }
                    self.scene_document_status =
                        Some("Play cancelled: C# script rebuild failed.".to_string());
                    return;
                }
            };
        tracing::info!(
            assemblies = refreshed_scripts.assemblies,
            "editor Play scripts refreshed"
        );

        match self.play_session.start(&authoring_scene, |scene| {
            let missing = super::super::project_app::missing_render_asset_dependencies(
                &game_loop.runtime,
                &scene,
            );
            if !missing.is_empty() {
                return Err(missing
                    .into_iter()
                    .map(|asset| {
                        let mut diagnostic = Diagnostic::new(
                            "EDPLAY_MISSING_ASSET",
                            DiagnosticSeverity::Error,
                            "editor.play-mode",
                            format!(
                                "Play mode cannot start while render asset '{}' is missing",
                                asset.id
                            ),
                        );
                        diagnostic.asset = Some(asset);
                        diagnostic
                    })
                    .collect());
            }
            game_loop.runtime.diagnostics_collector_mut().clear_frame();
            game_loop.load_scene(scene)?;
            let script_errors = game_loop
                .runtime
                .diagnostics_collector()
                .script_diagnostics
                .iter()
                .filter(|diagnostic| {
                    matches!(
                        diagnostic.severity,
                        DiagnosticSeverity::Error | DiagnosticSeverity::Fatal
                    )
                })
                .cloned()
                .collect::<Vec<_>>();
            if script_errors.is_empty() {
                Ok(())
            } else {
                Err(script_errors)
            }
        }) {
            Ok(true) => {
                self.viewport_tab = ViewportTab::Game;
                let mut runtime_scene_id = self.current_scene_id.clone();
                game_loop.init_physics();
                #[cfg(feature = "target-desktop")]
                self.input_state.reset(&mut game_loop.input_map);
                self.last_frame_time = Instant::now();
                match super::super::project_app::process_pending_scene_transitions(
                    game_loop,
                    &self.project,
                    &mut runtime_scene_id,
                ) {
                    Ok(transitions) => {
                        self.play_runtime_scene_id = Some(runtime_scene_id.clone());
                        tracing::info!(
                            transitions,
                            scene_id = runtime_scene_id,
                            "editor: Play mode started"
                        );
                    }
                    Err(error) => {
                        tracing::error!(%error, "editor Play startup scene transition failed");
                        let diagnostics = recover_play_after_scene_transition_error(
                            &mut self.play_session,
                            game_loop,
                            error,
                        );
                        log_scene_diagnostics(
                            "editor Play stopped after startup scene transition failure",
                            diagnostics.clone(),
                        );
                        if let Some(editor_scene) = self.editor_scene.as_mut() {
                            editor_scene.diagnostics.push_many(diagnostics);
                        }
                        self.play_runtime_scene_id = None;
                        #[cfg(feature = "target-desktop")]
                        self.input_state.reset(&mut game_loop.input_map);
                    }
                }
            }
            Ok(false) => {}
            Err(mut diagnostics) => {
                self.play_runtime_scene_id = None;
                log_scene_diagnostics("editor Play start failed", diagnostics.clone());
                match restore_editor_preview(game_loop, &authoring_scene) {
                    Ok(warnings) => diagnostics.extend(warnings),
                    Err(rollback_diagnostics) => {
                        log_scene_diagnostics(
                            "editor Play rollback failed",
                            rollback_diagnostics.clone(),
                        );
                        diagnostics.extend(rollback_diagnostics);
                    }
                }
                if let Some(editor_scene) = self.editor_scene.as_mut() {
                    editor_scene.diagnostics.push_many(diagnostics);
                }
            }
        }
        if self.play_session.is_editing() {
            self.request_ui_open_panel(protocol::UiPanel::Console, protocol::UiDockZone::Bottom);
        } else {
            self.request_ui_open_panel(protocol::UiPanel::Game, protocol::UiDockZone::Center);
        }
    }

    pub(super) fn pause_play(&mut self) {
        if self.play_session.pause() {
            tracing::info!("editor: Play mode paused");
        }
    }

    pub(super) fn resume_play(&mut self) {
        if self.play_session.resume() {
            self.last_frame_time = Instant::now();
            tracing::info!("editor: Play mode resumed");
        }
    }

    pub(super) fn step_play(&mut self) {
        if self.play_session.mode() == EditorPlayMode::Paused {
            self.step_play_once = true;
            tracing::info!("editor: Play mode scheduled one fixed simulation step");
        }
    }

    pub(super) fn request_editor_exit(&mut self) {
        if !self.play_session.is_editing() {
            self.stop_play();
            if !self.play_session.is_editing() {
                self.scene_document_status = Some(
                    "Could not close while Play mode failed to restore the authoring scene. Retry Stop first."
                        .to_string(),
                );
                return;
            }
        }
        let has_unsaved_changes = self.gizmo.dragging
            || self
                .editor_scene
                .as_ref()
                .is_some_and(|scene| scene.is_dirty() || scene.is_transform_gizmo_drag_active());
        if has_unsaved_changes {
            self.pending_scene_switch = None;
            self.pending_document_action = None;
            self.close_confirmation_pending = true;
            self.scene_document_status = Some(
                "Unsaved changes: choose Save & Close, Discard & Close, or Cancel Close."
                    .to_string(),
            );
        } else {
            self.exit_after_frame = true;
        }
    }

    pub(super) fn stop_play(&mut self) {
        let Some(game_loop) = self.game_loop.as_mut() else {
            return;
        };
        match self.play_session.stop(|scene| {
            let (preview_scene, _) = editor_preview_scene(&game_loop.runtime, &scene);
            game_loop.load_scene(preview_scene)
        }) {
            Ok(true) => {
                self.play_runtime_scene_id = None;
                self.viewport_tab = ViewportTab::Scene;
                game_loop.init_physics();
                #[cfg(feature = "target-desktop")]
                self.input_state.reset(&mut game_loop.input_map);
                self.last_frame_time = Instant::now();
                tracing::info!("editor: Play mode stopped; authoring scene restored");
            }
            Ok(false) => {
                self.play_runtime_scene_id = None;
            }
            Err(diagnostics) => {
                self.play_runtime_scene_id = None;
                log_scene_diagnostics("editor Play stop failed", diagnostics);
            }
        }
        if self.play_session.is_editing() {
            self.request_ui_open_panel(protocol::UiPanel::Scene, protocol::UiDockZone::Center);
        } else {
            self.request_ui_open_panel(protocol::UiPanel::Console, protocol::UiDockZone::Bottom);
        }
    }

    pub(super) fn tick_editor_play_mode(&mut self, delta_seconds: f32) {
        let stepping = std::mem::take(&mut self.step_play_once)
            && self.play_session.mode() == EditorPlayMode::Paused;
        if !self.play_session.should_tick() && !stepping {
            return;
        }
        let (Some(game_loop), Some(editor_scene)) =
            (self.game_loop.as_mut(), self.editor_scene.as_mut())
        else {
            return;
        };
        game_loop.update(if stepping { 1.0 / 60.0 } else { delta_seconds });
        if let Err(error) =
            super::super::project_scripts::fail_on_script_errors(&game_loop.runtime, "update")
        {
            let diagnostics =
                recover_play_after_script_error(&mut self.play_session, game_loop, error);
            editor_scene.diagnostics.push_many(diagnostics);
            self.play_runtime_scene_id = None;
            #[cfg(feature = "target-desktop")]
            self.input_state.reset(&mut game_loop.input_map);
            return;
        }
        let Some(runtime_scene_id) = self.play_runtime_scene_id.as_mut() else {
            return;
        };
        if let Err(error) = super::super::project_app::process_pending_scene_transitions(
            game_loop,
            &self.project,
            runtime_scene_id,
        ) {
            let diagnostics =
                recover_play_after_scene_transition_error(&mut self.play_session, game_loop, error);
            editor_scene.diagnostics.push_many(diagnostics);
            self.play_runtime_scene_id = None;
            #[cfg(feature = "target-desktop")]
            self.input_state.reset(&mut game_loop.input_map);
        }
    }
}
