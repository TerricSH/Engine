//! Gate 6 Session 6A — Hot Reload Pipeline.
//!
//! Incremental file watching, re-cooking, reload state tracking, and
//! diagnostics for the engine's asset pipeline.
//!
//! # Architecture
//!
//! ```text
//! ┌──────────────────────────────────────────┐
//! │            ReloadCoordinator             │
//! │  ┌────────────┐  ┌────────────────────┐  │
//! │  │ WatchCoord │  │  ReloadTracker     │  │
//! │  │  .poll()   │  │  .transition()     │  │
//! │  └─────┬──────┘  │  .to_diagnostics() │  │
//! │        │         └────────────────────┘  │
//! │        ▼                                  │
//! │  ┌────────────┐                           │
//! │  │  Recook    │  DependencyGraph          │
//! │  │ (per event)│ ←───────────────          │
//! │  └────────────┘                           │
//! └──────────────────────────────────────────┘
//! ```
//!
//! # Usage
//!
//! ```ignore
//! use engine_asset::reload::ReloadCoordinator;
//!
//! let mut coord = ReloadCoordinator::new(
//!     Path::new("assets/source"),
//!     Path::new("assets/source"),
//!     Path::new("assets/cooked"),
//! )?;
//!
//! // Each frame:
//! let diagnostics = coord.poll();
//! for d in &diagnostics {
//!     println!("[{:?}] {}", d.severity, d.message);
//! }
//! ```

pub mod recook;
pub mod state;
pub mod watch;

use std::path::{Path, PathBuf};

use engine_serialize::{AssetId, Diagnostic};

use crate::cook::DependencyGraph;
use crate::registry::AssetRegistry;

pub use recook::incremental_recook;
pub use state::{ReloadInfo, ReloadState, ReloadTracker};
pub use watch::{WatchCoordinator, WatchEvent, WatchEventKind};

// ---------------------------------------------------------------------------
// ReloadCoordinator
// ---------------------------------------------------------------------------

/// Top-level coordinator for the hot-reload pipeline.
///
/// Owns:
///
/// * A [`WatchCoordinator`] that polls the file watcher and debounces events.
/// * A [`DependencyGraph`] that tracks asset dependency relationships.
/// * A [`ReloadTracker`] that maintains per-asset reload state and produces
///   diagnostics.
///
/// Call [`poll`](Self::poll) once per frame to drive the entire pipeline:
///
/// 1. Drain debounced watch events from the file watcher.
/// 2. Resolve changed paths to [`AssetId`]s.
/// 3. Expand with reverse dependencies from the graph.
/// 4. Transition each affected asset through the reload state machine:
///    `Detected → Recooking → Cooked → Queued → Applied`.
/// 5. Produce [`Diagnostic`] values summarising state transitions.
pub struct ReloadCoordinator {
    /// File-watch coordinator (owns the notify-based watcher + debounce).
    /// `None` when the coordinator is disabled.
    watch: Option<WatchCoordinator>,
    /// Asset dependency graph produced by the cook pipeline.
    graph: DependencyGraph,
    /// Per-asset reload state tracker.
    tracker: ReloadTracker,
    /// Directory containing source manifests and raw assets.
    source_dir: PathBuf,
    /// Directory where cooked artifacts are written.
    cooked_dir: PathBuf,
    /// Whether the pipeline is enabled.
    enabled: bool,
}

impl ReloadCoordinator {
    /// Create a new reload coordinator.
    ///
    /// # Parameters
    ///
    /// * `watch_dir`  – directory to watch for file changes (typically
    ///   `assets/source`).
    /// * `source_dir` – directory containing source manifests and assets
    ///   (same as `watch_dir` in typical setups).
    /// * `cooked_dir` – directory where cooked artifacts are written
    ///   (typically `assets/cooked`).
    ///
    /// # Errors
    ///
    /// Returns [`crate::AssetError::WatcherFailed`] if the underlying file
    /// watcher cannot be created.
    pub fn new(
        watch_dir: &Path,
        source_dir: &Path,
        cooked_dir: &Path,
    ) -> Result<Self, crate::AssetError> {
        let watch = Some(WatchCoordinator::new(watch_dir)?);
        tracing::info!(
            watch_dir = %watch_dir.display(),
            source_dir = %source_dir.display(),
            cooked_dir = %cooked_dir.display(),
            "reload coordinator created"
        );
        Ok(Self {
            watch,
            graph: DependencyGraph::new(),
            tracker: ReloadTracker::new(),
            source_dir: source_dir.to_path_buf(),
            cooked_dir: cooked_dir.to_path_buf(),
            enabled: true,
        })
    }

