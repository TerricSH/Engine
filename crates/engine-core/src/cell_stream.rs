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
//! [`EngineRuntime::load_scene`].
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
//! [`World::world_origin`] for every bounds test and rebases merged
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

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::PathBuf;

use engine_asset::partition::{CellBounds, WorldPartition};
use engine_asset::project::GameProject;
use engine_scene::components::Transform;
use engine_scene::{
    active_camera_world_position, validate_scene, Entity, Scene, World, SCENE_ONLY_COMPONENT_TYPES,
};
use engine_serialize::{AssetId, Diagnostic, DiagnosticSeverity, PersistentId};
use glam::Vec3;

use crate::EngineRuntime;

/// The f32 origin-relative offset corresponding to a world origin.
///
/// Streaming decisions run in logical (authored) space, so origin-relative
/// positions from the world are lifted by this offset before they are
/// compared against partition bounds.
fn origin_offset(origin: [f64; 3]) -> Vec3 {
    Vec3::new(origin[0] as f32, origin[1] as f32, origin[2] as f32)
}

/// Default enter factor: the camera must be inside `bounds * 1.0`.
pub const DEFAULT_ENTER_FACTOR: f32 = 1.0;
/// Default exit factor: a loaded cell stays until the camera leaves
/// `bounds * 1.15`.
pub const DEFAULT_EXIT_FACTOR: f32 = 1.15;
/// Default maximum number of cell merges committed per frame-boundary tick.
pub const DEFAULT_MAX_MERGES_PER_COMMIT: usize = 1;
/// Default maximum number of cell unloads committed per frame-boundary tick.
pub const DEFAULT_MAX_UNLOADS_PER_COMMIT: usize = 4;

/// Tunables of a [`CellStreamingDriver`].
///
/// Hysteresis: an unloaded cell becomes desired when the camera enters
/// `bounds * enter_factor`; a loaded (or loading) cell stops being desired
/// only when the camera leaves `bounds * exit_factor`. Keeping
/// `exit_factor >= enter_factor` prevents boundary ping-ponging.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellStreamingConfig {
    /// Bounds scale at which an unloaded cell becomes desired. Must be
    /// finite and greater than zero.
    pub enter_factor: f32,
    /// Bounds scale at which a loaded cell stops being desired. Must be
    /// finite and not smaller than `enter_factor`.
    pub exit_factor: f32,
    /// Maximum cell merges committed per tick. Zero is clamped to one.
    pub max_merges_per_commit: usize,
    /// Maximum cell unloads committed per tick. Zero is clamped to one.
    pub max_unloads_per_commit: usize,
}

impl Default for CellStreamingConfig {
    fn default() -> Self {
        Self {
            enter_factor: DEFAULT_ENTER_FACTOR,
            exit_factor: DEFAULT_EXIT_FACTOR,
            max_merges_per_commit: DEFAULT_MAX_MERGES_PER_COMMIT,
            max_unloads_per_commit: DEFAULT_MAX_UNLOADS_PER_COMMIT,
        }
    }
}

impl CellStreamingConfig {
    fn validated(self) -> Result<Self, CellStreamError> {
        if !self.enter_factor.is_finite() || self.enter_factor <= 0.0 {
            return Err(CellStreamError::InvalidConfig(format!(
                "enter_factor must be finite and greater than zero, got {}",
                self.enter_factor
            )));
        }
        if !self.exit_factor.is_finite() || self.exit_factor < self.enter_factor {
            return Err(CellStreamError::InvalidConfig(format!(
                "exit_factor must be finite and not smaller than enter_factor ({}), got {}",
                self.enter_factor, self.exit_factor
            )));
        }
        Ok(Self {
            max_merges_per_commit: self.max_merges_per_commit.max(1),
            max_unloads_per_commit: self.max_unloads_per_commit.max(1),
            ..self
        })
    }
}

/// Streaming state of one partition cell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CellState {
    /// Not in the world and no load in flight.
    Unloaded,
    /// Cooked assets are decoding/committing on the background stream.
    LoadingAssets,
    /// Assets are committed; the scene merge waits for the commit budget.
    Merging,
    /// Merged into the live world.
    Loaded,
    /// Queued for destruction at the commit budget.
    Unloading,
    /// Terminal error state until the next rebaseline; carries the reason.
    Failed(String),
}

/// Failure returned when constructing a [`CellStreamingDriver`] or validating
/// partition cell scenes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CellStreamError {
    /// The hysteresis factors or budgets are invalid.
    InvalidConfig(String),
    /// A cell references a scene that is not in the project catalog.
    UnknownCellScene { cell_id: String, scene_id: String },
    /// A cell scene file could not be read or parsed.
    CellSceneLoad {
        cell_id: String,
        scene_id: String,
        message: String,
    },
    /// A cell scene failed the standard scene validation.
    CellSceneInvalid {
        cell_id: String,
        scene_id: String,
        messages: Vec<String>,
    },
    /// A cell scene contains an `engine.script` component (forbidden in v1).
    ScriptComponentInCell { cell_id: String, entity_id: String },
    /// Two cells contain the same persistent entity ID.
    DuplicatePersistentIdAcrossCells {
        first_cell: String,
        second_cell: String,
        persistent_id: PersistentId,
    },
    /// A cell that does not reference the startup scene shares persistent
    /// entity IDs with it.
    StartupSceneIdConflict {
        cell_id: String,
        persistent_id: PersistentId,
        startup_scene_id: String,
    },
}

impl std::fmt::Display for CellStreamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidConfig(reason) => {
                write!(f, "invalid cell streaming configuration: {reason}")
            }
            Self::UnknownCellScene { cell_id, scene_id } => write!(
                f,
                "world partition cell \"{cell_id}\" references scene \"{scene_id}\", which is not in the project scene catalog"
            ),
            Self::CellSceneLoad {
                cell_id,
                scene_id,
                message,
            } => write!(
                f,
                "world partition cell \"{cell_id}\" scene \"{scene_id}\" could not be loaded: {message}"
            ),
            Self::CellSceneInvalid {
                cell_id,
                scene_id,
                messages,
            } => write!(
                f,
                "world partition cell \"{cell_id}\" scene \"{scene_id}\" is invalid:\n{}",
                messages.join("\n")
            ),
            Self::ScriptComponentInCell { cell_id, entity_id } => write!(
                f,
                "world partition cell \"{cell_id}\" entity \"{entity_id}\" has an engine.script component; scripts in partition cells are not supported (attach scripts in the startup scene instead)"
            ),
            Self::DuplicatePersistentIdAcrossCells {
                first_cell,
                second_cell,
                persistent_id,
            } => write!(
                f,
                "world partition cells \"{first_cell}\" and \"{second_cell}\" both contain persistent entity id \"{persistent_id}\"; cell scene entity ids must be unique across cells"
            ),
            Self::StartupSceneIdConflict {
                cell_id,
                persistent_id,
                startup_scene_id,
            } => write!(
                f,
                "world partition cell \"{cell_id}\" shares persistent entity id \"{persistent_id}\" with startup scene \"{startup_scene_id}\"; a cell that reuses startup content must reference the startup scene itself"
            ),
        }
    }
}

impl std::error::Error for CellStreamError {}

/// Validate partition cell scenes for streaming compatibility.
///
/// This is the shared rule set enforced by `sandbox project check` and by
/// [`CellStreamingDriver::new`]:
///
/// - every cell's scene must be present in `scenes` (keyed by scene ID),
/// - cell scenes must not contain `engine.script` components,
/// - persistent entity IDs must be unique **across** cells (a load-time
///   merge would fail otherwise), and
/// - a cell that does not reference the startup scene must not share
///   persistent entity IDs with it (the startup scene is already live when
///   streaming begins; a cell that intentionally reuses the startup scene's
///   content must reference the startup scene itself, in which case the
///   driver adopts the already-live entities).
pub fn validate_partition_cell_scenes(
    partition: &WorldPartition,
    startup_scene_id: &str,
    scenes: &BTreeMap<String, &Scene>,
) -> Result<(), CellStreamError> {
    let mut owner_of: BTreeMap<&str, &str> = BTreeMap::new();
    for (cell_id, cell) in &partition.cells {
        let Some(scene) = scenes.get(&cell.scene).copied() else {
            return Err(CellStreamError::UnknownCellScene {
                cell_id: cell_id.clone(),
                scene_id: cell.scene.clone(),
            });
        };
        for entity in &scene.entities {
            if entity.components.contains_key("engine.script") {
                return Err(CellStreamError::ScriptComponentInCell {
                    cell_id: cell_id.clone(),
                    entity_id: entity.persistent_id.clone(),
                });
            }
            let persistent_id = entity.persistent_id.as_str();
            if let Some(first_cell) = owner_of.insert(persistent_id, cell_id) {
                if first_cell != cell_id {
                    return Err(CellStreamError::DuplicatePersistentIdAcrossCells {
                        first_cell: first_cell.to_string(),
                        second_cell: cell_id.clone(),
                        persistent_id: entity.persistent_id.clone(),
                    });
                }
            }
        }
        if cell.scene != startup_scene_id {
            if let Some(startup_scene) = scenes.get(startup_scene_id).copied() {
                let startup_ids: BTreeSet<&str> = startup_scene
                    .entities
                    .iter()
                    .map(|entity| entity.persistent_id.as_str())
                    .collect();
                if let Some(conflict) = scene
                    .entities
                    .iter()
                    .map(|entity| entity.persistent_id.as_str())
                    .find(|id| startup_ids.contains(id))
                {
                    return Err(CellStreamError::StartupSceneIdConflict {
                        cell_id: cell_id.clone(),
                        persistent_id: conflict.to_string(),
                        startup_scene_id: startup_scene_id.to_string(),
                    });
                }
            }
        }
    }
    Ok(())
}

