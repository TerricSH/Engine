use super::*;

#[cfg(feature = "backend-vulkan")]
pub(super) fn run_windowed(
    project: GameProject,
    scene: Scene,
    max_frames: Option<u64>,
    stream_cells: bool,
) -> Result<(), String> {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Instant;

    use platform::{EventFlow, PlatformEvent, PlatformWindow, WindowApp, WindowDescriptor};

    struct ProjectPlayerApp {
        project: GameProject,
        scene: Scene,
        game_loop: Option<GameLoop>,
        cell_streaming: Option<CellStreamingDriver>,
        stream_cells: bool,
        frame: u64,
        max_frames: Option<u64>,
        previous_frame: Instant,
        current_scene_id: String,
        failed: Arc<AtomicBool>,
        #[cfg(feature = "target-desktop")]
        input_state: crate::project_input::ProjectInputState,
    }

    impl ProjectPlayerApp {
        fn fail(&self, message: impl std::fmt::Display) -> EventFlow {
            tracing::error!(error = %message, "game project player failed");
            self.failed.store(true, Ordering::Release);
            EventFlow::Exit
        }
    }

    impl WindowApp for ProjectPlayerApp {
        fn on_create(&mut self, window: &PlatformWindow) {
            let (width, height) = window.size();
            let surface = match window.surface() {
                Ok(surface) => surface,
                Err(error) => {
                    self.failed.store(true, Ordering::Release);
                    tracing::error!(%error, "could not acquire platform surface");
                    return;
                }
            };
            let backend = match render_vulkan::create_backend_renderer_for_surface(
                surface,
                width.max(1),
                height.max(1),
                std::env::var("ENGINE_VK_VALIDATION").is_ok(),
                None,
            ) {
                Ok(backend) => backend,
                Err(error) => {
                    self.failed.store(true, Ordering::Release);
                    tracing::error!(%error, "could not create Vulkan project renderer");
                    return;
                }
            };

            match create_game_loop(&self.project, self.scene.clone()) {
                Ok((mut game_loop, _)) => {
                    game_loop.runtime.set_renderer_backend(backend);
                    #[cfg(feature = "runtime-subsystems")]
                    game_loop.set_ui_viewport_size(width, height);
                    match create_cell_streaming_driver(&self.project, self.stream_cells) {
                        Ok(mut driver) => {
                            if let Some(driver) = driver.as_mut() {
                                driver.rebaseline(&game_loop.runtime);
                            }
                            self.cell_streaming = driver;
                        }
                        Err(error) => {
                            self.failed.store(true, Ordering::Release);
                            tracing::error!(%error, "cell streaming setup failed");
                            return;
                        }
                    }
                    let initial_transitions = match process_pending_scene_transitions(
                        &mut game_loop,
                        &self.project,
                        &mut self.current_scene_id,
                    ) {
                        Ok(transitions) => transitions,
                        Err(error) => {
                            self.failed.store(true, Ordering::Release);
                            tracing::error!(%error, "initial scene transition failed");
                            return;
                        }
                    };
                    game_loop.tick_world_origin_shift();
                    tick_cell_streaming(
                        &mut game_loop,
                        &mut self.cell_streaming,
                        initial_transitions,
                    );
                    self.game_loop = Some(game_loop);
                    self.previous_frame = Instant::now();
                    window.request_redraw();
                }
                Err(error) => {
                    self.failed.store(true, Ordering::Release);
                    tracing::error!(%error, "could not initialize game project");
                }
            }
        }

        fn on_event(&mut self, window: &PlatformWindow, event: PlatformEvent) -> EventFlow {
            #[cfg(feature = "target-desktop")]
            if let Some(game_loop) = self.game_loop.as_mut() {
                self.input_state
                    .apply_platform_event(&mut game_loop.input_map, &event);
            }
            #[cfg(feature = "runtime-subsystems")]
            if let Some(game_loop) = self.game_loop.as_mut() {
                route_project_player_ui_event(game_loop, &event);
            }
            match event {
                PlatformEvent::Redraw => {
                    if self.failed.load(Ordering::Acquire) {
                        return EventFlow::Exit;
                    }
                    let now = Instant::now();
                    let dt = now
                        .duration_since(self.previous_frame)
                        .as_secs_f32()
                        .clamp(0.0, 0.1);
                    self.previous_frame = now;
                    let Some(game_loop) = self.game_loop.as_mut() else {
                        return self.fail("renderer was not initialized");
                    };
                    game_loop.update(dt);
                    if let Err(error) =
                        crate::project_scripts::fail_on_script_errors(&game_loop.runtime, "update")
                    {
                        return self.fail(error);
                    }
                    let frame_transitions = match process_pending_scene_transitions(
                        game_loop,
                        &self.project,
                        &mut self.current_scene_id,
                    ) {
                        Ok(transitions) => transitions,
                        Err(error) => return self.fail(error),
                    };
                    game_loop.tick_world_origin_shift();
                    tick_cell_streaming(game_loop, &mut self.cell_streaming, frame_transitions);
                    if let Err(diagnostics) = game_loop.render(self.frame) {
                        return self.fail(format_diagnostics(diagnostics));
                    }
                    self.frame += 1;
                    if self.max_frames.is_some_and(|limit| self.frame >= limit) {
                        return EventFlow::Exit;
                    }
                    window.request_redraw();
                    EventFlow::Continue
                }
                PlatformEvent::Resized { width, height } => {
                    if let Some(game_loop) = self.game_loop.as_mut() {
                        #[cfg(feature = "runtime-subsystems")]
                        game_loop.set_ui_viewport_size(width, height);
                        if let Err(diagnostics) = game_loop.runtime.resize_renderer(width, height) {
                            return self.fail(format_diagnostics(diagnostics));
                        }
                    }
                    EventFlow::Continue
                }
                PlatformEvent::CloseRequested => EventFlow::Exit,
                _ => EventFlow::Continue,
            }
        }
    }

    let failed = Arc::new(AtomicBool::new(false));
    let app = ProjectPlayerApp {
        project: project.clone(),
        scene,
        game_loop: None,
        cell_streaming: None,
        stream_cells,
        frame: 0,
        max_frames,
        previous_frame: Instant::now(),
        current_scene_id: project.startup_scene_id().to_string(),
        failed: Arc::clone(&failed),
        #[cfg(feature = "target-desktop")]
        input_state: crate::project_input::ProjectInputState::default(),
    };
    platform::run(
        WindowDescriptor {
            title: project.manifest.window.title.clone(),
            width: project.manifest.window.width,
            height: project.manifest.window.height,
        },
        app,
    )
    .map_err(|error| format!("platform run failed: {error}"))?;
    if failed.load(Ordering::Acquire) {
        return Err("project player stopped after a runtime failure".into());
    }
    Ok(())
}

#[cfg(not(feature = "backend-vulkan"))]
pub(super) fn run_windowed(
    _project: GameProject,
    _scene: Scene,
    _max_frames: Option<u64>,
    _stream_cells: bool,
) -> Result<(), String> {
    Err("windowed project run requires the `backend-vulkan` feature; use --headless or rebuild with Vulkan support".into())
}