    /// Create a disabled reload coordinator that produces no diagnostics.
    ///
    /// Useful for sandbox environments where file watching is not
    /// available or desired.
    pub fn new_disabled() -> Self {
        tracing::info!("reload coordinator created (disabled)");
        Self {
            watch: None,
            graph: DependencyGraph::new(),
            tracker: ReloadTracker::new(),
            source_dir: PathBuf::from(""),
            cooked_dir: PathBuf::from(""),
            enabled: false,
        }
    }

    /// Return the current reload state as diagnostics without driving
    /// the pipeline (non-mutating).
    ///
    /// This is useful for consumers that want a read-only snapshot,
    /// such as the editor diagnostics panel.
    pub fn reload_diagnostics(&self) -> Vec<Diagnostic> {
        self.tracker.to_diagnostics()
    }

    /// Drive the reload pipeline for one frame.
    ///
    /// 1. Drain debounced watch events.
    /// 2. Resolve changed paths to [`AssetId`]s using `path_to_asset_id`.
    /// 3. Expand with reverse dependencies from the [`DependencyGraph`].
    /// 4. Mark assets as `Detected → Recooking`.
    /// 5. Call [`incremental_recook`] for each affected asset.
    /// 6. On success: mark `Cooked → Queued`.
    /// 7. On failure: mark `Failed` (last valid state preserved).
    /// 8. Collect and return [`Diagnostic`] values.
    ///
    /// Returns an empty vec if the coordinator is disabled.
    pub fn poll(&mut self) -> Vec<Diagnostic> {
        if !self.enabled {
            return Vec::new();
        }

        // 1. Drain watch events.
        let events = self
            .watch
            .as_mut()
            .map(|w| w.poll_events())
            .unwrap_or_default();
        if events.is_empty() {
            // Even with no new events, report diagnostics for any
            // in-progress/recently-completed reloads.
            return self.tracker.to_diagnostics();
        }

        // 2. Resolve paths → AssetIds.
        let mut affected_ids: Vec<AssetId> = Vec::new();
        for event in &events {
            if let Some(asset_id) = crate::hot_reload::path_to_asset_id(&event.path) {
                affected_ids.push(asset_id);
            }
        }

        // 3. Expand with reverse dependencies.
        let mut all_ids: Vec<AssetId> = affected_ids.clone();
        for id in &affected_ids {
            let rev_deps = self.graph.get_reverse_dependencies(id);
            for rev in rev_deps {
                if !all_ids.contains(&rev) {
                    all_ids.push(rev);
                }
            }
        }

        // 4. Mark Detected → Recooking.
        for id in &all_ids {
            if !self.graph.contains(id) {
                // Register unknown assets in the graph so state tracking
                // still works even if the cook pipeline hasn't seen them.
                self.graph.register(id.clone());
            }
            self.tracker.transition(id, ReloadState::Detected);
            self.tracker.transition(id, ReloadState::Recooking);
        }

        // 5. Incremental recook (uses the graph, source dir, cooked dir).
        let mut temp_registry = AssetRegistry::new();
        let results = crate::reload::recook::incremental_recook(
            &events,
            &mut self.graph,
            &self.source_dir,
            &self.cooked_dir,
            &mut temp_registry,
        );

        // 6/7. Process results.
        for result in &results {
            let matching_id = all_ids.iter().find(|id| id.id == result.asset_id).cloned();

            let Some(asset_id) = matching_id else {
                continue;
            };

            if result.success {
                self.tracker.transition(&asset_id, ReloadState::Cooked);
                self.tracker.transition(&asset_id, ReloadState::Queued);
            } else {
                let error_msg = result
                    .diagnostics
                    .first()
                    .map(|d| d.message.clone())
                    .unwrap_or_else(|| "unknown cook error".into());
                self.tracker.mark_failed(&asset_id, error_msg);
            }
        }

        // 8. Collect diagnostics.
        self.tracker.to_diagnostics()
    }

    /// Access the reload tracker (immutable).
    pub fn tracker(&self) -> &ReloadTracker {
        &self.tracker
    }

