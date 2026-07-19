use std::path::Path;

use engine_asset::partition::WorldPartition;
use engine_asset::project::GameProject;
use engine_core::cell_stream::{CellStreamingConfig, CellStreamingDriver};
use engine_core::game_loop::GameLoop;
use engine_core::{CookedAssetLoadReport, EngineConfig, EngineRuntime, SceneLoadRequest};
use engine_scene::Scene;

use crate::project_cli::ProjectRunRequest;

pub fn run_project(request: ProjectRunRequest) -> Result<(), String> {
    // Authoring runs rebuild managed scripts before switching to the runtime
    // view of the project. Packaged projects intentionally have no source
    // asset/script requirements and skip this branch.
    if let Ok(authoring_project) = GameProject::load(&request.project) {
        if authoring_project.script_project.is_some() {
            crate::project_scripts::build_project_scripts(&authoring_project)?;
        }
    }
    let project = GameProject::load_runtime(&request.project).map_err(|error| error.to_string())?;
    let scene = load_startup_scene(&project.startup_scene)?;
    if request.headless {
        run_headless(
            project,
            scene,
            request.frames.unwrap_or(3),
            request.report.as_deref(),
            request.stream_cells,
        )
    } else {
        run_windowed(project, scene, request.frames, request.stream_cells)
    }
}

const MAX_CHAINED_SCENE_TRANSITIONS: usize = 8;

#[cfg(all(feature = "runtime-subsystems", feature = "backend-vulkan"))]
fn route_project_player_ui_event(
    game_loop: &mut GameLoop,
    event: &platform::PlatformEvent,
) -> bool {
    match event {
        platform::PlatformEvent::MouseMoved { x, y } => {
            game_loop.ui_pointer_move(*x as f32, *y as f32);
            true
        }
        platform::PlatformEvent::MousePressed { button, x, y }
            if *button == platform::MouseButton::Left =>
        {
            game_loop.ui_pointer_move(*x as f32, *y as f32);
            game_loop.ui_pointer_left_press();
            true
        }
        platform::PlatformEvent::MouseReleased { button, x, y }
            if *button == platform::MouseButton::Left =>
        {
            game_loop.ui_pointer_move(*x as f32, *y as f32);
            game_loop.ui_pointer_left_release();
            true
        }
        platform::PlatformEvent::Focused(false) | platform::PlatformEvent::Suspended => {
            game_loop.cancel_ui_pointer();
            true
        }
        _ => false,
    }
}

fn load_startup_scene(path: &Path) -> Result<Scene, String> {
    Scene::load_from_file(path)
        .map_err(|error| format!("could not load startup scene {}: {error}", path.display()))
}

fn create_game_loop(
    project: &GameProject,
    scene: Scene,
) -> Result<(GameLoop, CookedAssetLoadReport), String> {
    let expected_script_instances =
        crate::project_scripts::validate_runtime_script_references(project, &scene)?;
    let input_map = super::project_input::load_project_input_map(project)?;
    let mut game_loop = GameLoop::new(EngineConfig {
        application_name: project.manifest.name.clone(),
        gpu_timestamps: true,
    });
    #[cfg(any(feature = "target-desktop", feature = "subsystem-scripting-csharp"))]
    {
        game_loop.input_map = input_map;
    }
    #[cfg(not(any(feature = "target-desktop", feature = "subsystem-scripting-csharp")))]
    let _ = input_map;
    let cooked_report = prepare_project_runtime(&mut game_loop.runtime, project, &scene)?;
    let prepared_scripts =
        crate::project_scripts::prepare_project_scripts(&mut game_loop.runtime, project)?;
    game_loop.load_scene(scene).map_err(format_diagnostics)?;
    crate::project_scripts::fail_on_script_errors(&game_loop.runtime, "attachment/OnCreate")?;
    let (_, attached_script_instances, _) =
        crate::project_scripts::script_runtime_counts(&game_loop.runtime);
    if attached_script_instances != expected_script_instances {
        return Err(format!(
            "script attachment count mismatch: expected {expected_script_instances}, attached {attached_script_instances}"
        ));
    }
    tracing::info!(
        assemblies = prepared_scripts.assemblies,
        instances = attached_script_instances,
        "project scripts prepared"
    );
    game_loop.init_physics();
    game_loop.validate_ready().map_err(format_diagnostics)?;
    Ok((game_loop, cooked_report))
}

/// Build the world-partition cell streaming driver when `--stream-cells` is
/// set. The flag requires a `world.partition.json` at the project root;
/// without the flag no driver is constructed and streaming stays off.
fn create_cell_streaming_driver(
    project: &GameProject,
    stream_cells: bool,
) -> Result<Option<CellStreamingDriver>, String> {
    if !stream_cells {
        return Ok(None);
    }
    let partition = WorldPartition::load_for_project(project)
        .map_err(|error| format!("world partition validation failed: {error}"))?
        .ok_or_else(|| {
            "--stream-cells requires a world.partition.json at the project root".to_string()
        })?;
    CellStreamingDriver::new(&partition, project, CellStreamingConfig::default())
        .map(Some)
        .map_err(|error| format!("world partition cell streaming setup failed: {error}"))
}

