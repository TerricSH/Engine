//! Soak / stability harness and determinism gates for the engine (ENG-71).
//!
//! The harness drives a scripted long-run scenario entirely through public
//! engine APIs, exercising the subsystems that only misbehave over thousands
//! of frames:
//!
//! - a camera patrolling a fixed triangle-wave path far beyond the
//!   world-origin shift threshold, so the origin rebases repeatedly;
//! - a world-partition cell chain along the path with `--stream-cells`
//!   semantics, so cells load/unload around the camera with hysteresis;
//! - per-cell cooked materials that only exist on disk, so first-time cell
//!   loads keep the background asset stream in flight (decodes are reaped
//!   deterministically at the frame boundary).
//!
//! Assertions are regression-oriented rather than absolute budgets: entity
//! and asset counts must plateau after warmup, the process working set must
//! not keep growing once the world reaches steady state, and the rolling
//! frame-time p95 must stay within a generous multiple of its warmup value.
//!
//! The same scenario with a fixed seed runs twice in
//! [`soak_scenario_is_deterministic_across_runs`] and must produce identical
//! report hashes (timing and memory fields excluded).
//!
//! Budget: `SOAK_FRAMES` overrides the frame count (default
//! [`DEFAULT_SOAK_FRAMES`], a few thousand frames — seconds of wall time).
//! Longer runs are opt-in via the env var, never the CI default. The JSON
//! report lands at `target/soak/soak-report.json` (override with
//! `SOAK_REPORT`) so CI can archive it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use engine_asset::cook::cook_material;
use engine_asset::partition::{CellBounds, PartitionCell, WorldPartition, WORLD_PARTITION_SCHEMA};
use engine_asset::project::{GameProject, ProjectManifest};
use engine_core::cell_stream::{CellStreamingConfig, CellStreamingDriver};
use engine_core::game_loop::GameLoop;
use engine_core::{EngineConfig, EngineRuntime};
use engine_renderer::{
    BackendRenderer, Diagnostic, DiagnosticSeverity, FrameStats, MaterialUpload, MeshUpload,
    RenderFrameInput, TextureUpload, UploadReceipt,
};
use engine_scene::{ComponentRecord, EntityRecord, Scene};
use engine_serialize::{AssetId, SchemaVersion, Value};
use serde::Serialize;
use sha2::{Digest, Sha256};

// ── Scenario parameters ─────────────────────────────────────────────────────

/// Default soak length: a few thousand frames, seconds of wall time.
const DEFAULT_SOAK_FRAMES: u64 = 2048;
/// Fixed scenario seed: every fixture detail derived from it (cell jitter,
/// material colors) is identical across runs and machines.
const SOAK_SEED: u64 = 0x0071_5EED_5EA0_C1D2;
/// Frames for one warmup phase: two full patrol periods. The first period
/// streams every cell's assets; the second is the steady-state reference.
const WARMUP_FRAMES: u64 = 160;
/// Shorter fixed length for the in-test determinism double run.
const DETERMINISM_FRAMES: u64 = 480;
/// Patrol end-to-end distance in logical units.
const PATROL_RANGE: f32 = 640.0;
/// Patrol speed in logical units per frame.
const PATROL_SPEED: f32 = 16.0;
/// Frames per full out-and-back patrol period.
const PATROL_PERIOD_FRAMES: u64 = 2 * (PATROL_RANGE as u64) / (PATROL_SPEED as u64);
/// Origin shifting rebases the world every time the camera strays this far
/// from the current origin, so each leg of the patrol shifts repeatedly.
const ORIGIN_SHIFT_THRESHOLD: f32 = 100.0;
/// Streamed cells tiled along the patrol path; the first covers the origin.
const CELL_COUNT: usize = 8;
const CELL_SPACING: f32 = 80.0;
const CELL_HALF_EXTENT: f32 = 40.0;
const CELL_ENTITIES: usize = 2;
/// Working-set sampling cadence (frames between samples).
const WORKING_SET_SAMPLE_INTERVAL: u64 = 64;
/// Memory plateau slack: post-warmup growth allowance of the larger of 50%
/// of the post-warmup baseline or 64 MiB. A genuine leak blows past this on
/// any decently long run; steady-state noise does not.
const MEMORY_GROWTH_FRACTION_DENOMINATOR: u64 = 2;
const MEMORY_GROWTH_FLOOR_BYTES: u64 = 64 * 1024 * 1024;
/// Frame-time regression tripwire: final p95 must stay within this multiple
/// (plus a floor) of the warmup p95. Generous on purpose — this catches
/// runaway growth, not absolute performance.
const TIMING_REGRESSION_FACTOR: f32 = 4.0;
const TIMING_REGRESSION_FLOOR_MS: f32 = 10.0;

