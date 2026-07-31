use super::*;

pub(super) fn run_headless(
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

pub(super) fn format_diagnostics(diagnostics: Vec<engine_serialize::Diagnostic>) -> String {
    diagnostics
        .into_iter()
        .map(|diagnostic| format!("{}: {}", diagnostic.code, diagnostic.message))
        .collect::<Vec<_>>()
        .join("\n")
}