/// Outcome of one [`CellStreamingDriver::tick`] call.
#[derive(Clone, Debug, Default)]
pub struct CellStreamTickReport {
    /// Camera position used for the desired-set computation, if any.
    pub camera: Option<[f32; 3]>,
    /// Cells the hysteresis evaluation wants live right now.
    pub desired_cells: BTreeSet<String>,
    /// Cooked assets enqueued on the background stream during this tick.
    pub enqueued_assets: usize,
    /// Cells whose scene merge was committed during this tick.
    pub merged_cells: Vec<String>,
    /// Cells whose unload was committed during this tick.
    pub unloaded_cells: Vec<String>,
    /// Cells that entered the failed state during this tick.
    pub failed_cells: Vec<String>,
    /// Persistent IDs that joined the resident set during this tick.
    pub resident_ids_added: Vec<PersistentId>,
}

impl CellStreamTickReport {
    /// The live world changed (a merge or an unload was committed), so the
    /// host should re-synchronise derived state such as the physics world.
    pub fn world_changed(&self) -> bool {
        !self.merged_cells.is_empty() || !self.unloaded_cells.is_empty()
    }
}

struct CellRecord {
    bounds: CellBounds,
    scene: Scene,
    state: CellState,
    /// Asset IDs enqueued for this cell and not yet observed as installed.
    pending_assets: BTreeSet<AssetId>,
    /// Persistent IDs currently live in the world from this cell's merges.
    merged_ids: Vec<PersistentId>,
    /// Cell scene hierarchy roots: records with no parent or with a parent
    /// outside the cell scene's own ID set.
    root_ids: Vec<PersistentId>,
}

/// Runtime streaming driver for a project's [`WorldPartition`].
///
/// Construct once the startup scene is live, call
/// [`rebaseline`](Self::rebaseline) initially and after every scene
/// replacement, then [`tick`](Self::tick) once per frame at the frame
/// boundary. All world mutations (merges and subtree destroys) happen inside
/// `tick`, never mid-frame.
pub struct CellStreamingDriver {
    config: CellStreamingConfig,
    cells: BTreeMap<String, CellRecord>,
    cooked_dir: PathBuf,
    /// Persistent IDs that must never unload (runtime-created or moved out).
    resident: BTreeSet<PersistentId>,
    /// Persistent IDs accounted for: live world IDs at the last rebaseline
    /// plus every ID the driver merged since. Anything else appearing in the
    /// world was created at runtime and becomes resident.
    known_ids: BTreeSet<PersistentId>,
    /// World IDs that belong to the loaded scene itself rather than to any
    /// adopted cell; retained for introspection and future save semantics.
    base_ids: BTreeSet<PersistentId>,
    total_merges: u64,
    total_unloads: u64,
}

impl CellStreamingDriver {
    /// Build a driver for `partition` against `project`.
    ///
    /// Every cell scene is loaded up front (so ticks never perform file I/O),
    /// checked with the standard scene validation, and verified against
    /// [`validate_partition_cell_scenes`]. Cooked asset paths resolve to
    /// `<project cooked_assets dir>/<asset-id>.cooked`, the same naming the
    /// cooker and the whole-directory runtime loader use.
    pub fn new(
        partition: &WorldPartition,
        project: &GameProject,
        config: CellStreamingConfig,
    ) -> Result<Self, CellStreamError> {
        let config = config.validated()?;
        let mut scenes: BTreeMap<String, Scene> = BTreeMap::new();
        for (cell_id, cell) in &partition.cells {
            let Some(path) = project.scene_path(&cell.scene) else {
                return Err(CellStreamError::UnknownCellScene {
                    cell_id: cell_id.clone(),
                    scene_id: cell.scene.clone(),
                });
            };
            if scenes.contains_key(&cell.scene) {
                continue;
            }
            let scene =
                Scene::load_from_file(&path).map_err(|error| CellStreamError::CellSceneLoad {
                    cell_id: cell_id.clone(),
                    scene_id: cell.scene.clone(),
                    message: format!("{}: {error}", path.display()),
                })?;
            let messages = validate_scene(&scene)
                .into_iter()
                .filter(|diagnostic| {
                    matches!(
                        diagnostic.severity,
                        DiagnosticSeverity::Error | DiagnosticSeverity::Fatal
                    )
                })
                .map(|diagnostic| format!("{}: {}", diagnostic.code, diagnostic.message))
                .collect::<Vec<_>>();
            if !messages.is_empty() {
                return Err(CellStreamError::CellSceneInvalid {
                    cell_id: cell_id.clone(),
                    scene_id: cell.scene.clone(),
                    messages,
                });
            }
            scenes.insert(cell.scene.clone(), scene);
        }
        let scene_refs = scenes
            .iter()
            .map(|(id, scene)| (id.clone(), scene))
            .collect::<BTreeMap<String, &Scene>>();
        validate_partition_cell_scenes(partition, project.startup_scene_id(), &scene_refs)?;

        let cells = partition
            .cells
            .iter()
            .map(|(cell_id, cell)| {
                let scene = scenes
                    .get(&cell.scene)
                    .expect("every cell scene was loaded above")
                    .clone();
                let record = CellRecord {
                    bounds: cell.bounds,
                    root_ids: cell_root_ids(&scene),
                    scene,
                    state: CellState::Unloaded,
                    pending_assets: BTreeSet::new(),
                    merged_ids: Vec::new(),
                };
                (cell_id.clone(), record)
            })
            .collect();

        Ok(Self {
            config,
            cells,
            cooked_dir: project.cooked_assets.clone(),
            resident: BTreeSet::new(),
            known_ids: BTreeSet::new(),
            base_ids: BTreeSet::new(),
            total_merges: 0,
            total_unloads: 0,
        })
    }

    /// Re-baseline the driver against the runtime's current world.
    ///
    /// Call once after the startup scene is live and after every scene
    /// replacement (a `Scene.Load` transition). A cell whose entire entity
    /// set is already live — typically the startup scene referenced as a
    /// cell — is adopted as `Loaded` and streams like any other cell; every
    /// other cell resets to `Unloaded`, in-flight asset bookkeeping is
    /// dropped (already-enqueued assets still commit additively), failed
    /// cells get a fresh start, and the resident set is cleared because the
    /// world it referred to is gone.
    pub fn rebaseline(&mut self, runtime: &EngineRuntime) {
        let world_ids: BTreeSet<PersistentId> = runtime
            .with_world(|world| {
                world
                    .persistent_entities()
                    .map(|(id, _)| id.to_owned())
                    .collect()
            })
            .unwrap_or_default();

        self.resident.clear();
        for record in self.cells.values_mut() {
            record.pending_assets.clear();
            record.merged_ids.clear();
            let adopted = !record.scene.entities.is_empty()
                && record
                    .scene
                    .entities
                    .iter()
                    .all(|entity| world_ids.contains(&entity.persistent_id));
            if adopted {
                record.state = CellState::Loaded;
                record.merged_ids = record
                    .scene
                    .entities
                    .iter()
                    .map(|entity| entity.persistent_id.clone())
                    .collect();
            } else {
                record.state = CellState::Unloaded;
            }
        }
        let merged: BTreeSet<&PersistentId> = self
            .cells
            .values()
            .flat_map(|record| record.merged_ids.iter())
            .collect();
        self.base_ids = world_ids
            .iter()
            .filter(|id| !merged.contains(*id))
            .cloned()
            .collect();
        self.known_ids = world_ids;
    }

    /// Current state of one cell, if the ID exists.
    pub fn cell_state(&self, cell_id: &str) -> Option<&CellState> {
        self.cells.get(cell_id).map(|record| &record.state)
    }

    /// Snapshot of every cell's state in deterministic cell-ID order.
    pub fn cell_states(&self) -> BTreeMap<String, CellState> {
        self.cells
            .iter()
            .map(|(cell_id, record)| (cell_id.clone(), record.state.clone()))
            .collect()
    }

    /// Cells currently merged into the live world (including adopted ones).
    pub fn loaded_cells(&self) -> Vec<String> {
        self.cells
            .iter()
            .filter(|(_, record)| record.state == CellState::Loaded)
            .map(|(cell_id, _)| cell_id.clone())
            .collect()
    }

    /// Persistent IDs that never unload (runtime-created or moved out).
    pub fn resident_ids(&self) -> &BTreeSet<PersistentId> {
        &self.resident
    }

    /// World IDs that belong to the loaded scene rather than any cell.
    pub fn base_ids(&self) -> &BTreeSet<PersistentId> {
        &self.base_ids
    }

