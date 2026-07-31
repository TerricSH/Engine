fn run_scenario(frames: u64, warmup_frames: u64, seed: u64) -> SoakOutcome {
    let started = Instant::now();
    let fixture = build_fixture(seed);
    let warmup_frames = warmup_frames.min(frames.max(1) / 2).max(1);

    let startup =
        Scene::load_from_file(fixture.project.startup_scene_path()).expect("load startup scene");
    let mut game_loop = GameLoop::new(EngineConfig {
        application_name: "soak".to_string(),
        gpu_timestamps: false,
    });
    game_loop.load_scene(startup).expect("startup scene loads");
    game_loop.init_physics();
    game_loop.validate_ready().expect("runtime ready");
    game_loop
        .runtime
        .set_renderer_backend(Box::new(SoakBackend::default()));
    let mut driver = CellStreamingDriver::new(
        &fixture.partition,
        &fixture.project,
        CellStreamingConfig::default(),
    )
    .expect("cell streaming driver");
    driver.rebaseline(&game_loop.runtime);

    let base_entities = persistent_entity_count(&game_loop);
    let mut digest = DeterminismDigest::default();
    let mut entity_samples = Vec::with_capacity(frames as usize);
    let mut working_set_samples = Vec::new();
    let mut failed_cells = 0usize;
    let mut failed_stream_batches = 0usize;
    let mut enqueued_assets = 0usize;
    let mut cached_assets_at_warmup = None;
    let mut installed_materials_at_warmup = None;
    let mut warmup_p95_ms = None;

    push_working_set_sample(0, &mut working_set_samples);

    for frame in 0..frames {
        // Script the camera in logical space; the transform stores the
        // origin-relative position.
        let logical_x = patrol_position(frame);
        let origin = game_loop.world_origin();
        set_camera_relative_x(&mut game_loop, (f64::from(logical_x) - origin[0]) as f32);

        game_loop.update(1.0 / 60.0);

        if let Some(shift) = game_loop.tick_world_origin_shift() {
            digest.shift_events.push((frame, shift.origin));
        }

        let report = driver.tick(&mut game_loop.runtime);
        failed_cells += report.failed_cells.len();
        enqueued_assets += report.enqueued_assets;
        let world_changed = report.world_changed();
        for cell in report.merged_cells {
            digest.merge_events.push((frame, cell));
        }
        for cell in report.unloaded_cells {
            digest.unload_events.push((frame, cell));
        }
        if report.enqueued_assets > 0 {
            digest.enqueue_events.push((frame, report.enqueued_assets));
        }
        if world_changed {
            game_loop.resync_physics_from_world();
        }
        failed_stream_batches += drain_asset_stream_until_idle(&mut game_loop.runtime);

        let stats = game_loop.render(frame).expect("frame renders");
        digest.total_draw_calls += u64::from(stats.draw_calls);
        digest.total_triangles += stats.triangles;

        let entities = persistent_entity_count(&game_loop);
        digest.entity_count_trace.push(entities as u32);
        entity_samples.push(entities);

        if frame % WORKING_SET_SAMPLE_INTERVAL == 0 {
            push_working_set_sample(frame, &mut working_set_samples);
        }
        if frame + 1 == warmup_frames {
            cached_assets_at_warmup = Some(game_loop.runtime.asset_registry().cached_ids().len());
            installed_materials_at_warmup = Some(count_installed_materials(&game_loop, &fixture));
            warmup_p95_ms = total_cpu_p95_ms(&game_loop);
            push_working_set_sample(frame + 1, &mut working_set_samples);
        }
    }
    push_working_set_sample(frames, &mut working_set_samples);

    digest.entity_positions_final = logical_entity_positions(&game_loop);
    digest.installed_materials = installed_material_list(&game_loop, &fixture);
    digest.cached_asset_count = game_loop.runtime.asset_registry().cached_ids().len();
    digest.loaded_cells_final = driver.loaded_cells();
    digest.world_origin_final = game_loop.world_origin();
    digest.world_origin_shifts = game_loop.world_origin_shift_count();
    digest.camera_logical_x_final = patrol_position(frames.saturating_sub(1));

    let error_diagnostics = game_loop
        .runtime
        .diagnostics_collector()
        .all()
        .into_iter()
        .filter(|diagnostic| {
            matches!(
                diagnostic.severity,
                DiagnosticSeverity::Error | DiagnosticSeverity::Fatal
            )
        })
        .count();

    let digest_bytes = serde_json::to_vec(&digest).expect("digest serialization");
    let digest_sha256 = format!("{:x}", Sha256::digest(&digest_bytes));

    SoakOutcome {
        frames,
        warmup_frames,
        seed,
        digest,
        digest_sha256,
        base_entities,
        entity_samples,
        working_set_samples,
        cached_assets_at_warmup: cached_assets_at_warmup
            .unwrap_or_else(|| game_loop.runtime.asset_registry().cached_ids().len()),
        installed_materials_at_warmup: installed_materials_at_warmup
            .unwrap_or_else(|| count_installed_materials(&game_loop, &fixture)),
        warmup_p95_ms,
        final_p95_ms: total_cpu_p95_ms(&game_loop),
        failed_cells,
        failed_stream_batches,
        enqueued_assets,
        error_diagnostics,
        wall_time: started.elapsed(),
    }
}