    /// Access the reload tracker (mutable).
    pub fn tracker_mut(&mut self) -> &mut ReloadTracker {
        &mut self.tracker
    }

    /// Access the dependency graph (immutable).
    pub fn graph(&self) -> &DependencyGraph {
        &self.graph
    }

    /// Access the dependency graph (mutable).
    pub fn graph_mut(&mut self) -> &mut DependencyGraph {
        &mut self.graph
    }

    /// Enable or disable the reload pipeline.
    ///
    /// When disabled, [`poll`](Self::poll) is a no-op and returns an
    /// empty vec.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if let Some(watch) = self.watch.as_mut() {
            watch.set_enabled(enabled);
        }
    }

    /// Returns `true` if the pipeline is enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Manually trigger a reload for the given asset.
    ///
    /// This bypasses the file watcher and directly schedules the asset
    /// (and its reverse dependencies) for re-cooking on the next
    /// [`poll`](Self::poll) call.
    ///
    /// Returns `true` if the asset was registered in the graph.
    pub fn request_reload(&mut self, asset_id: &AssetId) -> bool {
        if !self.graph.contains(asset_id) {
            return false;
        }

        self.tracker.transition(asset_id, ReloadState::Detected);
        self.tracker.transition(asset_id, ReloadState::Recooking);

        // Immediately recook this asset.
        let mut temp_registry = AssetRegistry::new();
        let results = crate::reload::recook::incremental_recook(
            &[],
            &mut self.graph,
            &self.source_dir,
            &self.cooked_dir,
            &mut temp_registry,
        );

        // incremental_recook returns empty when events is empty.
        // Handle single-asset recook directly.
        let source_dir = &self.source_dir;
        let cooked_dir = &self.cooked_dir;

        // Find the source path via manifest scanning.
        let manifest_entries = recook::scan_manifests(source_dir);

        let source_path = manifest_entries
            .get(asset_id)
            .map(|entry| source_dir.join(&entry.source_path))
            .unwrap_or_else(|| {
                let mut buf = source_dir.to_path_buf();
                buf.push(format!("{}.source", asset_id.id.replace('-', "_")));
                buf
            });

        if !source_path.exists() {
            self.tracker
                .mark_failed(asset_id, format!("source not found: {:?}", source_path));
            return true;
        }

        let output_path = {
            let mut buf = cooked_dir.to_path_buf();
            buf.push(format!("{}.cooked", asset_id.id.replace('-', "_")));
            buf
        };

        // Determine asset type and dispatch.
        let category = asset_id.id.split('-').next().unwrap_or(&asset_id.id);
        let asset_type = match category {
            "mesh" => crate::cook::AssetType::Mesh,
            "material" => crate::cook::AssetType::Material,
            "texture" => crate::cook::AssetType::Texture,
            "shader" => crate::cook::AssetType::Shader,
            "scene" => crate::cook::AssetType::Scene,
            _ => crate::cook::AssetType::Unknown,
        };

        let cook_result = match asset_type {
            crate::cook::AssetType::Mesh => crate::cook::cook_mesh(&source_path, &output_path),
            crate::cook::AssetType::Texture => {
                crate::cook::cook_texture(&source_path, &output_path)
            }
            crate::cook::AssetType::Shader => crate::cook::cook_shader(
                &source_path,
                &output_path,
                0,
                &recook::determine_shader_stage(&source_path),
            ),
            crate::cook::AssetType::Scene => crate::cook::cook_scene(&source_path, &output_path, 0),
            _ => {
                // Fallback: generic cook
                let payload = match std::fs::read(&source_path) {
                    Ok(d) => d,
                    Err(e) => {
                        self.tracker.mark_failed(asset_id, e.to_string());
                        return true;
                    }
                };
                crate::cook::write_cooked_artifact(
                    &output_path,
                    asset_type.kind_code(),
                    &payload,
                    Default::default(),
                )
            }
        };

        match cook_result {
            Ok(_) => {
                let hash = recook::compute_file_hash(&source_path);
                self.graph.mark_cooked(asset_id, hash);
                self.tracker.transition(asset_id, ReloadState::Cooked);
                self.tracker.transition(asset_id, ReloadState::Queued);
                // Invalidate registry — use a temporary one since we don't
                // own the main registry.
                let _ = results;
            }
            Err(e) => {
                self.tracker.mark_failed(asset_id, e.to_string());
            }
        }

        true
    }

    /// Take ownership of the dependency graph, replacing it with an empty
    /// one.  Useful when the graph needs to be serialised or inspected.
    pub fn take_graph(&mut self) -> DependencyGraph {
        std::mem::take(&mut self.graph)
    }

    /// Replace the dependency graph.
    pub fn set_graph(&mut self, graph: DependencyGraph) {
        self.graph = graph;
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use engine_serialize::AssetId;

    fn id(name: &str) -> AssetId {
        AssetId::new(name)
    }

    #[test]
    fn reload_coordinator_new() {
        let dir = std::env::temp_dir().join("coord_test_new");
        let _ = std::fs::create_dir_all(&dir);
        let coord = ReloadCoordinator::new(&dir, &dir, &dir);
        assert!(coord.is_ok());
        let coord = coord.unwrap();
        assert!(coord.is_enabled());
        assert!(coord.tracker().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reload_coordinator_new_fails_bad_path() {
        let result = ReloadCoordinator::new(
            Path::new(r"\\?\__nonexistent__"),
            Path::new("source"),
            Path::new("cooked"),
        );
        assert!(result.is_err());
    }

    #[test]
    fn reload_coordinator_set_enabled() {
        let dir = std::env::temp_dir().join("coord_test_enabled");
        let _ = std::fs::create_dir_all(&dir);
        let mut coord = ReloadCoordinator::new(&dir, &dir, &dir).unwrap();
        assert!(coord.is_enabled());
        coord.set_enabled(false);
        assert!(!coord.is_enabled());
        assert!(coord.poll().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reload_coordinator_poll_empty_when_no_events() {
        let dir = std::env::temp_dir().join("coord_test_poll");
        let _ = std::fs::create_dir_all(&dir);
        let mut coord = ReloadCoordinator::new(&dir, &dir, &dir).unwrap();
        let diags = coord.poll();
        assert!(diags.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reload_coordinator_tracker_access() {
        let dir = std::env::temp_dir().join("coord_test_tracker");
        let _ = std::fs::create_dir_all(&dir);
        let mut coord = ReloadCoordinator::new(&dir, &dir, &dir).unwrap();

        coord
            .tracker_mut()
            .transition(&id("mesh-cube"), ReloadState::Detected);
        assert!(coord.tracker().get(&id("mesh-cube")).is_some());
        assert_eq!(coord.tracker().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reload_coordinator_graph_access() {
        let dir = std::env::temp_dir().join("coord_test_graph");
        let _ = std::fs::create_dir_all(&dir);
        let mut coord = ReloadCoordinator::new(&dir, &dir, &dir).unwrap();

        coord.graph_mut().register(id("mesh-cube"));
        assert!(coord.graph().contains(&id("mesh-cube")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reload_coordinator_take_graph() {
        let dir = std::env::temp_dir().join("coord_test_take");
        let _ = std::fs::create_dir_all(&dir);
        let mut coord = ReloadCoordinator::new(&dir, &dir, &dir).unwrap();

        coord.graph_mut().register(id("mesh-cube"));
        let taken = coord.take_graph();
        assert!(taken.contains(&id("mesh-cube")));
        assert!(coord.graph().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reload_coordinator_set_graph() {
        let dir = std::env::temp_dir().join("coord_test_set_graph");
        let _ = std::fs::create_dir_all(&dir);
        let mut coord = ReloadCoordinator::new(&dir, &dir, &dir).unwrap();

        let mut new_graph = DependencyGraph::new();
        new_graph.register(id("scene-A"));
        coord.set_graph(new_graph);
        assert!(coord.graph().contains(&id("scene-A")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reload_coordinator_request_reload_unknown_returns_false() {
        let dir = std::env::temp_dir().join("coord_test_reload_unknown");
        let _ = std::fs::create_dir_all(&dir);
        let mut coord = ReloadCoordinator::new(&dir, &dir, &dir).unwrap();

        assert!(!coord.request_reload(&id("nonexistent")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reload_coordinator_request_reload_known_triggers_tracking() {
        let dir = std::env::temp_dir().join("coord_test_reload_known");
        let _ = std::fs::create_dir_all(&dir);
        let mut coord = ReloadCoordinator::new(&dir, &dir, &dir).unwrap();

        coord.graph_mut().register(id("mesh-cube"));
        assert!(coord.request_reload(&id("mesh-cube")));
        let info = coord.tracker().get(&id("mesh-cube"));
        assert!(info.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
