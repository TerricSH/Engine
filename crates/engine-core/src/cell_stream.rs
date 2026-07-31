//! Runtime world-partition cell streaming driver (ENG-02, Phase 4).
//!
//! [`CellStreamingDriver`] turns the declarative
//! [`engine_asset::partition::WorldPartition`] manifest into live world
//! mutations: each frame the host resolves the active camera position, the
//! driver computes the desired cell set with hysteresis, streams each newly
//! desired cell's cooked assets in the background, and commits merges and
//! unloads at the host's existing frame boundary (between `update()` and
//! `render()`, alongside scene-transition processing).
//!
//! Per-cell state machine:
//!
//! ```text
//! Unloaded ──desired──▶ LoadingAssets ──assets committed──▶ Merging ──commit──▶ Loaded
//!    ▲                       │                                 │                │
//!    │                       └──no longer desired (assets done)┘                │ no longer
//!    │                                                                          │ desired
//!    └────────commit──────── Unloading ◀────────────────────────────────────────┘
//! ```
//!
//! `Failed(String)` is a terminal side state entered on scene/asset/merge
//! errors; a failed cell never retries on its own but resets on
//! [`rebaseline`](CellStreamingDriver::rebaseline) (scene replacement).
//!
//! Residency rules (v1):
//!
//! - **Runtime-created entities never unload.** Any persistent entity that
//!   appears in the world without being merged by this driver — script
//!   `Scene.CreateEntity` / `Scene.Spawn` output, host-created helpers — joins
//!   the resident set automatically.
//! - **Entities that leave their authoring cell never unload.** A merged cell
//!   entity whose world-space position moves outside its cell bounds (exit
//!   factor applied) joins the resident set; on unload it is detached from
//!   its cell hierarchy instead of destroyed.
//! - Everything else authored in a cell scene unloads with its cell.
//!
//! Scripts in cells are **forbidden** in this version: cell scenes must not
//! contain `engine.script` components. The rule is enforced by
//! [`validate_partition_cell_scenes`] — called from both this driver and
//! `sandbox project check` — because the per-entity attach/teardown script
//! lifecycle is tied to whole-scene loads. The driver still strips
//! scene-only component records defensively before merging, mirroring
//! [`crate::EngineRuntime::load_scene`].
//!
//! Physics is *not* touched by the driver itself; after a tick that reports
//! [`CellStreamTickReport::world_changed`] the host calls
//! [`crate::game_loop::GameLoop::resync_physics_from_world`], which runs the
//! incremental `PhysicsWorld::sync_from_ecs` — newly merged entities gain
//! bodies, unloaded entities lose theirs, and every other body keeps its
//! simulation state.
//!
//! World-origin shifts (ENG-01 Phase 2) compose with streaming: partition
//! bounds and cell scene data are authored in logical coordinates, so the
//! driver lifts origin-relative world positions by
//! [`engine_scene::World::world_origin`] for every bounds test and rebases merged
//! hierarchy roots by `-origin`. World-space component data other than
//! `Transform` inside cell scenes (for example `engine.nav_agent` targets
//! or `engine.gravity_source` centers) is **not** rebased on merge in this
//! version.
//!
//! v1 limitations (documented in `docs/GAME_PROJECTS.md`):
//!
//! - Shared cooked assets are never unloaded; there is no per-cell asset
//!   reference counting yet.
//! - Once a cell's asset stream is enqueued it is never cancelled; leaving
//!   the cell's bounds before the merge only skips the merge.
//! - A scene transition replaces the whole world; the driver rebaselines and
//!   starts streaming from scratch around the new scene.
//! - Save/rollback semantics are whole-scene only; a failed scene transition
//!   rolls back to the retained scene snapshot exactly as without streaming.

use glam::Vec3;

/// The f32 origin-relative offset corresponding to a world origin.
///
/// Streaming decisions run in logical (authored) space, so origin-relative
/// positions from the world are lifted by this offset before they are
/// compared against partition bounds.
fn origin_offset(origin: [f64; 3]) -> Vec3 {
    Vec3::new(origin[0] as f32, origin[1] as f32, origin[2] as f32)
}

mod config;
mod driver;
mod state;
mod validation;
mod world_positions;

pub use config::{
    CellState, CellStreamError, CellStreamingConfig, DEFAULT_ENTER_FACTOR, DEFAULT_EXIT_FACTOR,
    DEFAULT_MAX_MERGES_PER_COMMIT, DEFAULT_MAX_UNLOADS_PER_COMMIT,
};
pub use driver::CellStreamingDriver;
pub use state::CellStreamTickReport;
pub use validation::validate_partition_cell_scenes;

#[cfg(test)]
#[path = "cell_stream/tests.rs"]
mod tests;
