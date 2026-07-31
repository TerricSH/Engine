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
//! [`DEFAULT_SOAK_FRAMES`], a few thousand frames, or seconds of wall time).
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

// Scenario parameters.

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
/// (plus a floor) of the warmup p95. Generous on purpose: this catches
/// runaway growth, not absolute performance.
const TIMING_REGRESSION_FACTOR: f32 = 4.0;
const TIMING_REGRESSION_FLOOR_MS: f32 = 10.0;

include!("soak/fixtures.rs");
include!("soak/metrics.rs");
include!("soak/scenario.rs");
include!("soak/assertions.rs");