/// Advance cell streaming at the frame boundary: rebaseline after scene
/// transitions, tick the driver, and re-sync physics when the world changed.
fn tick_cell_streaming(
    game_loop: &mut GameLoop,
    driver: &mut Option<CellStreamingDriver>,
    scene_transitions: usize,
) {
    let Some(driver) = driver.as_mut() else {
        return;
    };
    if scene_transitions > 0 {
        driver.rebaseline(&game_loop.runtime);
    }
    let report = driver.tick(&mut game_loop.runtime);
    if report.world_changed() {
        game_loop.resync_physics_from_world();
    }
}

fn transition_to_project_scene(
    game_loop: &mut GameLoop,
    project: &GameProject,
    request: &SceneLoadRequest,
) -> Result<(), String> {
    let scene_path = project.scene_path(&request.scene_id).ok_or_else(|| {
        format!(
            "script entity '{}' requested unknown scene '{}'; available scenes: {}",
            request.requested_by,
            request.scene_id,
            project
                .scenes()
                .into_iter()
                .map(|(id, _)| id)
                .collect::<Vec<_>>()
                .join(", ")
        )
    })?;
    let scene = Scene::load_from_file(&scene_path).map_err(|error| {
        format!(
            "could not load requested scene '{}' from {}: {error}",
            request.scene_id,
            scene_path.display()
        )
    })?;
    let expected_script_instances = crate::project_scripts::validate_runtime_script_references(
        project, &scene,
    )
    .map_err(|error| format!("requested scene '{}' is invalid: {error}", request.scene_id))?;
    let missing = missing_runtime_asset_dependencies(&game_loop.runtime, &scene);
    if !missing.is_empty() {
        return Err(format!(
            "requested scene '{}' references assets unavailable at runtime: {}",
            request.scene_id,
            missing
                .into_iter()
                .map(|asset| asset.id)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    let previous_scene = capture_scene_transition_rollback(&game_loop.runtime);
    game_loop.runtime.diagnostics_collector_mut().clear_frame();
    game_loop.load_scene(scene).map_err(format_diagnostics)?;
    let transition_validation = crate::project_scripts::fail_on_script_errors(
        &game_loop.runtime,
        &format!("scene '{}' attachment/OnCreate", request.scene_id),
    )
    .and_then(|()| {
        let (_, attached_script_instances, _) =
            crate::project_scripts::script_runtime_counts(&game_loop.runtime);
        if attached_script_instances == expected_script_instances {
            Ok(())
        } else {
            Err(format!(
                "requested scene '{}' attached {attached_script_instances} script instances; expected {expected_script_instances}",
                request.scene_id
            ))
        }
    })
    .and_then(|()| game_loop.validate_ready().map_err(format_diagnostics));

    match transition_validation {
        Ok(()) => Ok(()),
        Err(error) => rollback_failed_scene_transition(game_loop, previous_scene, error),
    }
}

fn capture_scene_transition_rollback(runtime: &EngineRuntime) -> Option<Scene> {
    let retained = runtime.scene_ref()?.clone();
    let mut snapshot = runtime
        .with_world(|world| world.to_scene())
        .unwrap_or_else(|| retained.clone());

    // `engine.script` is intentionally scene-only metadata and is removed
    // before building the World. Merge it back onto the live ECS snapshot so
    // a failed transition restores both current transforms/entities and the
    // script attachment contract of every surviving entity.
    let script_components = retained
        .entities
        .iter()
        .filter_map(|entity| {
            entity
                .components
                .get("engine.script")
                .cloned()
                .map(|component| (entity.persistent_id.clone(), component))
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    for entity in &mut snapshot.entities {
        if let Some(component) = script_components.get(&entity.persistent_id) {
            entity
                .components
                .insert("engine.script".to_string(), component.clone());
        }
    }
    Some(snapshot)
}

fn rollback_failed_scene_transition(
    game_loop: &mut GameLoop,
    previous_scene: Option<Scene>,
    transition_error: String,
) -> Result<(), String> {
    let Some(previous_scene) = previous_scene else {
        return Err(transition_error);
    };
    game_loop.runtime.diagnostics_collector_mut().clear_frame();
    let rollback = game_loop
        .load_scene(previous_scene)
        .map_err(format_diagnostics)
        .and_then(|()| {
            crate::project_scripts::fail_on_script_errors(
                &game_loop.runtime,
                "previous scene attachment/OnCreate during transition rollback",
            )
        })
        .and_then(|()| game_loop.validate_ready().map_err(format_diagnostics));
    match rollback {
        Ok(()) => Err(format!(
            "{transition_error}; the previous scene was restored after the failed transition"
        )),
        Err(rollback_error) => Err(format!(
            "{transition_error}; restoring the previous scene also failed: {rollback_error}"
        )),
    }
}

pub(crate) fn process_pending_scene_transitions(
    game_loop: &mut GameLoop,
    project: &GameProject,
    current_scene_id: &mut String,
) -> Result<usize, String> {
    let mut transitions = 0usize;
    for _ in 0..MAX_CHAINED_SCENE_TRANSITIONS {
        let Some(request) = game_loop.runtime.take_pending_scene_request() else {
            return Ok(transitions);
        };
        if request.scene_id == *current_scene_id {
            tracing::warn!(
                scene = request.scene_id,
                entity = request.requested_by,
                "ignored request to reload the already-active scene"
            );
            continue;
        }
        transition_to_project_scene(game_loop, project, &request)?;
        *current_scene_id = request.scene_id;
        transitions += 1;
    }
    if let Some(request) = game_loop.runtime.take_pending_scene_request() {
        return Err(format!(
            "scene transition chain exceeded {MAX_CHAINED_SCENE_TRANSITIONS} loads in one frame; latest request was '{}' from '{}'",
            request.scene_id, request.requested_by
        ));
    }
    Ok(transitions)
}

pub(crate) fn prepare_project_runtime(
    runtime: &mut EngineRuntime,
    project: &GameProject,
    scene: &Scene,
) -> Result<CookedAssetLoadReport, String> {
    let cooked_report = load_project_assets(runtime, project)?;
    let missing_assets = missing_runtime_asset_dependencies(runtime, scene);
    if !missing_assets.is_empty() {
        return Err(format!(
            "scene references runtime assets that are neither built-in nor present in {}: {}",
            project.cooked_assets.display(),
            missing_assets
                .into_iter()
                .map(|asset| asset.id)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    Ok(cooked_report)
}

/// Load every runtime-supported cooked asset without rejecting unresolved
/// scene references.
///
/// The player calls the strict wrapper above; the editor uses this entry point
/// so a broken authoring reference can still be opened and repaired.
pub(crate) fn load_project_assets(
    runtime: &mut EngineRuntime,
    project: &GameProject,
) -> Result<CookedAssetLoadReport, String> {
    let cooked_report = runtime
        .load_cooked_assets(&project.cooked_assets)
        .map_err(format_diagnostics)?;
    tracing::info!(
        discovered = cooked_report.discovered_assets,
        loaded_meshes = cooked_report.loaded_meshes,
        loaded_textures = cooked_report.loaded_textures,
        loaded_materials = cooked_report.loaded_materials,
        loaded_extensions = cooked_report.loaded_extension_assets(),
        skipped = cooked_report.skipped_assets.len(),
        "project cooked assets loaded"
    );
    Ok(cooked_report)
}

#[cfg(all(feature = "tooling-editor", feature = "backend-vulkan"))]
pub(crate) fn missing_render_asset_dependencies(
    runtime: &EngineRuntime,
    scene: &Scene,
) -> Vec<engine_serialize::AssetId> {
    render_asset_dependencies(scene)
        .into_iter()
        .filter(|asset| !runtime.asset_registry().contains(asset))
        .collect()
}

fn missing_runtime_asset_dependencies(
    runtime: &EngineRuntime,
    scene: &Scene,
) -> Vec<engine_serialize::AssetId> {
    scene
        .collect_asset_dependencies()
        .into_iter()
        .filter(|asset| !runtime.asset_registry().contains(asset))
        .collect()
}

#[cfg(all(feature = "tooling-editor", feature = "backend-vulkan"))]
fn render_asset_dependencies(scene: &Scene) -> Vec<engine_serialize::AssetId> {
    let mut dependencies = scene
        .entities
        .iter()
        .filter_map(|entity| entity.components.get("engine.renderable"))
        .flat_map(|component| {
            ["mesh", "material"].into_iter().filter_map(move |field| {
                match component.fields.get(field) {
                    Some(engine_serialize::Value::Asset(asset)) => Some(asset.clone()),
                    _ => None,
                }
            })
        })
        .collect::<Vec<_>>();
    dependencies.sort();
    dependencies.dedup();
    dependencies
}

fn run_headless(
    project: GameProject,
    scene: Scene,
    frames: u64,
    report_path: Option<&Path>,
    stream_cells: bool,
) -> Result<(), String> {
    let (mut game_loop, cooked_report) = create_game_loop(&project, scene)?;
    game_loop
        .runtime
        .set_renderer_backend(Box::<crate::qa::QaBackend>::default());
    let mut cell_streaming = create_cell_streaming_driver(&project, stream_cells)?;
    if let Some(driver) = cell_streaming.as_mut() {
        driver.rebaseline(&game_loop.runtime);
    }
    let mut total_draw_calls = 0u64;
    let mut total_triangles = 0u64;
    let mut last_visible_drawables = 0u32;
    let mut current_scene_id = project.startup_scene_id().to_string();
    let mut scene_transitions =
        process_pending_scene_transitions(&mut game_loop, &project, &mut current_scene_id)?;
    game_loop.tick_world_origin_shift();
    tick_cell_streaming(&mut game_loop, &mut cell_streaming, scene_transitions);
    for frame in 0..frames {
        game_loop.update(1.0 / 60.0);
        crate::project_scripts::fail_on_script_errors(&game_loop.runtime, "update")?;
        let frame_transitions =
            process_pending_scene_transitions(&mut game_loop, &project, &mut current_scene_id)?;
        scene_transitions += frame_transitions;
        game_loop.tick_world_origin_shift();
        tick_cell_streaming(&mut game_loop, &mut cell_streaming, frame_transitions);
        let stats = game_loop.render(frame).map_err(format_diagnostics)?;
        total_draw_calls += u64::from(stats.draw_calls);
        total_triangles += stats.triangles;
        last_visible_drawables = stats.visible_drawables;
    }
    if total_draw_calls == 0 || last_visible_drawables == 0 {
        return Err(format!(
            "startup scene produced no visible drawables (draw_calls={total_draw_calls}, visible={last_visible_drawables})"
        ));
    }
    let (script_assemblies, script_instances, script_started_instances) =
        crate::project_scripts::script_runtime_counts(&game_loop.runtime);
    let script_update_count =
        crate::project_scripts::script_int_field_sum(&game_loop.runtime, "UpdateCount");
    let script_entity_translations =
        crate::project_scripts::script_entity_translations(&game_loop.runtime);
    let cell_streaming_report = cell_streaming.as_ref().map(|driver| {
        serde_json::json!({
            "enabled": true,
            "loaded_cells": driver.loaded_cells(),
            "total_merges": driver.total_merges(),
            "total_unloads": driver.total_unloads(),
            "resident_entities": driver.resident_ids().len(),
            "cell_states": driver
                .cell_states()
                .into_iter()
                .map(|(cell_id, state)| (cell_id, format!("{state:?}")))
                .collect::<std::collections::BTreeMap<_, _>>(),
        })
    });

    let report = serde_json::to_string_pretty(&serde_json::json!({
        "schema": "ProjectRunReport-v0",
        "project": project.manifest.name,
        "startup_scene_id": project.startup_scene_id(),
        "startup_scene": project.startup_scene_path().to_string_lossy(),
        "final_scene_id": current_scene_id,
        "scene_transitions": scene_transitions,
        "mode": "headless",
        "frames": frames,
        "simulation_updates": frames,
        "total_draw_calls": total_draw_calls,
        "total_triangles": total_triangles,
        "last_visible_drawables": last_visible_drawables,
        "cooked_discovered_assets": cooked_report.discovered_assets,
        "loaded_meshes": cooked_report.loaded_meshes,
        "loaded_textures": cooked_report.loaded_textures,
        "loaded_materials": cooked_report.loaded_materials,
        "skipped_cooked_assets": cooked_report.skipped_assets.len(),
        "script_assemblies": script_assemblies,
        "script_instances": script_instances,
        "script_started_instances": script_started_instances,
        "script_update_count": script_update_count,
        "script_entity_translations": script_entity_translations,
        "cell_streaming": cell_streaming_report,
        "world_origin": game_loop.world_origin(),
        "world_origin_shifts": game_loop.world_origin_shift_count(),
        // ENG-04: rolling per-pass CPU timing summary. The headless QA
        // backend reports GPU timing as unavailable; GPU fields are absent.
        "frame_timing": game_loop.runtime.frame_timing_summary(),
        "script_errors": 0,
        "passed": true
    }))
    .expect("JSON value serialization cannot fail");
    println!("{report}");
    if let Some(path) = report_path {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "could not create report directory {}: {error}",
                    parent.display()
                )
            })?;
        }
        std::fs::write(path, format!("{report}\n"))
            .map_err(|error| format!("could not write run report {}: {error}", path.display()))?;
    }
    Ok(())
}

fn format_diagnostics(diagnostics: Vec<engine_serialize::Diagnostic>) -> String {
    diagnostics
        .into_iter()
        .map(|diagnostic| format!("{}: {}", diagnostic.code, diagnostic.message))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(feature = "backend-vulkan")]
fn run_windowed(
    project: GameProject,
    scene: Scene,
    max_frames: Option<u64>,
    stream_cells: bool,
) -> Result<(), String> {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Instant;

    use engine_core::create_vulkan_backend_renderer;
    use platform::winit::window::Window;
    use platform::{EventFlow, PlatformEvent, WindowApp, WindowDescriptor};
    use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

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
        fn on_create(&mut self, window: Arc<Window>) {
            let size = window.inner_size();
            let display_handle = match window.display_handle() {
                Ok(handle) => handle.as_raw(),
                Err(error) => {
                    self.failed.store(true, Ordering::Release);
                    tracing::error!(%error, "could not acquire display handle");
                    return;
                }
            };
            let window_handle = match window.window_handle() {
                Ok(handle) => handle.as_raw(),
                Err(error) => {
                    self.failed.store(true, Ordering::Release);
                    tracing::error!(%error, "could not acquire window handle");
                    return;
                }
            };
            let backend = match create_vulkan_backend_renderer(
                display_handle,
                window_handle,
                size.width.max(1),
                size.height.max(1),
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
                    game_loop.set_ui_viewport_size(size.width, size.height);
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

        fn on_event(&mut self, window: &Window, event: PlatformEvent) -> EventFlow {
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
fn run_windowed(
    _project: GameProject,
    _scene: Scene,
    _max_frames: Option<u64>,
    _stream_cells: bool,
) -> Result<(), String> {
    Err("windowed project run requires the `backend-vulkan` feature; use --headless or rebuild with Vulkan support".into())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use engine_asset::project::ProjectManifest;

    use super::*;

    fn scene_project_fixture() -> (tempfile::TempDir, GameProject) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let scene_dir = root.join("assets/scenes");
        let source = root.join("assets/source");
        let cooked = root.join("build/cooked");
        std::fs::create_dir_all(&scene_dir).unwrap();
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&cooked).unwrap();

        let main_path = scene_dir.join("main.scene.ron");
        let level_path = scene_dir.join("level_two.scene.ron");
        let mut main = engine_scene::sample_scene();
        main.scene_id = "main".into();
        main.save_to_file(&main_path).unwrap();
        let mut level = engine_scene::sample_scene();
        level.scene_id = "level_two".into();
        level.name = "Level Two".into();
        level.save_to_file(&level_path).unwrap();

        let mut manifest = ProjectManifest::new("Scene Transition Test");
        manifest.startup_scene = PathBuf::from("main");
        manifest.input_actions = None;
        manifest.scenes = BTreeMap::from([
            ("main".into(), PathBuf::from("assets/scenes/main.scene.ron")),
            (
                "level_two".into(),
                PathBuf::from("assets/scenes/level_two.scene.ron"),
            ),
        ]);
        let manifest_path = manifest.write_to_root(&root).unwrap();
        assert!(manifest_path.is_file());
        let project = GameProject::load(&root).unwrap();
        (temp, project)
    }

    fn transform_record(translation: [f32; 3]) -> engine_scene::ComponentRecord {
        engine_scene::ComponentRecord {
            schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: BTreeMap::from([
                (
                    "translation".into(),
                    engine_serialize::Value::Vec3(translation),
                ),
                (
                    "rotation".into(),
                    engine_serialize::Value::Quat([0.0, 0.0, 0.0, 1.0]),
                ),
                ("scale".into(), engine_serialize::Value::Vec3([1.0; 3])),
            ]),
        }
    }

    fn cube_renderable_record() -> engine_scene::ComponentRecord {
        engine_scene::ComponentRecord {
            schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: BTreeMap::from([
                (
                    "mesh".into(),
                    engine_serialize::Value::Asset(engine_serialize::AssetId::new("mesh-cube")),
                ),
                (
                    "material".into(),
                    engine_serialize::Value::Asset(engine_serialize::AssetId::new("mat-default")),
                ),
                ("visible".into(), engine_serialize::Value::Bool(true)),
                (
                    "render_layer".into(),
                    engine_serialize::Value::Str("Default".into()),
                ),
                ("cast_shadows".into(), engine_serialize::Value::Bool(true)),
            ]),
        }
    }

    /// Project with a world partition: `cell_two` covers the origin and
    /// streams `level_two` (one cube with unique IDs) around the camera.
    fn cell_streaming_project_fixture() -> (tempfile::TempDir, GameProject) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let scene_dir = root.join("assets/scenes");
        let source = root.join("assets/source");
        let cooked = root.join("build/cooked");
        std::fs::create_dir_all(&scene_dir).unwrap();
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&cooked).unwrap();

        // Startup scene: the sample scene plus a mutable camera transform.
        let mut main = engine_scene::sample_scene();
        main.scene_id = "main".into();
        main.entities
            .iter_mut()
            .find(|entity| entity.persistent_id == "camera-main")
            .expect("sample scene camera")
            .components
            .insert("engine.transform".into(), transform_record([0.0; 3]));
        main.save_to_file(&scene_dir.join("main.scene.ron"))
            .unwrap();

        // Cell scene: unique entity IDs, no camera of its own.
        let mut level_two = engine_scene::sample_scene();
        level_two.scene_id = "level_two".into();
        level_two.name = "Streamed Cell".into();
        level_two.scene_settings.active_camera = None;
        level_two.entities = vec![engine_scene::EntityRecord {
            persistent_id: "cube-two".into(),
            parent: None,
            name: Some("Streamed Cube".into()),
            enabled: true,
            components: BTreeMap::from([
                ("engine.transform".into(), transform_record([1.0, 0.0, 0.0])),
                ("engine.renderable".into(), cube_renderable_record()),
            ]),
        }];
        level_two
            .save_to_file(&scene_dir.join("level_two.scene.ron"))
            .unwrap();

        std::fs::write(
            root.join(engine_asset::partition::WORLD_PARTITION_FILE_NAME),
            format!(
                "{{ \"schema\": \"{}\", \"cells\": {{ \"cell_two\": {{ \"scene\": \"level_two\", \"bounds\": {{ \"center\": [0.0, 0.0, 0.0], \"half_extents\": [10.0, 10.0, 10.0] }} }} }} }}\n",
                engine_asset::partition::WORLD_PARTITION_SCHEMA
            ),
        )
        .unwrap();

        let mut manifest = ProjectManifest::new("Cell Streaming Test");
        manifest.startup_scene = PathBuf::from("main");
        manifest.input_actions = None;
        manifest.scenes = BTreeMap::from([
            ("main".into(), PathBuf::from("assets/scenes/main.scene.ron")),
            (
                "level_two".into(),
                PathBuf::from("assets/scenes/level_two.scene.ron"),
            ),
        ]);
        manifest.write_to_root(&root).unwrap();
        let project = GameProject::load(&root).unwrap();
        (temp, project)
    }

    fn set_main_camera_position(game_loop: &mut GameLoop, position: [f32; 3]) {
        game_loop.runtime.with_world_mut(|world| {
            let camera = world.entity_by_persistent_id("camera-main").unwrap();
            world
                .get_mut::<engine_scene::components::Transform>(camera)
                .unwrap()
                .translation = glam::Vec3::from(position);
        });
    }

    /// Project whose startup scene opts into origin shifting: threshold 100,
    /// camera at x = 150 (past the threshold), and a visible cube five metres
    /// in front of the camera so draw calls keep flowing after the shift.
    fn origin_shift_project_fixture() -> (tempfile::TempDir, GameProject) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let scene_dir = root.join("assets/scenes");
        let source = root.join("assets/source");
        let cooked = root.join("build/cooked");
        std::fs::create_dir_all(&scene_dir).unwrap();
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&cooked).unwrap();

        let mut main = engine_scene::sample_scene();
        main.scene_id = "main".into();
        main.entities
            .iter_mut()
            .find(|entity| entity.persistent_id == "camera-main")
            .expect("sample scene camera")
            .components
            .insert(
                "engine.transform".into(),
                transform_record([150.0, 0.0, 0.0]),
            );
        main.entities
            .iter_mut()
            .find(|entity| entity.persistent_id == "cube-01")
            .expect("sample scene cube")
            .components
            .insert(
                "engine.transform".into(),
                transform_record([150.0, 0.0, -5.0]),
            );
        main.scene_settings.origin_shift.enabled = true;
        main.scene_settings.origin_shift.threshold = 100.0;
        main.save_to_file(&scene_dir.join("main.scene.ron"))
            .unwrap();

        let mut manifest = ProjectManifest::new("Origin Shift Test");
        manifest.startup_scene = PathBuf::from("main");
        manifest.input_actions = None;
        manifest.scenes =
            BTreeMap::from([("main".into(), PathBuf::from("assets/scenes/main.scene.ron"))]);
        manifest.write_to_root(&root).unwrap();
        let project = GameProject::load(&root).unwrap();
        (temp, project)
    }

    fn has_persistent_entity(game_loop: &GameLoop, id: &str) -> bool {
        game_loop
            .runtime
            .with_world(|world| world.entity_by_persistent_id(id).is_some())
            .unwrap_or(false)
    }

    #[test]
    fn cell_streaming_is_opt_in_and_requires_a_partition_manifest() {
        let (_temp, project) = cell_streaming_project_fixture();
        // Without the flag no driver is constructed even when a partition
        // manifest exists; with the flag the partition builds a driver.
        assert!(create_cell_streaming_driver(&project, false)
            .unwrap()
            .is_none());
        assert!(create_cell_streaming_driver(&project, true)
            .unwrap()
            .is_some());

        // The flag without a partition manifest is an explicit error.
        let (_temp2, no_partition) = scene_project_fixture();
        let error = create_cell_streaming_driver(&no_partition, true)
            .err()
            .expect("streaming without a partition manifest must fail");
        assert!(error.contains("world.partition.json"), "{error}");
    }

    #[test]
    fn headless_cell_streaming_loads_and_unloads_cells_around_the_camera() {
        let (_temp, project) = cell_streaming_project_fixture();
        let scene = Scene::load_from_file(project.startup_scene_path()).unwrap();
        let (mut game_loop, _) = create_game_loop(&project, scene).unwrap();
        game_loop
            .runtime
            .set_renderer_backend(Box::<crate::qa::QaBackend>::default());
        let mut driver = create_cell_streaming_driver(&project, true).unwrap();
        driver.as_mut().unwrap().rebaseline(&game_loop.runtime);
        let mut current_scene_id = project.startup_scene_id().to_string();

        // Frame boundary with the camera at the origin: the cell streams in.
        let transitions =
            process_pending_scene_transitions(&mut game_loop, &project, &mut current_scene_id)
                .unwrap();
        tick_cell_streaming(&mut game_loop, &mut driver, transitions);
        assert!(has_persistent_entity(&game_loop, "cube-two"));
        assert_eq!(
            driver.as_ref().unwrap().loaded_cells(),
            vec!["cell_two".to_string()]
        );

        // The camera leaves the cell bounds: the cell unloads at the next
        // frame boundary.
        game_loop.update(1.0 / 60.0);
        set_main_camera_position(&mut game_loop, [100.0, 0.0, 0.0]);
        let transitions =
            process_pending_scene_transitions(&mut game_loop, &project, &mut current_scene_id)
                .unwrap();
        tick_cell_streaming(&mut game_loop, &mut driver, transitions);
        assert!(!has_persistent_entity(&game_loop, "cube-two"));
        assert!(driver.as_ref().unwrap().loaded_cells().is_empty());

        // The camera returns: the cell streams back in.
        game_loop.update(1.0 / 60.0);
        set_main_camera_position(&mut game_loop, [0.0, 0.0, 0.0]);
        let transitions =
            process_pending_scene_transitions(&mut game_loop, &project, &mut current_scene_id)
                .unwrap();
        tick_cell_streaming(&mut game_loop, &mut driver, transitions);
        assert!(has_persistent_entity(&game_loop, "cube-two"));
        let driver = driver.unwrap();
        assert_eq!(driver.total_merges(), 2);
        assert_eq!(driver.total_unloads(), 1);
    }

    #[test]
    fn headless_run_report_includes_cell_streaming_state() {
        let (_temp, project) = cell_streaming_project_fixture();
        let scene = Scene::load_from_file(project.startup_scene_path()).unwrap();
        let report_path = project.root.join("build/run-report.json");
        run_headless(project, scene, 3, Some(&report_path), true).unwrap();

        let report: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&report_path).unwrap()).unwrap();
        assert_eq!(report["passed"], true);
        assert_eq!(report["cell_streaming"]["enabled"], true);
        assert_eq!(
            report["cell_streaming"]["loaded_cells"],
            serde_json::json!(["cell_two"])
        );
        assert_eq!(report["cell_streaming"]["total_merges"], 1);
        assert_eq!(report["cell_streaming"]["total_unloads"], 0);
        assert_eq!(report["cell_streaming"]["resident_entities"], 0);
        assert_eq!(
            report["cell_streaming"]["cell_states"]["cell_two"],
            "Loaded"
        );
        // No origin shifting configured: the report shows the zero origin.
        assert_eq!(report["world_origin"], serde_json::json!([0.0, 0.0, 0.0]));
        assert_eq!(report["world_origin_shifts"], 0);
    }

    #[test]
    fn headless_run_report_includes_frame_timing_section() {
        let (_temp, project) = scene_project_fixture();
        let scene = Scene::load_from_file(project.startup_scene_path()).unwrap();
        let report_path = project.root.join("build/run-report.json");
        run_headless(project, scene, 3, Some(&report_path), false).unwrap();

        let report: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&report_path).unwrap()).unwrap();
        assert_eq!(report["passed"], true);

        let frame_timing = &report["frame_timing"];
        assert_eq!(frame_timing["window_frames"], 3);
        // The headless QA backend cannot provide GPU timestamps: the section
        // reports "unavailable" and carries CPU-only aggregates.
        assert_eq!(frame_timing["gpu_status"], "unavailable");
        assert!(frame_timing.get("total_gpu").is_none());
        assert!(frame_timing["total_cpu"]["avg_ms"].is_number());

        let passes = frame_timing["passes"].as_array().unwrap();
        for stage in [
            "update",
            "extraction",
            "sync_render_assets",
            "render_submit",
        ] {
            let pass = passes
                .iter()
                .find(|pass| pass["name"] == stage)
                .unwrap_or_else(|| panic!("missing stage '{stage}' in {passes:?}"));
            assert_eq!(pass["cpu"]["samples"], 3);
            assert!(pass["cpu"]["avg_ms"].is_number());
            assert!(pass["cpu"]["p95_ms"].is_number());
            assert!(pass["cpu"]["max_ms"].is_number());
            assert!(pass.get("gpu").is_none());
        }
    }

    #[test]
    fn headless_run_shifts_world_origin_past_threshold() {
        let (_temp, project) = origin_shift_project_fixture();
        let scene = Scene::load_from_file(project.startup_scene_path()).unwrap();
        let report_path = project.root.join("build/run-report.json");
        run_headless(project, scene, 3, Some(&report_path), false).unwrap();

        let report: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&report_path).unwrap()).unwrap();
        assert_eq!(report["passed"], true);
        assert!(report["total_draw_calls"].as_u64().unwrap() > 0);
        // The camera starts at x = 150 with threshold 100: exactly one shift
        // runs at the first frame boundary and the camera lands back on the
        // relative origin, so no further shift triggers.
        assert_eq!(report["world_origin_shifts"], 1);
        let origin = report["world_origin"].as_array().unwrap();
        assert_eq!(origin[0].as_f64().unwrap(), 150.0);
        assert_eq!(origin[1].as_f64().unwrap(), 0.0);
        assert_eq!(origin[2].as_f64().unwrap(), 0.0);
    }

    #[test]
    fn project_scene_transition_loads_catalog_scene_and_rejects_unknown_id() {
        let (_temp, project) = scene_project_fixture();
        let mut game_loop = GameLoop::new(EngineConfig::default());
        game_loop
            .load_scene(Scene::load_from_file(project.startup_scene_path()).unwrap())
            .unwrap();

        let unknown = SceneLoadRequest {
            scene_id: "missing".into(),
            requested_by: "cube-01".into(),
        };
        assert!(transition_to_project_scene(&mut game_loop, &project, &unknown).is_err());
        assert_eq!(
            game_loop
                .runtime
                .scene_ref()
                .map(|scene| scene.scene_id.as_str()),
            Some("main")
        );

        let request = SceneLoadRequest {
            scene_id: "level_two".into(),
            requested_by: "cube-01".into(),
        };
        transition_to_project_scene(&mut game_loop, &project, &request).unwrap();
        assert_eq!(
            game_loop
                .runtime
                .scene_ref()
                .map(|scene| scene.scene_id.as_str()),
            Some("level_two")
        );
        #[cfg(any(feature = "target-desktop", feature = "subsystem-scripting-csharp"))]
        assert!(game_loop.physics.is_some());
    }

    #[test]
    fn failed_post_load_validation_restores_the_previous_scene() {
        let mut game_loop = GameLoop::new(EngineConfig::default());
        let mut previous = engine_scene::sample_scene();
        previous.scene_id = "previous".into();
        previous
            .entities
            .iter_mut()
            .find(|entity| entity.persistent_id == "cube-01")
            .expect("sample scene cube")
            .components
            .insert(
                "engine.transform".into(),
                engine_scene::ComponentRecord {
                    schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
                    enabled: true,
                    fields: BTreeMap::from([
                        (
                            "translation".into(),
                            engine_serialize::Value::Vec3([0.0; 3]),
                        ),
                        (
                            "rotation".into(),
                            engine_serialize::Value::Quat([0.0, 0.0, 0.0, 1.0]),
                        ),
                        ("scale".into(), engine_serialize::Value::Vec3([1.0; 3])),
                    ]),
                },
            );
        game_loop.load_scene(previous).unwrap();
        game_loop
            .runtime
            .with_world_mut(|world| {
                let entity = world.entity_by_persistent_id("cube-01").unwrap();
                world
                    .get_mut::<engine_scene::components::Transform>(entity)
                    .unwrap()
                    .translation
                    .x = 42.0;
            })
            .unwrap();
        let rollback_snapshot = capture_scene_transition_rollback(&game_loop.runtime).unwrap();

        let mut rejected = engine_scene::sample_scene();
        rejected.scene_id = "rejected".into();
        game_loop.load_scene(rejected).unwrap();

        let error = rollback_failed_scene_transition(
            &mut game_loop,
            Some(rollback_snapshot),
            "post-load validation failed".into(),
        )
        .unwrap_err();
        assert!(error.contains("previous scene was restored"));
        assert_eq!(
            game_loop
                .runtime
                .scene_ref()
                .map(|scene| scene.scene_id.as_str()),
            Some("previous")
        );
        assert_eq!(
            game_loop.runtime.with_world(|world| {
                let entity = world.entity_by_persistent_id("cube-01").unwrap();
                world
                    .get::<engine_scene::components::Transform>(entity)
                    .unwrap()
                    .translation
                    .x
            }),
            Some(42.0)
        );
    }

    #[test]
    fn runtime_dependency_check_includes_extension_asset_fields() {
        let mut scene = engine_scene::sample_scene();
        scene.entities[0].components.insert(
            "engine.audio_source".into(),
            engine_scene::ComponentRecord {
                schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: BTreeMap::from([(
                    "clip_asset".into(),
                    engine_serialize::Value::Asset(engine_serialize::AssetId::new("audio.missing")),
                )]),
            },
        );
        scene.entities[0].components.insert(
            "engine.canvas".into(),
            engine_scene::ComponentRecord {
                schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: BTreeMap::from([(
                    "elements".into(),
                    engine_serialize::Value::List(vec![engine_serialize::Value::Map(
                        BTreeMap::from([(
                            "texture".into(),
                            engine_serialize::Value::Asset(engine_serialize::AssetId::new(
                                "ui.image",
                            )),
                        )]),
                    )]),
                )]),
            },
        );
        let mut runtime = EngineRuntime::new(EngineConfig::default());

        let missing = missing_runtime_asset_dependencies(&runtime, &scene);
        assert!(missing.iter().any(|asset| asset.id == "audio.missing"));
        assert!(missing.iter().any(|asset| asset.id == "ui.image"));

        runtime
            .asset_registry_mut()
            .insert_typed(engine_serialize::AssetId::new("audio.missing"), vec![0u8]);
        runtime
            .asset_registry_mut()
            .insert_typed(engine_serialize::AssetId::new("ui.image"), vec![0u8]);
        assert!(missing_runtime_asset_dependencies(&runtime, &scene).is_empty());
    }
}