fn scenario_lock() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Deterministic PRNG (xorshift64*) for fixture derivation. Nothing in the
/// scenario consumes wall-clock or thread-timing randomness; every varying
/// detail comes from this generator seeded with [`SOAK_SEED`].
struct XorShift64(u64);

impl XorShift64 {
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform-ish f32 in `[min, max)` with 24-bit granularity.
    fn next_range(&mut self, min: f32, max: f32) -> f32 {
        let unit = (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32;
        min + (max - min) * unit
    }
}

/// Camera logical X position: triangle wave between 0 and [`PATROL_RANGE`].
fn patrol_position(frame: u64) -> f32 {
    let distance = (frame % PATROL_PERIOD_FRAMES) as f32 * PATROL_SPEED;
    if distance <= PATROL_RANGE {
        distance
    } else {
        2.0 * PATROL_RANGE - distance
    }
}

// ── Headless backend ────────────────────────────────────────────────────────

/// Minimal headless backend mirroring the QA backend semantics: validates
/// frame ordering, counts forward-pass draw calls and triangles, and accepts
/// every resource upload without owning GPU objects.
#[derive(Default)]
struct SoakBackend {
    frame_active: bool,
    mesh_triangles: BTreeMap<AssetId, u64>,
}

impl SoakBackend {
    fn error(code: &'static str, message: impl Into<String>) -> Vec<Diagnostic> {
        vec![Diagnostic::new(
            code,
            DiagnosticSeverity::Error,
            "sandbox.soak",
            message.into(),
        )]
    }
}

impl BackendRenderer for SoakBackend {
    fn begin_frame(&mut self, _input: &RenderFrameInput) -> Result<(), Vec<Diagnostic>> {
        if self.frame_active {
            return Err(Self::error("SOAK0001", "frame already active"));
        }
        self.frame_active = true;
        Ok(())
    }

    fn apply_pass_barriers(
        &mut self,
        _input: &RenderFrameInput,
        _pass: &engine_renderer::render_graph2::PassNode,
        _barriers: &[engine_renderer::render_graph2::CompiledBarrier],
    ) -> Result<(), Vec<Diagnostic>> {
        Ok(())
    }

    fn execute_pass(
        &mut self,
        input: &RenderFrameInput,
        pass: &engine_renderer::render_graph2::PassNode,
        stats: &mut FrameStats,
    ) -> Result<(), Vec<Diagnostic>> {
        if !self.frame_active {
            return Err(Self::error("SOAK0002", "render pass outside a frame"));
        }
        if pass.kind == engine_renderer::render_graph2::PassKind::OpaquePbrForward {
            let meshes = input
                .drawables
                .iter()
                .map(|item| &item.mesh)
                .chain(input.skinned_items.iter().map(|item| &item.mesh));
            let mut draw_calls = 0u32;
            for mesh in meshes {
                draw_calls = draw_calls.saturating_add(1);
                stats.triangles = stats
                    .triangles
                    .saturating_add(self.mesh_triangles.get(mesh).copied().unwrap_or(0));
            }
            stats.draw_calls = stats.draw_calls.saturating_add(draw_calls);
            stats.visible_drawables = draw_calls;
        }
        Ok(())
    }

    fn end_frame(&mut self, _stats: &mut FrameStats) -> Result<(), Vec<Diagnostic>> {
        if !self.frame_active {
            return Err(Self::error("SOAK0003", "ending an inactive frame"));
        }
        self.frame_active = false;
        Ok(())
    }