    /// Total cell merges committed since construction.
    pub fn total_merges(&self) -> u64 {
        self.total_merges
    }

    /// Total cell unloads committed since construction.
    pub fn total_unloads(&self) -> u64 {
        self.total_unloads
    }

    /// Advance streaming by one frame-boundary tick.
    ///
    /// Intended call site: after `GameLoop::update` and scene-transition
    /// processing, before `GameLoop::render`. The tick drains the background
    /// asset stream, refreshes the resident set, recomputes the desired cell
    /// set from the active camera with hysteresis, starts asset streams for
    /// newly desired cells, and commits queued unloads and merges under the
    /// configured budgets. With no active camera the tick only drains the
    /// asset stream.
    pub fn tick(&mut self, runtime: &mut EngineRuntime) -> CellStreamTickReport {
        let mut report = CellStreamTickReport::default();

        // Always drain the background stream (a cheap no-op when idle) so
        // in-flight cells progress and their assets commit additively.
        runtime.drain_cooked_asset_stream();

        let camera = runtime
            .with_world(|world| {
                // Streaming decisions run in logical space: partition bounds
                // are authored logical coordinates, while world positions are
                // origin-relative after a world-origin shift (ENG-01).
                active_camera_world_position(world)
                    .map(|camera| camera + origin_offset(world.world_origin()))
            })
            .flatten();
        let Some(camera) = camera else {
            return report;
        };
        report.camera = Some(camera.to_array());

        self.update_residency(runtime, &mut report);

        let desired = self.compute_desired(camera);
        report.desired_cells = desired.clone();

        // Start loads for newly desired cells.
        let to_load = self
            .cells
            .iter()
            .filter(|(cell_id, record)| {
                record.state == CellState::Unloaded && desired.contains(*cell_id)
            })
            .map(|(cell_id, _)| cell_id.clone())
            .collect::<Vec<_>>();
        for cell_id in to_load {
            self.start_cell_load(&cell_id, runtime, &mut report);
        }

        // Progress in-flight loads against the freshly drained stream.
        let loading = self
            .cells
            .iter()
            .filter(|(_, record)| record.state == CellState::LoadingAssets)
            .map(|(cell_id, _)| cell_id.clone())
            .collect::<Vec<_>>();
        for cell_id in loading {
            self.refresh_loading_cell(&cell_id, runtime, &desired, &mut report);
        }

        // Cancel queued commits whose cell is desired again (unload) or no
        // longer desired (merge).
        for (cell_id, record) in &mut self.cells {
            match record.state {
                CellState::Merging if !desired.contains(cell_id) => {
                    record.state = CellState::Unloaded;
                }
                CellState::Unloading if desired.contains(cell_id) => {
                    record.state = CellState::Loaded;
                }
                _ => {}
            }
        }

        // Queue loaded cells the camera has left for unload, then commit
        // unloads before merges so freed IDs and budget favour departures.
        let to_unload = self
            .cells
            .iter()
            .filter(|(cell_id, record)| {
                record.state == CellState::Loaded && !desired.contains(*cell_id)
            })
            .map(|(cell_id, _)| cell_id.clone())
            .collect::<Vec<_>>();
        for cell_id in to_unload {
            if let Some(record) = self.cells.get_mut(&cell_id) {
                record.state = CellState::Unloading;
            }
        }
        self.commit_unloads(runtime, &mut report);
        self.commit_merges(runtime, &mut report);

        report
    }

    /// Whether `point` lies inside `bounds` scaled by `factor` per axis.
    fn inside_bounds(bounds: &CellBounds, factor: f32, point: Vec3) -> bool {
        (0..3).all(|axis| {
            let half_extent = bounds.half_extents[axis] * factor;
            (point[axis] - bounds.center[axis]).abs() <= half_extent
        })
    }

    /// Cells the camera wants live: unloaded cells enter at
    /// `enter_factor`, every in-flight/live cell leaves at `exit_factor`.
    fn compute_desired(&self, camera: Vec3) -> BTreeSet<String> {
        self.cells
            .iter()
            .filter(|(_, record)| {
                let factor = match record.state {
                    CellState::Unloaded | CellState::Failed(_) => self.config.enter_factor,
                    CellState::LoadingAssets
                    | CellState::Merging
                    | CellState::Loaded
                    | CellState::Unloading => self.config.exit_factor,
                };
                Self::inside_bounds(&record.bounds, factor, camera)
            })
            .map(|(cell_id, _)| cell_id.clone())
            .collect()
    }

    /// Refresh the resident set: adopt runtime-created entities and protect
    /// merged cell entities that moved out of their authoring cell bounds.
    fn update_residency(&mut self, runtime: &EngineRuntime, report: &mut CellStreamTickReport) {
        let Some((new_ids, moved_out)) = runtime.with_world(|world| {
            let origin = origin_offset(world.world_origin());
            let world_ids: Vec<PersistentId> = world
                .persistent_entities()
                .map(|(id, _)| id.to_owned())
                .collect();
            let new_ids = world_ids
                .into_iter()
                .filter(|id| !self.known_ids.contains(id) && !self.resident.contains(id))
                .collect::<Vec<_>>();

            let mut positions = WorldPositions::default();
            let mut moved_out = Vec::new();
            for record in self.cells.values() {
                if record.state != CellState::Loaded {
                    continue;
                }
                for id in &record.merged_ids {
                    if self.resident.contains(id) {
                        continue;
                    }
                    let Some(entity) = world.entity_by_persistent_id(id) else {
                        continue;
                    };
                    let Some(position) = positions.position(world, entity) else {
                        continue;
                    };
                    // Compare in logical space: bounds are authored logical
                    // coordinates, positions from the world are
                    // origin-relative after a world-origin shift.
                    let position = position + origin;
                    if !Self::inside_bounds(&record.bounds, self.config.exit_factor, position) {
                        moved_out.push(id.clone());
                    }
                }
            }
            (new_ids, moved_out)
        }) else {
            return;
        };

        for id in new_ids.into_iter().chain(moved_out.into_iter()) {
            if self.resident.insert(id.clone()) {
                self.known_ids.insert(id.clone());
                report.resident_ids_added.push(id);
            }
        }
    }

    /// Transition a cell to `LoadingAssets` (or straight to `Merging` when
    /// every dependency is already installed), enqueueing the missing cooked
    /// artifacts on the background stream.
    fn start_cell_load(
        &mut self,
        cell_id: &str,
        runtime: &mut EngineRuntime,
        report: &mut CellStreamTickReport,
    ) {
        let pending_elsewhere: BTreeSet<AssetId> = self
            .cells
            .iter()
            .filter(|(other_id, _)| other_id.as_str() != cell_id)
            .flat_map(|(_, record)| record.pending_assets.iter().cloned())
            .collect();
        let record = self
            .cells
            .get_mut(cell_id)
            .expect("cell IDs come from the driver map");

        let mut wanted = record.scene.collect_asset_dependencies();
        wanted.extend(record.scene.dependencies.iter().cloned());
        wanted.sort();
        wanted.dedup();

        let mut paths = Vec::new();
        let mut missing_files = Vec::new();
        for id in &wanted {
            if runtime.asset_registry().contains(id) {
                // Already installed — engine builtin, startup content, or an
                // asset shared with another cell. v1 never re-streams or
                // unloads shared assets.
                continue;
            }
            record.pending_assets.insert(id.clone());
            if pending_elsewhere.contains(id) {
                // Another cell's stream already covers this artifact; wait
                // for it instead of double-decoding the same file.
                continue;
            }
            let path = self.cooked_dir.join(format!("{}.cooked", id.id));
            if path.is_file() {
                paths.push(path);
            } else {
                missing_files.push(format!("{} ({})", id.id, path.display()));
            }
        }

        if !missing_files.is_empty() {
            record.pending_assets.clear();
            self.fail_cell(
                cell_id,
                format!(
                    "cell references assets with no cooked artifact; run `sandbox project cook`: {}",
                    missing_files.join(", ")
                ),
                runtime,
                report,
            );
            return;
        }

        if !paths.is_empty() {
            report.enqueued_assets += runtime.enqueue_cooked_asset_stream(paths);
        }
        let record = self.cells.get_mut(cell_id).expect("cell exists");
        record.state = if record.pending_assets.is_empty() {
            CellState::Merging
        } else {
            CellState::LoadingAssets
        };
    }

    /// Advance a `LoadingAssets` cell: once every awaited asset is installed
    /// the cell either proceeds to `Merging` (still desired) or returns to
    /// `Unloaded` (the camera left while assets streamed — the merge is
    /// skipped but committed assets stay installed, documented v1 behaviour).
    fn refresh_loading_cell(
        &mut self,
        cell_id: &str,
        runtime: &mut EngineRuntime,
        desired: &BTreeSet<String>,
        report: &mut CellStreamTickReport,
    ) {
        {
            let record = self.cells.get_mut(cell_id).expect("cell exists");
            record
                .pending_assets
                .retain(|id| !runtime.asset_registry().contains(id));
        }
        let record = &self.cells[cell_id];
        if record.pending_assets.is_empty() {
            let still_desired = desired.contains(cell_id);
            let record = self.cells.get_mut(cell_id).expect("cell exists");
            record.state = if still_desired {
                CellState::Merging
            } else {
                CellState::Unloaded
            };
        } else if runtime.cooked_asset_stream_pending() == 0 {
            let missing = record
                .pending_assets
                .iter()
                .map(|id| id.id.clone())
                .collect::<Vec<_>>()
                .join(", ");
            self.fail_cell(
                cell_id,
                format!(
                    "cell assets failed to decode or commit on the background stream: {missing}"
                ),
                runtime,
                report,
            );
        }
    }

