use super::*;

impl EditorApp {
    pub(super) fn render_react_frame(&mut self) -> EditorFrameOutcome {
        if self.editor_scene.is_none() || self.game_loop.is_none() {
            return EditorFrameOutcome::Completed;
        }
        let editor_job_completed = self.poll_editor_job();
        let editor_build_completed = self.poll_editor_build();
        if editor_job_completed || editor_build_completed {
            self.editor_revision = self.editor_revision.wrapping_add(1);
            self.pending_full_snapshot = true;
        }
        self.maybe_write_recovery_snapshot();
        if let Some(game_loop) = self.game_loop.as_ref() {
            engine_editor::animation_preview::refresh_animation_assets(
                &mut self.animation_preview,
                game_loop.runtime.asset_registry(),
            );
        }

        self.process_material_save();
        self.process_gizmo_inputs();

        let now = Instant::now();
        let delta_seconds = now
            .duration_since(self.last_frame_time)
            .as_secs_f32()
            .min(0.1);
        self.last_frame_time = now;
        engine_editor::animation_preview::update_preview(
            &mut self.animation_preview,
            delta_seconds,
        );
        self.tick_web_viewport_camera(delta_seconds);
        if self.play_session.is_editing() && self.viewport_tab == ViewportTab::Scene {
            if let Some(game_loop) = self.game_loop.as_ref() {
                let _ = apply_editor_camera(&game_loop.runtime, &self.scene_view);
            }
        }
        self.tick_editor_play_mode(delta_seconds);

        let Some((interaction_min, interaction_max, render_viewport)) = editor_render_viewport(
            self.web_viewport_rect,
            self.window_scale_factor,
            Vec2::new(self.window_w, self.window_h),
        ) else {
            self.frame = self.frame.wrapping_add(1);
            return EditorFrameOutcome::Completed;
        };
        let gizmo_batch = if gizmo_viewport_enabled(
            self.workspace_preferences.gizmos_visible,
            self.play_session.is_editing(),
            self.viewport_tab,
        ) {
            self.editor_scene.as_ref().and_then(|editor_scene| {
                let selected = editor_scene.selected_entity.as_deref()?;
                let game_loop = self.game_loop.as_ref()?;
                let view =
                    runtime_gizmo_view(&game_loop.runtime, selected, self.frame, render_viewport)?;
                let view = restrict_gizmo_view_to_rect(view, interaction_min, interaction_max)?;
                build_gizmo_ui_batch(
                    &self.gizmo,
                    view.world_position,
                    view.world_rotation,
                    view.view,
                    view.projection,
                    view.viewport_size,
                )
                .map(|batch| offset_gizmo_batch(batch, view))
            })
        } else {
            None
        };

        let mut engine_overlay_batches = Vec::new();
        if let Some(batch) = gizmo_batch {
            engine_overlay_batches.push(batch);
        }
        let overlay_batch_count = engine_overlay_batches.len();
        let overlay_vertex_count = engine_overlay_batches
            .iter()
            .map(|batch: &UiBatch| batch.vertices.len())
            .sum::<usize>();
        let Some(game_loop) = self.game_loop.as_mut() else {
            return EditorFrameOutcome::Completed;
        };
        let outcome = match game_loop.render_embedded_viewport(
            self.frame,
            engine_overlay_batches,
            render_viewport,
        ) {
            Ok(stats) => {
                if self.render_faulted {
                    tracing::info!(frame = self.frame, "editor renderer recovered");
                }
                self.render_faulted = false;
                let _ = game_loop.runtime.with_world(|world| {
                    engine_editor::performance::record_frame(
                        &mut self.performance.frame_stats,
                        world,
                        Some(&stats),
                    );
                });
                self.performance.frame_stats.frame_time_ms =
                    editor_frame_time_ms(self.performance.frame_stats.frame_time_ms, delta_seconds);
                self.performance.frame_stats.asset_count = game_loop
                    .runtime
                    .asset_registry()
                    .cached_ids()
                    .len()
                    .try_into()
                    .unwrap_or(u32::MAX);
                self.performance.commit_frame();
                tracing::debug!(
                    frame = self.frame,
                    draw_calls = stats.draw_calls,
                    overlay_batches = overlay_batch_count,
                    overlay_vertices = overlay_vertex_count,
                    "React editor viewport frame"
                );
                EditorFrameOutcome::Completed
            }
            Err(diagnostics) => {
                if !self.render_faulted {
                    super::super::log_renderer_diagnostics("editor render", &diagnostics);
                }
                self.render_faulted = true;
                EditorFrameOutcome::Failed
            }
        };
        self.frame = self.frame.wrapping_add(1);
        outcome
    }
    pub(super) fn initialize_native_surface(
        &mut self,
        surface: platform::PlatformSurface,
        width: u32,
        height: u32,
        scale_factor: f64,
    ) -> Result<(), String> {
        self.surface_zero_sized = width == 0 || height == 0;
        self.surface_occluded = false;
        self.render_faulted = false;
        self.window_w = width.max(1) as f32;
        self.window_h = height.max(1) as f32;
        self.window_scale_factor = scale_factor;
        let backend = create_backend_renderer_for_surface(
            surface,
            width.max(1),
            height.max(1),
            std::env::var("ENGINE_VK_VALIDATION").is_ok(),
            None,
        )
        .map_err(|error| format!("Vulkan backend creation failed: {error}"))?;

        let mut game_loop = GameLoop::new(EngineConfig {
            application_name: format!("{} Editor", self.project.manifest.name),
            gpu_timestamps: true,
        });
        #[cfg(feature = "target-desktop")]
        {
            game_loop.input_map =
                super::super::project_input::load_project_input_map(&self.project)
                    .map_err(|error| format!("editor input actions failed to load: {error}"))?;
        }
        game_loop.runtime.set_renderer_backend(backend);
        self.game_loop = Some(game_loop);
        self.init_scene();
        tracing::info!("React editor host and Vulkan viewport initialized");
        Ok(())
    }

