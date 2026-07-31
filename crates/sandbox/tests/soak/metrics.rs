/// Process working-set bytes for the memory-plateau regression check, or
/// `None` on platforms without a supported sampler.
fn working_set_bytes() -> Option<u64> {
    #[cfg(windows)]
    {
        windows_working_set_bytes()
    }
    #[cfg(target_os = "linux")]
    {
        linux_working_set_bytes()
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        None
    }
}

fn working_set_sampler_name() -> &'static str {
    #[cfg(windows)]
    {
        "windows-GetProcessMemoryInfo"
    }
    #[cfg(target_os = "linux")]
    {
        "linux-/proc/self/status-VmRSS"
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        "unavailable"
    }
}

#[cfg(windows)]
fn windows_working_set_bytes() -> Option<u64> {
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows::Win32::System::Threading::GetCurrentProcess;

    // GetCurrentProcess returns a pseudo-handle that must not be closed.
    unsafe {
        let mut info = PROCESS_MEMORY_COUNTERS::default();
        let result = GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut info,
            std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        );
        result.is_ok().then_some(info.WorkingSetSize as u64)
    }
}

#[cfg(target_os = "linux")]
fn linux_working_set_bytes() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let kilobytes = status.lines().find_map(|line| {
        line.strip_prefix("VmRSS:")?
            .trim()
            .strip_suffix("kB")?
            .trim()
            .parse::<u64>()
            .ok()
    })?;
    Some(kilobytes * 1024)
}

// Scenario metrics.

#[derive(Clone, Copy, Debug)]
struct WorkingSetSample {
    frame: u64,
    bytes: u64,
}

/// State hashed for the determinism gate: every externally visible outcome
/// of the scenario except timing and memory. Two runs with the same seed and
/// frame count must serialize to byte-identical JSON.
#[derive(Default, Serialize)]
struct DeterminismDigest {
    shift_events: Vec<(u64, [f64; 3])>,
    merge_events: Vec<(u64, String)>,
    unload_events: Vec<(u64, String)>,
    enqueue_events: Vec<(u64, usize)>,
    entity_count_trace: Vec<u32>,
    entity_positions_final: BTreeMap<String, [f32; 3]>,
    installed_materials: Vec<String>,
    cached_asset_count: usize,
    loaded_cells_final: Vec<String>,
    world_origin_final: [f64; 3],
    world_origin_shifts: u64,
    camera_logical_x_final: f32,
    total_draw_calls: u64,
    total_triangles: u64,
}

struct SoakOutcome {
    frames: u64,
    warmup_frames: u64,
    seed: u64,
    digest: DeterminismDigest,
    digest_sha256: String,
    base_entities: usize,
    entity_samples: Vec<usize>,
    working_set_samples: Vec<WorkingSetSample>,
    cached_assets_at_warmup: usize,
    installed_materials_at_warmup: usize,
    warmup_p95_ms: Option<f32>,
    final_p95_ms: Option<f32>,
    failed_cells: usize,
    failed_stream_batches: usize,
    enqueued_assets: usize,
    error_diagnostics: usize,
    wall_time: Duration,
}

fn set_camera_relative_x(game_loop: &mut GameLoop, relative_x: f32) {
    game_loop
        .runtime
        .with_world_mut(|world| {
            let camera = world
                .entity_by_persistent_id("camera-main")
                .expect("camera entity");
            world
                .get_mut::<engine_scene::components::Transform>(camera)
                .expect("camera transform")
                .translation = glam::Vec3::new(relative_x, 0.0, 0.0);
        })
        .expect("world active");
}

fn persistent_entity_count(game_loop: &GameLoop) -> usize {
    game_loop
        .runtime
        .with_world(|world| world.persistent_entities().count())
        .expect("world active")
}

/// Final per-entity logical positions (origin-relative position plus the
/// current world origin), keyed by persistent ID for the digest.
fn logical_entity_positions(game_loop: &GameLoop) -> BTreeMap<String, [f32; 3]> {
    let origin = game_loop.world_origin();
    game_loop
        .runtime
        .with_world(|world| {
            world
                .persistent_entities()
                .filter_map(|(id, entity)| {
                    let transform = world.get::<engine_scene::components::Transform>(entity)?;
                    Some((
                        id.to_string(),
                        [
                            transform.translation.x + origin[0] as f32,
                            transform.translation.y + origin[1] as f32,
                            transform.translation.z + origin[2] as f32,
                        ],
                    ))
                })
                .collect()
        })
        .expect("world active")
}

/// Reap the background asset stream until it is idle so every enqueued
/// decode commits at this frame boundary. The drain itself is asynchronous;
/// looping makes the observable install point deterministic.
fn drain_asset_stream_until_idle(runtime: &mut EngineRuntime) -> usize {
    let mut failed_batches = 0usize;
    for _ in 0..10_000 {
        let report = runtime.drain_cooked_asset_stream();
        failed_batches += report.failed_batches;
        if report.is_complete() {
            return failed_batches;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("background asset stream did not drain within the iteration budget");
}

fn push_working_set_sample(frame: u64, samples: &mut Vec<WorkingSetSample>) {
    if let Some(bytes) = working_set_bytes() {
        samples.push(WorkingSetSample { frame, bytes });
    }
}

fn total_cpu_p95_ms(game_loop: &GameLoop) -> Option<f32> {
    game_loop
        .runtime
        .frame_timing_summary()
        .total_cpu
        .map(|aggregate| aggregate.p95_ms)
}