fn count_installed_materials(game_loop: &GameLoop, fixture: &SoakFixture) -> usize {
    fixture
        .material_ids
        .iter()
        .filter(|id| game_loop.runtime.asset_registry().contains(id))
        .count()
}

fn installed_material_list(game_loop: &GameLoop, fixture: &SoakFixture) -> Vec<String> {
    fixture
        .material_ids
        .iter()
        .filter(|id| game_loop.runtime.asset_registry().contains(id))
        .map(|id| id.id.clone())
        .collect()
}

// Report generation.

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("sandbox must live under <workspace>/crates")
        .to_path_buf()
}

fn report_path() -> PathBuf {
    std::env::var_os("SOAK_REPORT")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace_root().join("target/soak/soak-report.json"))
}

fn soak_frames_from_env() -> u64 {
    std::env::var("SOAK_FRAMES")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|frames| *frames > 0)
        .unwrap_or(DEFAULT_SOAK_FRAMES)
}

fn write_soak_report(path: &Path, outcome: &SoakOutcome) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create soak report directory");
    }
    let ws = |index: usize| {
        outcome
            .working_set_samples
            .get(index)
            .map(|sample| sample.bytes)
    };
    let last_ws = outcome
        .working_set_samples
        .last()
        .map(|sample| sample.bytes);
    let plateau_max_ws = outcome
        .working_set_samples
        .iter()
        .filter(|sample| sample.frame >= outcome.warmup_frames)
        .map(|sample| sample.bytes)
        .max();
    let report = serde_json::json!({
        "schema": "SoakReport-v0",
        "passed": true,
        "seed": outcome.seed,
        "frames": outcome.frames,
        "warmup_frames": outcome.warmup_frames,
        "wall_time_ms": outcome.wall_time.as_secs_f64() * 1000.0,
        "determinism_sha256": outcome.digest_sha256,
        "world_origin_shifts": outcome.digest.world_origin_shifts,
        "world_origin_final": outcome.digest.world_origin_final,
        "cell_merges": outcome.digest.merge_events.len(),
        "cell_unloads": outcome.digest.unload_events.len(),
        "cells_loaded_final": outcome.digest.loaded_cells_final,
        "stream_enqueue_events": outcome.digest.enqueue_events.len(),
        "enqueued_assets": outcome.enqueued_assets,
        "failed_cells": outcome.failed_cells,
        "failed_stream_batches": outcome.failed_stream_batches,
        "error_diagnostics": outcome.error_diagnostics,
        "script_errors": 0,
        "entities": {
            "base": outcome.base_entities,
            "post_warmup_max": outcome.entity_samples
                .get(outcome.warmup_frames as usize..)
                .and_then(|samples| samples.iter().max()),
            "final": outcome.entity_samples.last(),
        },
        "assets": {
            "installed_cell_materials": outcome.digest.installed_materials,
            "cached_at_warmup": outcome.cached_assets_at_warmup,
            "cached_final": outcome.digest.cached_asset_count,
        },
        "memory": {
            "sampler": working_set_sampler_name(),
            "start_bytes": ws(0),
            "post_warmup_bytes": outcome.working_set_samples.iter()
                .rev()
                .find(|sample| sample.frame <= outcome.warmup_frames)
                .map(|sample| sample.bytes),
            "end_bytes": last_ws,
            "plateau_max_bytes": plateau_max_ws,
        },
        "frame_timing": {
            "warmup_p95_ms": outcome.warmup_p95_ms,
            "final_p95_ms": outcome.final_p95_ms,
        },
        "draw_calls_total": outcome.digest.total_draw_calls,
        "triangles_total": outcome.digest.total_triangles,
    });
    let json = serde_json::to_string_pretty(&report).expect("report serialization");
    std::fs::write(path, format!("{json}\n")).expect("write soak report");
}

// Tests.