    fn abort_frame(&mut self) -> Result<(), Vec<Diagnostic>> {
        self.frame_active = false;
        Ok(())
    }

    fn upload_mesh(&mut self, upload: MeshUpload) -> Result<UploadReceipt, Vec<Diagnostic>> {
        self.mesh_triangles
            .insert(upload.mesh_id, u64::from(upload.index_count / 3));
        Ok(UploadReceipt::new(1))
    }

    fn upload_texture(&mut self, _upload: TextureUpload) -> Result<UploadReceipt, Vec<Diagnostic>> {
        Ok(UploadReceipt::new(1))
    }

    fn upload_material(
        &mut self,
        _upload: MaterialUpload,
    ) -> Result<UploadReceipt, Vec<Diagnostic>> {
        Ok(UploadReceipt::new(1))
    }
}

// ── Fixture ─────────────────────────────────────────────────────────────────

fn component(fields: BTreeMap<String, Value>) -> ComponentRecord {
    ComponentRecord {
        schema_version: SchemaVersion::new(0, 1, 0),
        enabled: true,
        fields,
    }
}

fn transform_component(translation: [f32; 3]) -> ComponentRecord {
    component(BTreeMap::from([
        ("translation".to_string(), Value::Vec3(translation)),
        ("rotation".to_string(), Value::Quat([0.0, 0.0, 0.0, 1.0])),
        ("scale".to_string(), Value::Vec3([1.0, 1.0, 1.0])),
    ]))
}

fn renderable_component(mesh: &str, material: &str) -> ComponentRecord {
    component(BTreeMap::from([
        ("mesh".to_string(), Value::Asset(AssetId::new(mesh))),
        ("material".to_string(), Value::Asset(AssetId::new(material))),
        ("visible".to_string(), Value::Bool(true)),
        (
            "render_layer".to_string(),
            Value::Str("Default".to_string()),
        ),
        ("cast_shadows".to_string(), Value::Bool(true)),
    ]))
}

fn entity(id: &str, components: Vec<(&str, ComponentRecord)>) -> EntityRecord {
    EntityRecord {
        persistent_id: id.to_string(),
        parent: None,
        name: Some(id.to_string()),
        enabled: true,
        components: components
            .into_iter()
            .map(|(type_id, record)| (type_id.to_string(), record))
            .collect(),
    }
}

/// Write a `MaterialSource-v0` next to the cooked output and cook it, so the
/// cell's only runtime copy must arrive through the background asset stream.
fn cook_cell_material(cooked_dir: &Path, id: &str, rng: &mut XorShift64) {
    let color = [
        rng.next_range(0.2, 1.0),
        rng.next_range(0.2, 1.0),
        rng.next_range(0.2, 1.0),
        1.0,
    ];
    let source = cooked_dir.join(format!("{id}.material.json"));
    std::fs::write(
        &source,
        format!(
            "{{\n  \"schema\": \"MaterialSource-v0\",\n  \"base_color\": [{}, {}, {}, {}],\n  \"metallic\": 0.25,\n  \"roughness\": 0.5,\n  \"ambient_occlusion\": 1.0,\n  \"transparency\": \"Opaque\",\n  \"double_sided\": false\n}}\n",
            color[0], color[1], color[2], color[3]
        ),
    )
    .expect("write material source");
    cook_material(&source, &cooked_dir.join(format!("{id}.cooked"))).expect("cook cell material");
}

struct SoakFixture {
    _tempdir: tempfile::TempDir,
    project: GameProject,
    partition: WorldPartition,
    material_ids: Vec<AssetId>,
}

/// Build the soak project: an origin-shifting startup scene with a movable
/// camera, plus a chain of streamed cells along the patrol path whose unique
/// materials exist only as cooked artifacts on disk.
fn build_fixture(seed: u64) -> SoakFixture {
    let mut rng = XorShift64::new(seed);
    let tempdir = tempfile::tempdir().expect("soak fixture tempdir");
    let root = tempdir.path().to_path_buf();
    let scene_dir = root.join("assets/scenes");
    let cooked_dir = root.join("build/cooked");
    std::fs::create_dir_all(&scene_dir).expect("scene directory");
    std::fs::create_dir_all(root.join("assets/source")).expect("source directory");
    std::fs::create_dir_all(&cooked_dir).expect("cooked directory");

    let mut main = engine_scene::sample_scene();
    main.scene_id = "main".to_string();
    main.name = "Soak Main".to_string();
    main.entities = vec![
        entity(
            "camera-main",
            vec![
                ("engine.camera", component(BTreeMap::new())),
                ("engine.transform", transform_component([0.0, 0.0, 0.0])),
            ],
        ),
        entity(
            "cube-home",
            vec![
                ("engine.transform", transform_component([0.0, 0.0, -5.0])),
                (
                    "engine.renderable",
                    renderable_component("mesh-cube", "mat-default"),
                ),
            ],
        ),
    ];
    main.scene_settings.active_camera = Some("camera-main".to_string());
    main.scene_settings.origin_shift.enabled = true;
    main.scene_settings.origin_shift.threshold = ORIGIN_SHIFT_THRESHOLD;
    main.dependencies = vec![];
    main.save_to_file(&scene_dir.join("main.scene.ron"))
        .expect("write startup scene");

    let mut manifest_scenes = BTreeMap::from([(
        "main".to_string(),
        PathBuf::from("assets/scenes/main.scene.ron"),
    )]);
    let mut cells = BTreeMap::new();
    let mut material_ids = Vec::new();

    for index in 0..CELL_COUNT {
        let center = CELL_HALF_EXTENT + index as f32 * CELL_SPACING;
        let material_id = format!("mat-soak-{index}");
        cook_cell_material(&cooked_dir, &material_id, &mut rng);

        let scene_id = format!("cell-{index}");
        let mut scene = engine_scene::sample_scene();
        scene.scene_id = scene_id.clone();
        scene.name = format!("Soak Cell {index}");
        scene.scene_settings.active_camera = None;
        scene.entities = (0..CELL_ENTITIES)
            .map(|cube| {
                let x_offset = rng.next_range(-16.0, 16.0);
                let y = rng.next_range(-2.0, 2.0);
                let z = rng.next_range(-8.0, -4.0);
                entity(
                    &format!("soak-cube-{index}-{cube}"),
                    vec![
                        (
                            "engine.transform",
                            transform_component([center + x_offset, y, z]),
                        ),
                        (
                            "engine.renderable",
                            renderable_component("mesh-cube", &material_id),
                        ),
                    ],
                )
            })
            .collect();
        scene.dependencies = vec![];
        let relative = format!("assets/scenes/{scene_id}.scene.ron");
        scene
            .save_to_file(&root.join(&relative))
            .expect("write cell scene");
        manifest_scenes.insert(scene_id.clone(), PathBuf::from(relative));
        cells.insert(
            format!("cell_{index}"),
            PartitionCell {
                scene: scene_id,
                bounds: CellBounds {
                    center: [center, 0.0, 0.0],
                    half_extents: [CELL_HALF_EXTENT, 10.0, 10.0],
                },
            },
        );
        material_ids.push(AssetId::new(&material_id));
    }

    let mut manifest = ProjectManifest::new("Soak Harness");
    manifest.startup_scene = PathBuf::from("main");
    manifest.input_actions = None;
    manifest.scenes = manifest_scenes;
    manifest
        .write_to_root(&root)
        .expect("write project manifest");
    let project = GameProject::load(&root).expect("load soak fixture project");

    let partition = WorldPartition {
        schema: WORLD_PARTITION_SCHEMA.to_string(),
        cells,
    };
    SoakFixture {
        _tempdir: tempdir,
        project,
        partition,
        material_ids,
    }
}

// ── Working-set sampling ────────────────────────────────────────────────────

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

// ── Scenario ────────────────────────────────────────────────────────────────

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

// ── Report ──────────────────────────────────────────────────────────────────

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

// ── Tests ───────────────────────────────────────────────────────────────────

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
    // equality — not a bound — is the correct check).
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
/// count must produce byte-identical state across two runs — entity
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