    /// Commit queued unloads under the per-tick budget.
    fn commit_unloads(&mut self, runtime: &mut EngineRuntime, report: &mut CellStreamTickReport) {
        let unloading = self
            .cells
            .iter()
            .filter(|(_, record)| record.state == CellState::Unloading)
            .map(|(cell_id, _)| cell_id.clone())
            .collect::<Vec<_>>();
        for cell_id in unloading
            .into_iter()
            .take(self.config.max_unloads_per_commit)
        {
            self.commit_unload(&cell_id, runtime);
            report.unloaded_cells.push(cell_id);
        }
    }

    /// Destroy one cell's subtrees, sparing resident entities: a resident
    /// entity whose effective parent is itself resident keeps its hierarchy
    /// link; any other resident is detached to a root first so the subtree
    /// destroy cannot reach it.
    fn commit_unload(&mut self, cell_id: &str, runtime: &mut EngineRuntime) {
        let (root_ids, merged_ids) = {
            let record = &self.cells[cell_id];
            (record.root_ids.clone(), record.merged_ids.clone())
        };

        runtime.with_world_mut(|world| {
            for id in &merged_ids {
                if !self.resident.contains(id) {
                    continue;
                }
                let Some(entity) = world.entity_by_persistent_id(id) else {
                    continue;
                };
                let parent_is_resident = world
                    .parent_persistent_id(entity)
                    .is_some_and(|parent| self.resident.contains(parent));
                if !parent_is_resident {
                    world.detach_from_parent(entity);
                }
            }
            let destroy_roots = root_ids
                .iter()
                .filter(|id| !self.resident.contains(*id))
                .filter(|id| world.entity_by_persistent_id(id).is_some())
                .cloned()
                .collect::<Vec<_>>();
            // Every root resolves by construction; IDs a script destroyed at
            // runtime are filtered above, so this cannot fail.
            let _ = world.destroy_subtree_by_persistent_ids(&destroy_roots);
        });

        // IDs the destroy removed leave the known set unless they survived
        // as residents (script recreations of a cell ID are out of scope).
        let record = self.cells.get_mut(cell_id).expect("cell exists");
        record.merged_ids.clear();
        record.state = CellState::Unloaded;
        self.total_unloads += 1;
    }

    /// Commit queued merges under the per-tick budget.
    fn commit_merges(&mut self, runtime: &mut EngineRuntime, report: &mut CellStreamTickReport) {
        let merging = self
            .cells
            .iter()
            .filter(|(_, record)| record.state == CellState::Merging)
            .map(|(cell_id, _)| cell_id.clone())
            .collect::<Vec<_>>();
        for cell_id in merging.into_iter().take(self.config.max_merges_per_commit) {
            self.commit_merge(&cell_id, runtime, report);
        }
    }

    /// Merge one cell's scene into the live world. Resident IDs from an
    /// earlier unload cycle are filtered out of the merge (they still live
    /// in the world with their current runtime state); remaining records may
    /// re-parent onto them by persistent ID.
    fn commit_merge(
        &mut self,
        cell_id: &str,
        runtime: &mut EngineRuntime,
        report: &mut CellStreamTickReport,
    ) {
        let mut scene = {
            let record = &self.cells[cell_id];
            record.scene.clone()
        };
        scene
            .entities
            .retain(|entity| !self.resident.contains(&entity.persistent_id));
        for entity in &mut scene.entities {
            // Defensive: validation forbids engine.script in cell scenes,
            // but scene-only records are never mergeable regardless.
            entity
                .components
                .retain(|type_id, _| !SCENE_ONLY_COMPONENT_TYPES.contains(&type_id.as_str()));
        }

        let merged = runtime.with_world_mut(|world| world.merge_scene(&scene));
        match merged {
            Some(Ok(_)) => {
                Self::rebase_merged_roots(runtime, &scene);
                let record = self.cells.get_mut(cell_id).expect("cell exists");
                record.merged_ids = scene
                    .entities
                    .iter()
                    .map(|entity| entity.persistent_id.clone())
                    .collect();
                record.state = CellState::Loaded;
                for id in &record.merged_ids {
                    self.known_ids.insert(id.clone());
                }
                self.total_merges += 1;
                report.merged_cells.push(cell_id.to_string());
            }
            Some(Err(error)) => {
                self.fail_cell(
                    cell_id,
                    format!("cell scene merge failed: {error}"),
                    runtime,
                    report,
                );
            }
            None => {
                self.fail_cell(
                    cell_id,
                    "cell scene merge failed: the runtime has no active world".to_string(),
                    runtime,
                    report,
                );
            }
        }
    }

    /// Rebase freshly merged cell content into the world's origin-relative
    /// frame.
    ///
    /// Cell scenes are authored in logical coordinates, so while the world
    /// origin is non-zero (ENG-01 Phase 2) the merged hierarchy roots must be
    /// translated by `-origin` to land on their authored logical positions.
    /// Entities parented onto already-live residents keep their authored
    /// local translation: their parent's world position already accounts for
    /// the origin.
    fn rebase_merged_roots(runtime: &EngineRuntime, scene: &Scene) {
        let Some(origin) = runtime.with_world(|world| world.world_origin()) else {
            return;
        };
        if origin == [0.0; 3] {
            return;
        }
        let offset = origin_offset(origin);
        runtime.with_world_mut(|world| {
            for record in &scene.entities {
                let Some(entity) = world.entity_by_persistent_id(&record.persistent_id) else {
                    continue;
                };
                let Some(transform) = world.get_mut::<Transform>(entity) else {
                    continue;
                };
                if transform.parent.is_none() {
                    transform.translation -= offset;
                }
            }
        });
    }

    /// Move a cell into the terminal failed state and surface a diagnostic.
    fn fail_cell(
        &mut self,
        cell_id: &str,
        reason: String,
        runtime: &mut EngineRuntime,
        report: &mut CellStreamTickReport,
    ) {
        let record = self.cells.get_mut(cell_id).expect("cell exists");
        record.pending_assets.clear();
        record.state = CellState::Failed(reason.clone());
        report.failed_cells.push(cell_id.to_string());
        runtime
            .diagnostics_collector_mut()
            .push_scene_diags(vec![Diagnostic::new(
                "CELL_STREAM",
                DiagnosticSeverity::Error,
                "engine-core.cell-stream",
                format!("world partition cell \"{cell_id}\": {reason}"),
            )]);
    }
}

/// Hierarchy roots of a cell scene: entity records with no parent or with a
/// parent outside the scene's own persistent-ID set.
fn cell_root_ids(scene: &Scene) -> Vec<PersistentId> {
    let own_ids: BTreeSet<&str> = scene
        .entities
        .iter()
        .map(|entity| entity.persistent_id.as_str())
        .collect();
    scene
        .entities
        .iter()
        .filter(|entity| {
            entity
                .parent
                .as_deref()
                .is_none_or(|parent| !own_ids.contains(parent))
        })
        .map(|entity| entity.persistent_id.clone())
        .collect()
}

/// Memoised world-space positions for `Transform`-bearing entities, resolved
/// by walking `Transform.parent` chains. Entities without a Transform resolve
/// to `None`; an ancestor without a Transform counts as an identity root and
/// a parent cycle breaks the chain, mirroring extraction's tolerance.
#[derive(Default)]
struct WorldPositions {
    resolved: HashMap<Entity, Option<Vec3>>,
}

impl WorldPositions {
    fn position(&mut self, world: &World, entity: Entity) -> Option<Vec3> {
        if let Some(cached) = self.resolved.get(&entity) {
            return *cached;
        }
        let mut visiting = HashSet::new();
        let position = self
            .matrix(world, entity, &mut visiting, true)
            .map(|matrix| matrix.transform_point3(Vec3::ZERO));
        self.resolved.insert(entity, position);
        position
    }

