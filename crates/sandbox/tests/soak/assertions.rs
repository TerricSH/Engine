/// Long-run stability gate: the scenario must reach a steady state and stay
/// there — no streaming failures, no unbounded entity/asset growth, no
/// monotonic memory growth, no frame-time blow-up.
#[test]
fn soak_long_run_plateaus_and_produces_a_report() {
    let _guard = scenario_lock();
    let frames = soak_frames_from_env();
    let outcome = run_scenario(frames, WARMUP_FRAMES, SOAK_SEED);

    // The scenario actually engaged every subsystem under test.
    assert!(
        outcome.digest.world_origin_shifts >= frames / 16,
        "expected repeated origin shifts over {} frames, got {}",
        frames,
        outcome.digest.world_origin_shifts
    );
    assert!(
        outcome.digest.merge_events.len() >= CELL_COUNT,
        "every cell must merge at least once: {:?}",
        outcome.digest.merge_events
    );
    assert!(
        !outcome.digest.unload_events.is_empty(),
        "cells must unload behind the travelling camera"
    );
    assert!(
        outcome.enqueued_assets >= CELL_COUNT,
        "background asset streaming must enqueue every cell material"
    );
    assert!(
        outcome.digest.total_draw_calls > 0,
        "headless rendering must produce draw calls"
    );

    // No streaming, script, or runtime errors at any point in the run.
    assert_eq!(outcome.failed_cells, 0, "cells must not fail");
    assert_eq!(
        outcome.failed_stream_batches, 0,
        "background asset decode must not fail"
    );
    assert_eq!(
        outcome.error_diagnostics, 0,
        "no error-severity diagnostics may accumulate"
    );

    // Entity-count plateau: nothing after warmup may exceed the steady-state
    // peak observed during the second half of warmup.
    let warmup = outcome.warmup_frames as usize;
    let reference_max = outcome.entity_samples[warmup / 2..warmup]
        .iter()
        .copied()
        .max()
        .expect("warmup samples");
    let post_warmup_max = outcome.entity_samples[warmup..]
        .iter()
        .copied()
        .max()
        .expect("post-warmup samples");
    assert!(
        post_warmup_max <= reference_max,
        "entity count kept growing after warmup: steady-state peak {reference_max}, later peak {post_warmup_max}"
    );

    // Asset plateau: all cell materials stream in during warmup and nothing
    // new installs afterwards (streamed assets are never unloaded in v1, so
    // equality, not a bound, is the correct check).
    assert_eq!(
        outcome.installed_materials_at_warmup, CELL_COUNT,
        "every cell material must be installed by the end of warmup"
    );
    assert_eq!(
        outcome.cached_assets_at_warmup, outcome.digest.cached_asset_count,
        "asset registry kept growing after warmup"
    );
    assert_eq!(outcome.digest.installed_materials.len(), CELL_COUNT);

    // Memory plateau: process working set must not grow past the generous
    // regression bound after warmup. Skipped (never failed) on platforms
    // without a sampler.
    if outcome.working_set_samples.is_empty() {
        eprintln!(
            "SKIP (missing capability): no working-set sampler on this platform; memory plateau assertion disabled"
        );
    } else {
        let baseline = outcome
            .working_set_samples
            .iter()
            .rev()
            .find(|sample| sample.frame <= outcome.warmup_frames)
            .expect("post-warmup baseline sample");
        let end = outcome
            .working_set_samples
            .last()
            .expect("final working-set sample");
        let slack =
            (baseline.bytes / MEMORY_GROWTH_FRACTION_DENOMINATOR).max(MEMORY_GROWTH_FLOOR_BYTES);
        assert!(
            end.bytes <= baseline.bytes + slack,
            "working set kept growing after warmup: baseline {} bytes, end {} bytes (allowed +{} bytes)",
            baseline.bytes,
            end.bytes,
            slack
        );
    }

    // Frame-time regression tripwire: growth check, not an absolute budget.
    if let (Some(warmup_p95), Some(final_p95)) = (outcome.warmup_p95_ms, outcome.final_p95_ms) {
        assert!(
            final_p95 <= warmup_p95 * TIMING_REGRESSION_FACTOR + TIMING_REGRESSION_FLOOR_MS,
            "frame-time p95 regressed: warmup {warmup_p95} ms, final {final_p95} ms"
        );
    }

    let path = report_path();
    write_soak_report(&path, &outcome);
    eprintln!(
        "soak report written to {} (determinism sha256 {})",
        path.display(),
        outcome.digest_sha256
    );
}

/// Determinism gate: the same scripted scenario with the same seed and frame
/// count must produce byte-identical state across two runs: entity
/// positions, event sequences, and asset states alike. Timing and memory
/// fields are excluded from the compared digest.
#[test]
fn soak_scenario_is_deterministic_across_runs() {
    let _guard = scenario_lock();
    let first = run_scenario(DETERMINISM_FRAMES, WARMUP_FRAMES, SOAK_SEED);
    let second = run_scenario(DETERMINISM_FRAMES, WARMUP_FRAMES, SOAK_SEED);

    assert_eq!(
        first.digest.world_origin_shifts, second.digest.world_origin_shifts,
        "origin-shift cadence diverged"
    );
    let first_json = serde_json::to_string_pretty(&first.digest).expect("first digest JSON");
    let second_json = serde_json::to_string_pretty(&second.digest).expect("second digest JSON");
    assert_eq!(
        first_json, second_json,
        "identical scripted scenario must produce identical state across runs"
    );
    assert_eq!(first.digest_sha256, second.digest_sha256);
}

#[test]
fn patrol_path_is_periodic_and_bounded() {
    assert_eq!(PATROL_PERIOD_FRAMES, 80);
    for frame in 0..(PATROL_PERIOD_FRAMES * 3) {
        let position = patrol_position(frame);
        assert!((0.0..=PATROL_RANGE).contains(&position));
        let period_later = patrol_position(frame + PATROL_PERIOD_FRAMES);
        assert!(
            (position - period_later).abs() < f32::EPSILON,
            "patrol must be periodic: frame {frame}"
        );
    }
    let origin_position = patrol_position(0);
    assert!(
        origin_position.abs() < f32::EPSILON,
        "patrol starts at the origin"
    );
    let turnaround = patrol_position(PATROL_PERIOD_FRAMES / 2);
    assert!(
        (turnaround - PATROL_RANGE).abs() < f32::EPSILON,
        "patrol turns around at the range limit"
    );
}

#[test]
fn prng_is_deterministic_for_a_fixed_seed() {
    let mut first = XorShift64::new(SOAK_SEED);
    let mut second = XorShift64::new(SOAK_SEED);
    for _ in 0..128 {
        assert_eq!(first.next_u64(), second.next_u64());
    }
    let mut rng = XorShift64::new(SOAK_SEED);
    for _ in 0..128 {
        let value = rng.next_range(-16.0, 16.0);
        assert!((-16.0..16.0).contains(&value));
    }
}
