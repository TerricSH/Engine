use super::*;

pub(super) fn load_startup_scene(path: &Path) -> Result<Scene, String> {
    Scene::load_from_file(path)
        .map_err(|error| format!("could not load startup scene {}: {error}", path.display()))
}

pub(super) fn create_game_loop(
    project: &GameProject,
    scene: Scene,
) -> Result<(GameLoop, CookedAssetLoadReport), String> {
    let expected_script_instances =
        crate::project_scripts::validate_runtime_script_references(project, &scene)?;
    let input_map = crate::project_input::load_project_input_map(project)?;
    let mut game_loop = GameLoop::new(EngineConfig {
        application_name: project.manifest.name.clone(),
        gpu_timestamps: true,
    });
    #[cfg(feature = "subsystem-scripting-csharp")]
    game_loop.set_script_viewport_size(
        project.manifest.window.width,
        project.manifest.window.height,
    );
    #[cfg(feature = "subsystem-scripting-csharp")]
    game_loop.set_script_save_directory(project.root.join("savegames"));
    #[cfg(feature = "subsystem-ui")]
    game_loop.set_ui_viewport_size(
        project.manifest.window.width,
        project.manifest.window.height,
    );
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
pub(super) fn create_cell_streaming_driver(
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
pub(super) fn tick_cell_streaming(
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

#[derive(Debug, PartialEq, Eq)]
pub(super) enum SceneTransitionFailure {
    /// The original scene is known to still be active, either because loading
    /// never began or because rollback completed.
    Recoverable(String),
    /// The requested scene failed after activation and restoring the previous
    /// scene also failed, so the host must stop rather than claim a retryable
    /// retained-scene state.
    Fatal(String),
}

impl SceneTransitionFailure {
    pub(super) fn into_message(self) -> String {
        match self {
            Self::Recoverable(message) | Self::Fatal(message) => message,
        }
    }
}

pub(super) fn transition_to_project_scene(
    game_loop: &mut GameLoop,
    project: &GameProject,
    request: &SceneLoadRequest,
) -> Result<(), String> {
    transition_to_project_scene_classified(game_loop, project, request)
        .map_err(SceneTransitionFailure::into_message)
}

pub(super) fn transition_to_project_scene_classified(
    game_loop: &mut GameLoop,
    project: &GameProject,
    request: &SceneLoadRequest,
) -> Result<(), SceneTransitionFailure> {
    let scene_path = project.scene_path(&request.scene_id).ok_or_else(|| {
        SceneTransitionFailure::Recoverable(format!(
            "script entity '{}' requested unknown scene '{}'; available scenes: {}",
            request.requested_by,
            request.scene_id,
            project
                .scenes()
                .into_iter()
                .map(|(id, _)| id)
                .collect::<Vec<_>>()
                .join(", ")
        ))
    })?;
    let scene = Scene::load_from_file(&scene_path).map_err(|error| {
        SceneTransitionFailure::Recoverable(format!(
            "could not load requested scene '{}' from {}: {error}",
            request.scene_id,
            scene_path.display()
        ))
    })?;
    let expected_script_instances = crate::project_scripts::validate_runtime_script_references(
        project, &scene,
    )
    .map_err(|error| {
        SceneTransitionFailure::Recoverable(format!(
            "requested scene '{}' is invalid: {error}",
            request.scene_id
        ))
    })?;
    let missing = missing_runtime_asset_dependencies(&game_loop.runtime, &scene);
    if !missing.is_empty() {
        return Err(SceneTransitionFailure::Recoverable(format!(
            "requested scene '{}' references assets unavailable at runtime: {}",
            request.scene_id,
            missing
                .into_iter()
                .map(|asset| asset.id)
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }

    let previous_scene = capture_scene_transition_rollback(&game_loop.runtime);
    game_loop.runtime.diagnostics_collector_mut().clear_frame();
    game_loop.load_scene(scene).map_err(|diagnostics| {
        SceneTransitionFailure::Recoverable(format_diagnostics(diagnostics))
    })?;
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
        Err(error) => rollback_failed_scene_transition_classified(game_loop, previous_scene, error),
    }
}

pub(super) fn capture_scene_transition_rollback(runtime: &EngineRuntime) -> Option<Scene> {
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

#[cfg(test)]
pub(super) fn rollback_failed_scene_transition(
    game_loop: &mut GameLoop,
    previous_scene: Option<Scene>,
    transition_error: String,
) -> Result<(), String> {
    rollback_failed_scene_transition_classified(game_loop, previous_scene, transition_error)
        .map_err(SceneTransitionFailure::into_message)
}

pub(super) fn rollback_failed_scene_transition_classified(
    game_loop: &mut GameLoop,
    previous_scene: Option<Scene>,
    transition_error: String,
) -> Result<(), SceneTransitionFailure> {
    let Some(previous_scene) = previous_scene else {
        return Err(SceneTransitionFailure::Fatal(format!(
            "{transition_error}; no previous scene snapshot was available for rollback"
        )));
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
        Ok(()) => Err(SceneTransitionFailure::Recoverable(format!(
            "{transition_error}; the previous scene was restored after the failed transition"
        ))),
        Err(rollback_error) => Err(SceneTransitionFailure::Fatal(format!(
            "{transition_error}; restoring the previous scene also failed: {rollback_error}"
        ))),
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
            break;
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
        #[cfg(feature = "subsystem-terrain")]
        let _ = game_loop.cancel_pending_planet_scene_transition();
        return Err(format!(
            "scene transition chain exceeded {MAX_CHAINED_SCENE_TRANSITIONS} loads in one frame; latest request was '{}' from '{}'",
            request.scene_id, request.requested_by
        ));
    }
    if transitions > 0 {
        #[cfg(feature = "subsystem-terrain")]
        let _ = game_loop.cancel_pending_planet_scene_transition();
        return Ok(transitions);
    }
    #[cfg(feature = "subsystem-terrain")]
    {
        transitions +=
            process_pending_planet_scene_transition(game_loop, project, current_scene_id)?;
    }
    Ok(transitions)
}

#[cfg(feature = "subsystem-terrain")]
pub(super) fn process_pending_planet_scene_transition(
    game_loop: &mut GameLoop,
    project: &GameProject,
    current_scene_id: &mut String,
) -> Result<usize, String> {
    let Some(ticket) = game_loop.take_pending_planet_scene_transition() else {
        return Ok(0);
    };
    if ticket.request.scene_id == *current_scene_id {
        game_loop
            .reject_planet_scene_transition(&ticket)
            .map_err(|error| {
                format!(
                    "could not reject redundant planet scene transition from '{}': {error}",
                    ticket.controller_id
                )
            })?;
        tracing::warn!(
            scene = ticket.request.scene_id,
            controller = ticket.controller_id,
            terrain_volume = ticket.terrain_volume_id,
            "ignored planet transition to the already-active scene"
        );
        return Ok(0);
    }

    let request = SceneLoadRequest {
        scene_id: ticket.request.scene_id.clone(),
        requested_by: format!("planet-transition:{}", ticket.controller_id),
    };
    let result = transition_to_project_scene_classified(game_loop, project, &request);
    settle_planet_scene_transition(game_loop, current_scene_id, ticket, result)
}

#[cfg(feature = "subsystem-terrain")]
pub(super) fn settle_planet_scene_transition(
    game_loop: &mut GameLoop,
    current_scene_id: &mut String,
    ticket: engine_core::game_loop::PlanetSceneTransitionTicket,
    result: Result<(), SceneTransitionFailure>,
) -> Result<usize, String> {
    match result {
        Ok(()) => {
            game_loop
                .commit_planet_scene_transition(&ticket)
                .map_err(|error| {
                    format!(
                        "planet scene '{}' loaded but transition acknowledgement for '{}' failed: {error}",
                        ticket.request.scene_id, ticket.controller_id
                    )
                })?;
            *current_scene_id = ticket.request.scene_id;
            Ok(1)
        }
        Err(SceneTransitionFailure::Recoverable(error)) => {
            game_loop
                .reject_planet_scene_transition(&ticket)
                .map_err(|reject_error| {
                    format!(
                        "{error}; rejecting planet transition '{}' also failed: {reject_error}",
                        ticket.controller_id
                    )
                })?;
            tracing::warn!(
                scene = ticket.request.scene_id,
                controller = ticket.controller_id,
                terrain_volume = ticket.terrain_volume_id,
                reason = error,
                "planet scene transition failed; retained the active scene and will retry after a later update"
            );
            Ok(0)
        }
        Err(SceneTransitionFailure::Fatal(error)) => {
            game_loop
                .reject_planet_scene_transition(&ticket)
                .map_err(|reject_error| {
                    format!(
                        "{error}; rejecting fatal planet transition '{}' also failed: {reject_error}",
                        ticket.controller_id
                    )
                })?;
            Err(format!(
                "fatal planet scene transition '{}' to '{}' left scene state uncertain: {error}",
                ticket.controller_id, ticket.request.scene_id
            ))
        }
    }
}