    /// World matrix of `entity`. Returns `None` only when the queried entity
    /// itself has no Transform; missing ancestors and cycles degrade to
    /// identity roots so a position is still produced.
    fn matrix(
        &mut self,
        world: &World,
        entity: Entity,
        visiting: &mut HashSet<Entity>,
        root_query: bool,
    ) -> Option<glam::Mat4> {
        use engine_scene::components::Transform;

        let Some(transform) = world.get::<Transform>(entity) else {
            return if root_query {
                None
            } else {
                Some(glam::Mat4::IDENTITY)
            };
        };
        let local = glam::Mat4::from_scale_rotation_translation(
            transform.scale,
            transform.rotation,
            transform.translation,
        );
        let Some(parent) = transform.parent else {
            return Some(local);
        };
        if !visiting.insert(entity) {
            return None;
        }
        let parent_matrix = self.matrix(world, parent, visiting, false);
        visiting.remove(&entity);
        Some(parent_matrix.unwrap_or(glam::Mat4::IDENTITY) * local)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cooked_assets::tests::cook_test_material;
    use engine_asset::partition::{PartitionCell, WORLD_PARTITION_SCHEMA};
    use engine_asset::project::ProjectManifest;
    use engine_scene::components::Transform;
    use engine_scene::{sample_scene, ComponentRecord, EntityRecord};
    use engine_serialize::{SchemaVersion, Value};

    // ── Fixture helpers ─────────────────────────────────────────────────────

    struct StreamFixture {
        project: GameProject,
        partition: WorldPartition,
        scenes: BTreeMap<String, Scene>,
    }

    fn bounds(center: [f32; 3], half_extents: [f32; 3]) -> CellBounds {
        CellBounds {
            center,
            half_extents,
        }
    }

    fn origin_bounds() -> CellBounds {
        bounds([0.0, 0.0, 0.0], [10.0, 10.0, 10.0])
    }

    /// Write a project to a unique temp directory: every scene lands at
    /// `assets/scenes/<scene-id>.scene.ron`, the manifest catalogs them all,
    /// and `main` stays the startup scene. The partition is built in memory.
    fn stream_fixture(
        name: &str,
        scenes: Vec<Scene>,
        cells: Vec<(&str, &str, CellBounds)>,
    ) -> StreamFixture {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "engine-cell-stream-{name}-{}-{unique}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create fixture root");

        let mut manifest = ProjectManifest::new("Cell Stream Test");
        manifest.input_actions = None;
        manifest.scenes = scenes
            .iter()
            .map(|scene| {
                (
                    scene.scene_id.clone(),
                    PathBuf::from(format!("assets/scenes/{}.scene.ron", scene.scene_id)),
                )
            })
            .collect();
        assert!(
            manifest.scenes.contains_key("main"),
            "fixtures must include a \"main\" startup scene"
        );
        let manifest_path = manifest.write_to_root(&root).expect("write manifest");

        let mut scene_map = BTreeMap::new();
        for scene in scenes {
            let path = root.join(format!("assets/scenes/{}.scene.ron", scene.scene_id));
            scene.save_to_file(&path).expect("write cell scene");
            scene_map.insert(scene.scene_id.clone(), scene);
        }

        let project = GameProject {
            startup_scene: root.join(&manifest.startup_scene),
            asset_source: root.join(&manifest.asset_source),
            cooked_assets: root.join(&manifest.cooked_assets),
            manifest_path,
            root: root.clone(),
            manifest,
            script_project: None,
            script_assembly: None,
            input_actions: None,
        };
        let partition = WorldPartition {
            schema: WORLD_PARTITION_SCHEMA.to_string(),
            cells: cells
                .into_iter()
                .map(|(cell_id, scene_id, cell_bounds)| {
                    (
                        cell_id.to_string(),
                        PartitionCell {
                            scene: scene_id.to_string(),
                            bounds: cell_bounds,
                        },
                    )
                })
                .collect(),
        };
        StreamFixture {
            project,
            partition,
            scenes: scene_map,
        }
    }

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

    fn entity_record(
        id: &str,
        parent: Option<&str>,
        components: BTreeMap<String, ComponentRecord>,
    ) -> EntityRecord {
        EntityRecord {
            persistent_id: id.to_string(),
            parent: parent.map(str::to_string),
            name: Some(id.to_string()),
            enabled: true,
            components,
        }
    }

    fn cube_record(
        id: &str,
        parent: Option<&str>,
        translation: [f32; 3],
        material: &str,
    ) -> EntityRecord {
        entity_record(
            id,
            parent,
            BTreeMap::from([
                (
                    "engine.transform".to_string(),
                    transform_component(translation),
                ),
                (
                    "engine.renderable".to_string(),
                    renderable_component("mesh-cube", material),
                ),
            ]),
        )
    }

    /// Startup scene: a movable camera at the origin plus one static cube.
    fn startup_scene() -> Scene {
        let mut scene = sample_scene();
        scene.scene_id = "main".to_string();
        scene.entities = vec![
            entity_record(
                "camera-main",
                None,
                BTreeMap::from([
                    ("engine.camera".to_string(), component(BTreeMap::new())),
                    (
                        "engine.transform".to_string(),
                        transform_component([0.0, 0.0, 0.0]),
                    ),
                ]),
            ),
            cube_record("cube-01", None, [0.0, 0.0, 0.0], "mat-default"),
        ];
        scene.scene_settings.active_camera = Some("camera-main".to_string());
        scene.dependencies = vec![];
        scene
    }

    /// Cell scenes carry no camera of their own; the startup camera stays the
    /// active one and streamed cameras would only become overlay cameras.
    fn cell_scene(scene_id: &str, entities: Vec<EntityRecord>) -> Scene {
        let mut scene = sample_scene();
        scene.scene_id = scene_id.to_string();
        scene.entities = entities;
        scene.scene_settings.active_camera = None;
        scene.dependencies = vec![];
        scene
    }

    fn two_cell_fixture(name: &str) -> StreamFixture {
        stream_fixture(
            name,
            vec![
                startup_scene(),
                cell_scene(
                    "level-a",
                    vec![cube_record("cube-a", None, [0.0, 0.0, 0.0], "mat-default")],
                ),
                cell_scene(
                    "level-b",
                    vec![cube_record("cube-b", None, [5.0, 0.0, 0.0], "mat-default")],
                ),
            ],
            vec![
                ("cell-a", "level-a", origin_bounds()),
                (
                    "cell-b",
                    "level-b",
                    bounds([5.0, 0.0, 0.0], [10.0, 10.0, 10.0]),
                ),
            ],
        )
    }

    fn running_driver(
        fixture: &StreamFixture,
        config: CellStreamingConfig,
    ) -> (EngineRuntime, CellStreamingDriver) {
        let mut runtime = EngineRuntime::new(crate::EngineConfig::default());
        runtime
            .load_scene(fixture.scenes["main"].clone())
            .expect("startup scene loads");
        let mut driver =
            match CellStreamingDriver::new(&fixture.partition, &fixture.project, config) {
                Ok(driver) => driver,
                Err(error) => panic!("driver construction failed: {error}"),
            };
        driver.rebaseline(&runtime);
        (runtime, driver)
    }

    fn set_camera_position(runtime: &EngineRuntime, position: Vec3) {
        runtime.with_world_mut(|world| {
            let camera = world
                .entity_by_persistent_id("camera-main")
                .expect("camera entity");
            world
                .get_mut::<Transform>(camera)
                .expect("camera transform")
                .translation = position;
        });
    }

    fn has_entity(runtime: &EngineRuntime, id: &str) -> bool {
        runtime
            .with_world(|world| world.entity_by_persistent_id(id).is_some())
            .unwrap_or(false)
    }

    /// Pure in-memory partition for validation tests.
    fn partition_of(cells: &[(&str, &str)]) -> WorldPartition {
        WorldPartition {
            schema: WORLD_PARTITION_SCHEMA.to_string(),
            cells: cells
                .iter()
                .map(|(cell_id, scene_id)| {
                    (
                        cell_id.to_string(),
                        PartitionCell {
                            scene: scene_id.to_string(),
                            bounds: origin_bounds(),
                        },
                    )
                })
                .collect(),
        }
    }

    // ── Hysteresis and budgets ──────────────────────────────────────────────

    #[test]
    fn cell_streams_in_and_out_with_hysteresis() {
        let fixture = stream_fixture(
            "hysteresis",
            vec![
                startup_scene(),
                cell_scene(
                    "level-a",
                    vec![cube_record("cube-a", None, [2.0, 0.0, 0.0], "mat-default")],
                ),
            ],
            vec![("cell-a", "level-a", origin_bounds())],
        );
        let (mut runtime, mut driver) = running_driver(&fixture, CellStreamingConfig::default());

        // Camera at the origin: a zero-asset cell merges in a single tick.
        let report = driver.tick(&mut runtime);
        assert_eq!(report.merged_cells, vec!["cell-a".to_string()]);
        assert!(report.world_changed());
        assert_eq!(driver.cell_state("cell-a"), Some(&CellState::Loaded));
        assert!(has_entity(&runtime, "cube-a"));

        // Hysteresis band: outside enter (10.0) but inside exit (11.5).
        set_camera_position(&runtime, Vec3::new(11.0, 0.0, 0.0));
        let report = driver.tick(&mut runtime);
        assert!(!report.world_changed());
        assert_eq!(driver.cell_state("cell-a"), Some(&CellState::Loaded));
        assert!(has_entity(&runtime, "cube-a"));

        // Outside exit: the cell unloads.
        set_camera_position(&runtime, Vec3::new(20.0, 0.0, 0.0));
        let report = driver.tick(&mut runtime);
        assert_eq!(report.unloaded_cells, vec!["cell-a".to_string()]);
        assert_eq!(driver.cell_state("cell-a"), Some(&CellState::Unloaded));
        assert!(!has_entity(&runtime, "cube-a"));

        // Re-entering streams the cell back in.
        set_camera_position(&runtime, Vec3::ZERO);
        let report = driver.tick(&mut runtime);
        assert_eq!(report.merged_cells, vec!["cell-a".to_string()]);
        assert!(has_entity(&runtime, "cube-a"));
        assert_eq!(driver.total_merges(), 2);
        assert_eq!(driver.total_unloads(), 1);
    }