    pub(super) fn surface_render_suspended(&self) -> bool {
        self.surface_occluded || self.surface_zero_sized
    }

    pub(super) fn handle_native_surface_resize(
        &mut self,
        width: u32,
        height: u32,
        scale_factor: Option<f64>,
    ) -> HostDirective {
        if let Some(scale_factor) = scale_factor {
            self.window_scale_factor = scale_factor;
            self.web_viewport_rect = ScreenRect::default();
        }
        self.cancel_web_viewport_input();
        self.surface_zero_sized = width == 0 || height == 0;
        self.window_w = width as f32;
        self.window_h = height as f32;
        if self.surface_zero_sized {
            return HostDirective::Continue;
        }

        if let Some(game_loop) = self.game_loop.as_mut() {
            if let Err(diagnostics) = game_loop.runtime.resize_renderer(width, height) {
                if !self.render_faulted {
                    let operation = if scale_factor.is_some() {
                        "editor DPI resize"
                    } else {
                        "editor resize"
                    };
                    super::super::log_renderer_diagnostics(operation, &diagnostics);
                }
                self.render_faulted = true;
                return HostDirective::Continue;
            }
        }

        self.last_frame_time = Instant::now();
        if self.surface_occluded {
            HostDirective::Continue
        } else {
            HostDirective::RequestRedraw
        }
    }

    pub(super) fn handle_dropped_asset(&mut self, path: PathBuf) {
        if !self.play_session.is_editing() {
            self.record_build_error(
                "Import asset",
                "Stop Play mode before importing dropped files".to_string(),
            );
            return;
        }
        if self.background_job.is_some() {
            self.record_build_error(
                "Import asset",
                "Wait for the current asset operation to finish before dropping another file"
                    .to_string(),
            );
            return;
        }
        let mut asset_id = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("asset")
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                    character
                } else {
                    '_'
                }
            })
            .collect::<String>();
        if asset_id.is_empty()
            || !asset_id
                .chars()
                .next()
                .is_some_and(|character| character.is_ascii_alphanumeric())
        {
            asset_id.insert(0, 'a');
        }
        let project_path = self.project.manifest_path.clone();
        let source_path = path.clone();
        let imported_id = asset_id.clone();
        if let Err(error) = self.start_editor_job("Asset import", true, move || {
            super::super::project_cli::import_project_asset_from(
                project_path,
                source_path,
                asset_id,
                None,
                PathBuf::new(),
            )?;
            Ok(EditorJobOutput::SelectAsset(imported_id))
        }) {
            self.record_build_error("Asset import", error);
            return;
        }
        self.request_ui_open_panel(protocol::UiPanel::Project, protocol::UiDockZone::Bottom);
        self.build_status = Some(format!("Importing dropped asset '{}'.", path.display()));
    }

    pub(super) fn project_changed_json(&mut self) -> String {
        self.pending_full_snapshot = false;
        self.editor_event_sequence = self.editor_event_sequence.wrapping_add(1);
        serde_json::to_string(&protocol::BridgeEvent {
            protocol: protocol::EDITOR_PROTOCOL,
            session_id: self.session_id.clone(),
            sequence: self.editor_event_sequence,
            revision: self.editor_revision,
            event: protocol::PROJECT_CHANGED_EVENT,
            params: self.editor_snapshot(),
        })
        .expect("editor snapshots must serialize")
    }

    pub(super) fn telemetry_json(&mut self) -> String {
        self.editor_event_sequence = self.editor_event_sequence.wrapping_add(1);
        serde_json::to_string(&protocol::BridgeEvent {
            protocol: protocol::EDITOR_PROTOCOL,
            session_id: self.session_id.clone(),
            sequence: self.editor_event_sequence,
            revision: self.editor_revision,
            event: protocol::TELEMETRY_EVENT,
            params: self.editor_telemetry(),
        })
        .expect("editor telemetry must serialize")
    }

    pub(super) fn take_frame_bridge_messages(&mut self, periodic_telemetry: bool) -> Vec<String> {
        let mut messages = if self.pending_full_snapshot {
            vec![self.project_changed_json()]
        } else if periodic_telemetry {
            vec![self.telemetry_json()]
        } else {
            Vec::new()
        };
        messages.extend(self.take_ui_open_panel_events_json());
        messages
    }

    pub(super) fn handle_close_requested(&mut self) -> bool {
        if !self.play_session.is_editing() {
            self.stop_play();
            if !self.play_session.is_editing() {
                self.scene_document_status = Some(
                    "Could not close while Play mode failed to restore the authoring scene. Retry Stop or cancel Play errors first."
                        .to_string(),
                );
                return false;
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
            self.editor_revision = self.editor_revision.wrapping_add(1);
            false
        } else {
            true
        }
    }
}
