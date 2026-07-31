use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use engine_asset::partition::{CellBounds, WorldPartition};
use engine_asset::project::GameProject;
use engine_scene::components::Transform;
use engine_scene::{
    active_camera_world_position, validate_scene, Scene, SCENE_ONLY_COMPONENT_TYPES,
};
use engine_serialize::{AssetId, Diagnostic, DiagnosticSeverity, PersistentId};
use glam::Vec3;

use crate::EngineRuntime;

use super::state::CellRecord;
use super::world_positions::{cell_root_ids, WorldPositions};
use super::{
    origin_offset, validate_partition_cell_scenes, CellState, CellStreamError,
    CellStreamTickReport, CellStreamingConfig,
};

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