    #[test]
    fn custom_hysteresis_factors_are_honored() {
        let fixture = stream_fixture(
            "custom-factors",
            vec![
                startup_scene(),
                cell_scene(
                    "level-a",
                    vec![cube_record("cube-a", None, [0.0, 0.0, 0.0], "mat-default")],
                ),
            ],
            vec![("cell-a", "level-a", origin_bounds())],
        );
        let config = CellStreamingConfig {
            enter_factor: 0.5,
            exit_factor: 0.6,
            ..CellStreamingConfig::default()
        };
        let (mut runtime, mut driver) = running_driver(&fixture, config);

        // Enter band is |x| <= 5.0: the camera at 7.0 stays out.
        set_camera_position(&runtime, Vec3::new(7.0, 0.0, 0.0));
        let report = driver.tick(&mut runtime);
        assert!(!report.world_changed());
        assert_eq!(driver.cell_state("cell-a"), Some(&CellState::Unloaded));

        // Inside enter: merges.
        set_camera_position(&runtime, Vec3::new(4.0, 0.0, 0.0));
        let report = driver.tick(&mut runtime);
        assert_eq!(report.merged_cells, vec!["cell-a".to_string()]);

        // Outside enter but inside exit (6.0): stays loaded.
        set_camera_position(&runtime, Vec3::new(5.5, 0.0, 0.0));
        let report = driver.tick(&mut runtime);
        assert!(!report.world_changed());
        assert_eq!(driver.cell_state("cell-a"), Some(&CellState::Loaded));

        // Outside exit: unloads.
        set_camera_position(&runtime, Vec3::new(7.0, 0.0, 0.0));
        let report = driver.tick(&mut runtime);
        assert_eq!(report.unloaded_cells, vec!["cell-a".to_string()]);
        assert!(!has_entity(&runtime, "cube-a"));
    }

    #[test]
    fn invalid_hysteresis_config_is_rejected() {
        let fixture = stream_fixture("invalid-config", vec![startup_scene()], vec![]);
        for config in [
            CellStreamingConfig {
                enter_factor: 0.0,
                ..CellStreamingConfig::default()
            },
            CellStreamingConfig {
                enter_factor: f32::NAN,
                ..CellStreamingConfig::default()
            },
            CellStreamingConfig {
                enter_factor: 2.0,
                exit_factor: 1.5,
                ..CellStreamingConfig::default()
            },
        ] {
            let result = CellStreamingDriver::new(&fixture.partition, &fixture.project, config);
            assert!(
                matches!(result, Err(CellStreamError::InvalidConfig(_))),
                "config {config:?} must be rejected"
            );
        }

        // Zero budgets are clamped to one, not rejected.
        let clamped = CellStreamingConfig {
            max_merges_per_commit: 0,
            max_unloads_per_commit: 0,
            ..CellStreamingConfig::default()
        };
        assert!(
            CellStreamingDriver::new(&fixture.partition, &fixture.project, clamped)
                .err()
                .is_none()
        );
    }

    #[test]
    fn merge_budget_commits_one_cell_per_tick() {
        let fixture = two_cell_fixture("merge-budget");
        let (mut runtime, mut driver) = running_driver(&fixture, CellStreamingConfig::default());

        // Both cells contain the camera at the origin; the budget is one.
        let report = driver.tick(&mut runtime);
        assert_eq!(report.merged_cells, vec!["cell-a".to_string()]);
        assert_eq!(driver.cell_state("cell-a"), Some(&CellState::Loaded));
        assert_eq!(driver.cell_state("cell-b"), Some(&CellState::Merging));
        assert!(has_entity(&runtime, "cube-a"));
        assert!(!has_entity(&runtime, "cube-b"));

        let report = driver.tick(&mut runtime);
        assert_eq!(report.merged_cells, vec!["cell-b".to_string()]);
        assert_eq!(driver.cell_state("cell-b"), Some(&CellState::Loaded));
        assert!(has_entity(&runtime, "cube-b"));
    }

    #[test]
    fn unload_budget_and_camera_return_cancels_pending_unload() {
        let fixture = two_cell_fixture("unload-budget");
        let config = CellStreamingConfig {
            max_merges_per_commit: 2,
            max_unloads_per_commit: 1,
            ..CellStreamingConfig::default()
        };
        let (mut runtime, mut driver) = running_driver(&fixture, config);

        let report = driver.tick(&mut runtime);
        assert_eq!(
            report.merged_cells,
            vec!["cell-a".to_string(), "cell-b".to_string()]
        );

        // Both cells fall out of range; only one unload commits per tick.
        set_camera_position(&runtime, Vec3::new(100.0, 0.0, 0.0));
        let report = driver.tick(&mut runtime);
        assert_eq!(report.unloaded_cells, vec!["cell-a".to_string()]);
        assert_eq!(driver.cell_state("cell-a"), Some(&CellState::Unloaded));
        assert_eq!(driver.cell_state("cell-b"), Some(&CellState::Unloading));
        assert!(!has_entity(&runtime, "cube-a"));
        assert!(has_entity(&runtime, "cube-b"));

        // The camera returns before cell-b's unload commits: the unload is
        // cancelled and cell-b was never destroyed; cell-a streams back in.
        set_camera_position(&runtime, Vec3::ZERO);
        let report = driver.tick(&mut runtime);
        assert!(report.unloaded_cells.is_empty());
        assert_eq!(report.merged_cells, vec!["cell-a".to_string()]);
        assert_eq!(driver.cell_state("cell-b"), Some(&CellState::Loaded));
        assert!(has_entity(&runtime, "cube-a"));
        assert!(has_entity(&runtime, "cube-b"));
        assert_eq!(driver.total_unloads(), 1);
    }

    // ── Asset streaming ─────────────────────────────────────────────────────

    #[test]
    fn merge_waits_for_background_asset_stream() {
        let fixture = stream_fixture(
            "asset-stream",
            vec![
                startup_scene(),
                cell_scene(
                    "level-a",
                    vec![
                        cube_record("cube-a", None, [0.0, 0.0, 0.0], "mat.cell.a"),
                        cube_record("cube-b", None, [3.0, 0.0, 0.0], "mat.cell.b"),
                    ],
                ),
            ],
            vec![("cell-a", "level-a", origin_bounds())],
        );
        std::fs::create_dir_all(&fixture.project.cooked_assets).expect("cooked dir");
        cook_test_material(&fixture.project.cooked_assets, "mat.cell.a", None);
        cook_test_material(&fixture.project.cooked_assets, "mat.cell.b", None);

        let (mut runtime, mut driver) = running_driver(&fixture, CellStreamingConfig::default());
        runtime.set_cooked_asset_stream_budget(1);

        // First tick: assets are enqueued but cannot be committed yet.
        let report = driver.tick(&mut runtime);
        assert_eq!(report.enqueued_assets, 2);
        assert_eq!(driver.cell_state("cell-a"), Some(&CellState::LoadingAssets));
        assert!(!has_entity(&runtime, "cube-a"));

        // Pump ticks until the background stream commits both materials and
        // the merge lands (budget 1 per drain, so this takes a few ticks).
        // The sleep mirrors drain_until_idle: without it the spin loop can
        // exhaust all iterations before the decoder thread is scheduled.
        let mut merged = false;
        for _ in 0..200 {
            let report = driver.tick(&mut runtime);
            if report.merged_cells == vec!["cell-a".to_string()] {
                merged = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(merged, "cell merged once its assets streamed in");
        assert_eq!(driver.cell_state("cell-a"), Some(&CellState::Loaded));
        assert!(has_entity(&runtime, "cube-a"));
        assert!(has_entity(&runtime, "cube-b"));
        assert!(runtime
            .asset_registry()
            .contains(&AssetId::new("mat.cell.a")));
        assert!(runtime
            .asset_registry()
            .contains(&AssetId::new("mat.cell.b")));
    }

    #[test]
    fn missing_cooked_asset_fails_the_cell_without_retry() {
        let fixture = stream_fixture(
            "missing-asset",
            vec![
                startup_scene(),
                cell_scene(
                    "level-a",
                    vec![cube_record("cube-a", None, [0.0, 0.0, 0.0], "mat.missing")],
                ),
            ],
            vec![("cell-a", "level-a", origin_bounds())],
        );
        let (mut runtime, mut driver) = running_driver(&fixture, CellStreamingConfig::default());

        let report = driver.tick(&mut runtime);
        assert_eq!(report.failed_cells, vec!["cell-a".to_string()]);
        assert!(matches!(
            driver.cell_state("cell-a"),
            Some(CellState::Failed(_))
        ));
        assert!(!has_entity(&runtime, "cube-a"));
        assert!(runtime
            .diagnostics_collector()
            .all()
            .iter()
            .any(|diagnostic| diagnostic.code == "CELL_STREAM"));

        // A failed cell never retries on its own.
        let report = driver.tick(&mut runtime);
        assert!(report.failed_cells.is_empty());
        assert!(report.merged_cells.is_empty());
        assert_eq!(report.enqueued_assets, 0);
        assert!(matches!(
            driver.cell_state("cell-a"),
            Some(CellState::Failed(_))
        ));
    }

    // ── Residency ───────────────────────────────────────────────────────────

    #[test]
    fn runtime_created_entity_becomes_resident_and_survives_unload() {
        let fixture = stream_fixture(
            "resident-runtime",
            vec![
                startup_scene(),
                cell_scene(
                    "level-a",
                    vec![cube_record("cube-a", None, [0.0, 0.0, 0.0], "mat-default")],
                ),
            ],
            vec![("cell-a", "level-a", origin_bounds())],
        );
        let (mut runtime, mut driver) = running_driver(&fixture, CellStreamingConfig::default());
        driver.tick(&mut runtime);
        assert!(has_entity(&runtime, "cube-a"));

        // A script-style runtime creation: a persistent entity the driver
        // never merged.
        runtime.with_world_mut(|world| {
            world
                .create_persistent_entity("runtime-probe")
                .expect("create runtime entity");
        });

        set_camera_position(&runtime, Vec3::new(100.0, 0.0, 0.0));
        let report = driver.tick(&mut runtime);
        assert!(report
            .resident_ids_added
            .contains(&"runtime-probe".to_string()));
        assert_eq!(report.unloaded_cells, vec!["cell-a".to_string()]);
        assert!(!has_entity(&runtime, "cube-a"));
        assert!(has_entity(&runtime, "runtime-probe"));
        assert!(driver.resident_ids().contains("runtime-probe"));
    }

    #[test]
    fn entity_moved_out_of_cell_becomes_resident_detached_and_is_not_remerged() {
        let fixture = stream_fixture(
            "resident-moved",
            vec![
                startup_scene(),
                cell_scene(
                    "level-a",
                    vec![
                        cube_record("cell-parent", None, [0.0, 0.0, 0.0], "mat-default"),
                        cube_record(
                            "cell-child",
                            Some("cell-parent"),
                            [4.0, 0.0, 0.0],
                            "mat-default",
                        ),
                    ],
                ),
            ],
            vec![("cell-a", "level-a", origin_bounds())],
        );
        let (mut runtime, mut driver) = running_driver(&fixture, CellStreamingConfig::default());
        driver.tick(&mut runtime);
        assert!(has_entity(&runtime, "cell-parent"));
        assert!(has_entity(&runtime, "cell-child"));

        // Move the child far outside the cell's exit bounds (its world
        // position is parent(0) + local(500)).
        runtime.with_world_mut(|world| {
            let child = world
                .entity_by_persistent_id("cell-child")
                .expect("child entity");
            world
                .get_mut::<Transform>(child)
                .expect("child transform")
                .translation = Vec3::new(500.0, 0.0, 0.0);
        });

        set_camera_position(&runtime, Vec3::new(100.0, 0.0, 0.0));
        let report = driver.tick(&mut runtime);
        assert!(report
            .resident_ids_added
            .contains(&"cell-child".to_string()));
        assert_eq!(report.unloaded_cells, vec!["cell-a".to_string()]);
        assert!(!has_entity(&runtime, "cell-parent"));
        assert!(has_entity(&runtime, "cell-child"));
        runtime.with_world(|world| {
            let child = world
                .entity_by_persistent_id("cell-child")
                .expect("child survives");
            let transform = world.get::<Transform>(child).expect("child transform");
            assert!(transform.parent.is_none(), "resident child detached");
            assert_eq!(transform.translation, Vec3::new(500.0, 0.0, 0.0));
            assert!(world.parent_persistent_id(child).is_none());
        });

        // Re-entering re-merges the cell without duplicating the resident
        // child: the parent returns, the child keeps its runtime state.
        set_camera_position(&runtime, Vec3::ZERO);
        let report = driver.tick(&mut runtime);
        assert_eq!(report.merged_cells, vec!["cell-a".to_string()]);
        assert_eq!(driver.cell_state("cell-a"), Some(&CellState::Loaded));
        assert!(has_entity(&runtime, "cell-parent"));
        assert!(has_entity(&runtime, "cell-child"));
        runtime.with_world(|world| {
            let child = world
                .entity_by_persistent_id("cell-child")
                .expect("child still present");
            assert_eq!(
                world
                    .get::<Transform>(child)
                    .expect("child transform")
                    .translation,
                Vec3::new(500.0, 0.0, 0.0)
            );
        });
    }

    // ── Rebaseline ──────────────────────────────────────────────────────────

    #[test]
    fn rebaseline_adopts_live_cells_and_resets_after_scene_replacement() {
        let fixture = stream_fixture(
            "rebaseline",
            vec![
                startup_scene(),
                cell_scene(
                    "level-a",
                    vec![cube_record("cube-a", None, [0.0, 0.0, 0.0], "mat-default")],
                ),
            ],
            vec![
                (
                    "cell-main",
                    "main",
                    bounds([1000.0, 0.0, 0.0], [10.0, 10.0, 10.0]),
                ),
                ("cell-a", "level-a", origin_bounds()),
            ],
        );
        let (mut runtime, mut driver) = running_driver(&fixture, CellStreamingConfig::default());

        // cell-main references the startup scene, so every one of its
        // entities is already live: it is adopted as Loaded regardless of
        // where its bounds sit.
        assert_eq!(driver.cell_state("cell-main"), Some(&CellState::Loaded));
        assert_eq!(driver.cell_state("cell-a"), Some(&CellState::Unloaded));
        assert_eq!(driver.loaded_cells(), vec!["cell-main".to_string()]);
        assert!(driver.base_ids().is_empty());

        // A runtime-created entity joins the resident set...
        runtime.with_world_mut(|world| {
            world
                .create_persistent_entity("runtime-probe")
                .expect("create runtime entity");
        });
        driver.tick(&mut runtime);
        assert!(driver.resident_ids().contains("runtime-probe"));

        // ...and a scene replacement wipes the baseline: the old cell content
        // is gone with the world, the resident set clears, and the cell whose
        // scene was just loaded wholesale is adopted instead.
        runtime
            .load_scene(fixture.scenes["level-a"].clone())
            .expect("replacement scene loads");
        driver.rebaseline(&runtime);
        assert_eq!(driver.cell_state("cell-main"), Some(&CellState::Unloaded));
        assert_eq!(driver.cell_state("cell-a"), Some(&CellState::Loaded));
        assert_eq!(driver.loaded_cells(), vec!["cell-a".to_string()]);
        assert!(driver.resident_ids().is_empty());
    }

    // ── Cameraless worlds ───────────────────────────────────────────────────

    #[test]
    fn tick_without_active_camera_is_a_noop() {
        let mut no_camera = sample_scene();
        no_camera.scene_id = "main".to_string();
        no_camera.entities = vec![cube_record("cube-01", None, [0.0, 0.0, 0.0], "mat-default")];
        no_camera.scene_settings.active_camera = None;
        no_camera.dependencies = vec![];
        let fixture = stream_fixture(
            "no-camera",
            vec![
                no_camera,
                cell_scene(
                    "level-a",
                    vec![cube_record("cube-a", None, [0.0, 0.0, 0.0], "mat-default")],
                ),
            ],
            vec![("cell-a", "level-a", origin_bounds())],
        );
        let (mut runtime, mut driver) = running_driver(&fixture, CellStreamingConfig::default());

        let report = driver.tick(&mut runtime);
        assert_eq!(report.camera, None);
        assert!(report.merged_cells.is_empty());
        assert!(!report.world_changed());
        assert_eq!(driver.cell_state("cell-a"), Some(&CellState::Unloaded));
        assert!(!has_entity(&runtime, "cube-a"));
    }

    // ── Validation ──────────────────────────────────────────────────────────

    #[test]
    fn validation_rejects_duplicate_persistent_ids_across_cells() {
        let startup = startup_scene();
        let level_a = cell_scene(
            "level-a",
            vec![cube_record("shared-cube", None, [0.0; 3], "mat-default")],
        );
        let level_b = cell_scene(
            "level-b",
            vec![cube_record("shared-cube", None, [0.0; 3], "mat-default")],
        );
        let scenes = BTreeMap::from([
            ("main".to_string(), &startup),
            ("level-a".to_string(), &level_a),
            ("level-b".to_string(), &level_b),
        ]);
        let partition = partition_of(&[("cell-a", "level-a"), ("cell-b", "level-b")]);
        let error = validate_partition_cell_scenes(&partition, "main", &scenes)
            .expect_err("duplicate ids across cells must fail");
        assert!(matches!(
            error,
            CellStreamError::DuplicatePersistentIdAcrossCells { .. }
        ));
    }

    #[test]
    fn validation_rejects_script_components_in_cells() {
        let startup = startup_scene();
        let scripted = entity_record(
            "scripted",
            None,
            BTreeMap::from([(
                "engine.script".to_string(),
                component(BTreeMap::from([(
                    "script".to_string(),
                    Value::Str("Game.Player".to_string()),
                )])),
            )]),
        );
        let level_a = cell_scene("level-a", vec![scripted]);
        let scenes = BTreeMap::from([
            ("main".to_string(), &startup),
            ("level-a".to_string(), &level_a),
        ]);
        let partition = partition_of(&[("cell-a", "level-a")]);
        let error = validate_partition_cell_scenes(&partition, "main", &scenes)
            .expect_err("engine.script in a cell must fail");
        assert_eq!(
            error,
            CellStreamError::ScriptComponentInCell {
                cell_id: "cell-a".to_string(),
                entity_id: "scripted".to_string(),
            }
        );
    }

    #[test]
    fn validation_rejects_cell_ids_overlapping_the_startup_scene() {
        let startup = startup_scene();
        let level_a = cell_scene(
            "level-a",
            vec![cube_record("cube-01", None, [0.0; 3], "mat-default")],
        );
        let scenes = BTreeMap::from([
            ("main".to_string(), &startup),
            ("level-a".to_string(), &level_a),
        ]);
        let partition = partition_of(&[("cell-a", "level-a")]);
        let error = validate_partition_cell_scenes(&partition, "main", &scenes)
            .expect_err("startup id overlap must fail");
        assert_eq!(
            error,
            CellStreamError::StartupSceneIdConflict {
                cell_id: "cell-a".to_string(),
                persistent_id: "cube-01".to_string(),
                startup_scene_id: "main".to_string(),
            }
        );

        // A cell that references the startup scene itself may share its ids:
        // the driver adopts the already-live entities at rebaseline.
        let scenes = BTreeMap::from([("main".to_string(), &startup)]);
        let partition = partition_of(&[("cell-main", "main")]);
        validate_partition_cell_scenes(&partition, "main", &scenes)
            .expect("startup-referencing cell is valid");
    }

    #[test]
    fn validation_rejects_unknown_cell_scenes() {
        let startup = startup_scene();
        let scenes = BTreeMap::from([("main".to_string(), &startup)]);
        let partition = partition_of(&[("cell-ghost", "ghost")]);
        let error = validate_partition_cell_scenes(&partition, "main", &scenes)
            .expect_err("unknown cell scene must fail");
        assert_eq!(
            error,
            CellStreamError::UnknownCellScene {
                cell_id: "cell-ghost".to_string(),
                scene_id: "ghost".to_string(),
            }
        );
    }

    #[test]
    fn driver_new_rejects_cells_referencing_unknown_scenes() {
        let fixture = stream_fixture("unknown-cell-scene", vec![startup_scene()], vec![]);
        let partition = partition_of(&[("cell-ghost", "ghost")]);
        let error =
            CellStreamingDriver::new(&partition, &fixture.project, CellStreamingConfig::default())
                .err()
                .expect("driver construction must fail");
        assert_eq!(
            error,
            CellStreamError::UnknownCellScene {
                cell_id: "cell-ghost".to_string(),
                scene_id: "ghost".to_string(),
            }
        );
    }

    #[test]
    fn driver_new_reports_cell_scene_load_failures() {
        let mut fixture = stream_fixture("missing-scene-file", vec![startup_scene()], vec![]);
        // Catalog entry without a file on disk.
        fixture.project.manifest.scenes.insert(
            "ghost".to_string(),
            PathBuf::from("assets/scenes/ghost.scene.ron"),
        );
        let partition = partition_of(&[("cell-ghost", "ghost")]);
        let error =
            CellStreamingDriver::new(&partition, &fixture.project, CellStreamingConfig::default())
                .err()
                .expect("driver construction must fail");
        assert!(matches!(error, CellStreamError::CellSceneLoad { .. }));
    }

    // ── Physics (gameplay feature) ──────────────────────────────────────────

    #[cfg(feature = "gameplay")]
    #[test]
    fn physics_bodies_follow_cell_merge_and_unload() {
        let rigid_body = component(BTreeMap::from([
            ("body_type".to_string(), Value::Enum("Dynamic".to_string())),
            ("mass".to_string(), Value::Float32(1.0)),
        ]));
        let ball = entity_record(
            "ball",
            None,
            BTreeMap::from([
                (
                    "engine.transform".to_string(),
                    transform_component([0.0, 5.0, 0.0]),
                ),
                (
                    "engine.renderable".to_string(),
                    renderable_component("mesh-cube", "mat-default"),
                ),
                ("engine.physics.rigid_body".to_string(), rigid_body),
            ]),
        );
        let fixture = stream_fixture(
            "physics",
            vec![startup_scene(), cell_scene("level-a", vec![ball])],
            vec![("cell-a", "level-a", origin_bounds())],
        );

        let mut game_loop = crate::game_loop::GameLoop::new(crate::EngineConfig::default());
        game_loop
            .load_scene(fixture.scenes["main"].clone())
            .expect("startup scene loads");
        assert_eq!(
            game_loop
                .physics
                .as_ref()
                .expect("physics initialised")
                .body_count(),
            0
        );

        let mut driver = match CellStreamingDriver::new(
            &fixture.partition,
            &fixture.project,
            CellStreamingConfig::default(),
        ) {
            Ok(driver) => driver,
            Err(error) => panic!("driver construction failed: {error}"),
        };
        driver.rebaseline(&game_loop.runtime);

        // Merge: the rigid body gains a physics body after the resync.
        let report = driver.tick(&mut game_loop.runtime);
        assert!(report.world_changed());
        game_loop.resync_physics_from_world();
        let ball_entity = game_loop
            .runtime
            .with_world(|world| world.entity_by_persistent_id("ball"))
            .flatten()
            .expect("ball merged");
        assert!(game_loop
            .physics
            .as_ref()
            .expect("physics")
            .has_body(ball_entity));

        // Unload: the incremental sync removes the body again.
        set_camera_position(&game_loop.runtime, Vec3::new(100.0, 0.0, 0.0));
        let report = driver.tick(&mut game_loop.runtime);
        assert!(report.world_changed());
        game_loop.resync_physics_from_world();
        assert_eq!(game_loop.physics.as_ref().expect("physics").body_count(), 0);
    }

    // ── World-origin shifts (ENG-01 Phase 2) ───────────────────────────────

    /// Fixture with a far-field cell: authored bounds x ∈ [8000, 8020] and a
    /// cube authored at logical x = 8005.
    fn far_cell_fixture(name: &str) -> StreamFixture {
        stream_fixture(
            name,
            vec![
                startup_scene(),
                cell_scene(
                    "level-a",
                    vec![cube_record(
                        "cube-a",
                        None,
                        [8005.0, 0.0, 0.0],
                        "mat-default",
                    )],
                ),
            ],
            vec![(
                "cell-a",
                "level-a",
                bounds([8010.0, 0.0, 0.0], [10.0, 10.0, 10.0]),
            )],
        )
    }

    /// Move the camera to logical x = 8010, then shift the world origin by
    /// 8 km: the camera lands at relative x = 10 with its logical position
    /// unchanged.
    fn shift_origin_to_camera(runtime: &EngineRuntime) {
        set_camera_position(runtime, Vec3::new(8010.0, 0.0, 0.0));
        runtime.with_world_mut(|world| {
            world.shift_world_origin([8000.0, 0.0, 0.0]);
        });
    }

    #[test]
    fn streaming_decisions_use_logical_positions_with_a_non_zero_origin() {
        let fixture = far_cell_fixture("origin-logical");
        let (mut runtime, mut driver) = running_driver(&fixture, CellStreamingConfig::default());
        shift_origin_to_camera(&runtime);

        // Logical camera position 8010 is inside the cell bounds even though
        // the stored (relative) camera position is only 10.
        let report = driver.tick(&mut runtime);
        assert_eq!(report.merged_cells, vec!["cell-a".to_string()]);
        assert_eq!(driver.cell_state("cell-a"), Some(&CellState::Loaded));
        assert!(has_entity(&runtime, "cube-a"));

        // Moving the relative camera to 100 lifts the logical position to
        // 8100 — outside the exit band — so the cell unloads.
        set_camera_position(&runtime, Vec3::new(100.0, 0.0, 0.0));
        let report = driver.tick(&mut runtime);
        assert_eq!(report.unloaded_cells, vec!["cell-a".to_string()]);
        assert!(!has_entity(&runtime, "cube-a"));
    }

    #[test]
    fn merged_cell_roots_are_rebased_into_origin_relative_space() {
        let fixture = far_cell_fixture("origin-rebase");
        let (mut runtime, mut driver) = running_driver(&fixture, CellStreamingConfig::default());
        shift_origin_to_camera(&runtime);

        let report = driver.tick(&mut runtime);
        assert_eq!(report.merged_cells, vec!["cell-a".to_string()]);

        // The cube was authored at logical x = 8005; after the merge its
        // stored transform must be rebased by -origin, i.e. relative x ≈ 5,
        // so `world_origin + translation` still equals 8005.
        let translation = runtime
            .with_world(|world| {
                let cube = world.entity_by_persistent_id("cube-a")?;
                Some(world.get::<Transform>(cube)?.translation)
            })
            .flatten()
            .expect("merged cube transform");
        assert!(
            (translation.x - 5.0).abs() < 1e-3,
            "expected relative x ≈ 5, got {translation:?}"
        );
        let origin = runtime
            .with_world(|world| world.world_origin())
            .expect("world");
        assert_eq!(origin, [8000.0, 0.0, 0.0]);
    }
}
