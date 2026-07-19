//! Manifest-driven asset reload pipeline.
//!
//! [`ReloadCoordinator`] is the sole public entry point for file-watch and
//! explicit reload requests. Both triggers resolve exact manifest
//! [`AssetId`] values, expand reverse dependencies, and invoke the same
//! authoritative single-asset cooker.

mod recook;
pub mod state;
pub mod watch;

use std::path::{Path, PathBuf};

use engine_scene::registry::AssetTypeRegistry;
use engine_serialize::{AssetId, Diagnostic};

use crate::cook::DependencyGraph;

pub use state::{ReloadInfo, ReloadState, ReloadTracker};
pub use watch::{WatchCoordinator, WatchEvent, WatchEventKind};

/// Coordinates manifest resolution, debounced file watching, recooking,
/// dependency expansion, and reload-state diagnostics.
pub struct ReloadCoordinator {
    watch: Option<WatchCoordinator>,
    graph: DependencyGraph,
    tracker: ReloadTracker,
    source_dir: PathBuf,
    cooked_dir: PathBuf,
    asset_type_registry: AssetTypeRegistry,
    batch_diagnostics: Vec<Diagnostic>,
    enabled: bool,
}

impl ReloadCoordinator {
    /// Create an enabled coordinator with the engine's built-in asset types.
    ///
    /// Extension-owned types such as audio, animation, skeleton, and navmesh
    /// require [`Self::new_with_registry`] so their registered cooker and
    /// loader hooks are available to both full and incremental cooking.
    pub fn new(
        watch_dir: &Path,
        source_dir: &Path,
        cooked_dir: &Path,
    ) -> Result<Self, crate::AssetError> {
        Self::new_with_registry(
            watch_dir,
            source_dir,
            cooked_dir,
            default_asset_type_registry(),
        )
    }

    /// Create an enabled coordinator using the complete application asset
    /// type registry.
    pub fn new_with_registry(
        watch_dir: &Path,
        source_dir: &Path,
        cooked_dir: &Path,
        asset_type_registry: AssetTypeRegistry,
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
            asset_type_registry,
            batch_diagnostics: Vec::new(),
            enabled: true,
        })
    }

    /// Create a disabled coordinator with the built-in type registry.
    pub fn new_disabled() -> Self {
        Self::new_disabled_with_registry(default_asset_type_registry())
    }

    /// Create a disabled coordinator while retaining an application registry
    /// for a later configured lifecycle.
    pub fn new_disabled_with_registry(asset_type_registry: AssetTypeRegistry) -> Self {
        Self {
            watch: None,
            graph: DependencyGraph::new(),
            tracker: ReloadTracker::new(),
            source_dir: PathBuf::new(),
            cooked_dir: PathBuf::new(),
            asset_type_registry,
            batch_diagnostics: Vec::new(),
            enabled: false,
        }
    }

    /// Return state-machine diagnostics plus manifest and recook diagnostics
    /// from the most recent batch.
    pub fn reload_diagnostics(&self) -> Vec<Diagnostic> {
        let mut diagnostics = self.tracker.to_diagnostics();
        diagnostics.extend(self.batch_diagnostics.iter().cloned());
        diagnostics
    }

    /// Poll debounced filesystem changes and recook every directly or
    /// transitively affected manifest asset.
    pub fn poll(&mut self) -> Vec<Diagnostic> {
        if !self.enabled {
            return Vec::new();
        }

        let events = self
            .watch
            .as_mut()
            .map(WatchCoordinator::poll_events)
            .unwrap_or_default();
        if !events.is_empty() {
            let batch = recook::recook_assets(
                recook::RecookTrigger::WatchEvents(&events),
                &mut self.graph,
                &self.source_dir,
                &self.cooked_dir,
                &self.asset_type_registry,
            );
            self.apply_recook_batch(batch);
        }
        self.reload_diagnostics()
    }

    fn apply_recook_batch(&mut self, batch: recook::RecookBatch) {
        self.batch_diagnostics = batch.diagnostics;
        for result in batch.results {
            let asset_id = batch
                .resolved_ids
                .get(&result.asset_id)
                .cloned()
                .unwrap_or_else(|| AssetId::new(&result.asset_id));
            let source_path = Some(result.source_path.display().to_string());
            self.tracker
                .transition_with_path(&asset_id, ReloadState::Detected, source_path);
            self.tracker.transition(&asset_id, ReloadState::Recooking);
            if result.success {
                self.tracker.transition(&asset_id, ReloadState::Cooked);
                self.tracker.transition(&asset_id, ReloadState::Queued);
            } else {
                let message = result
                    .diagnostics
                    .first()
                    .map(|diagnostic| diagnostic.message.clone())
                    .unwrap_or_else(|| "unknown cook error".to_string());
                self.tracker.mark_failed(&asset_id, message);
                self.batch_diagnostics.extend(result.diagnostics);
            }
        }
    }

    pub fn tracker(&self) -> &ReloadTracker {
        &self.tracker
    }

    pub fn tracker_mut(&mut self) -> &mut ReloadTracker {
        &mut self.tracker
    }

    pub fn graph(&self) -> &DependencyGraph {
        &self.graph
    }

    pub fn graph_mut(&mut self) -> &mut DependencyGraph {
        &mut self.graph
    }

    pub fn asset_type_registry(&self) -> &AssetTypeRegistry {
        &self.asset_type_registry
    }

    /// Enable or disable the pipeline. Disabling also clears every buffered
    /// and pending raw filesystem event in the watch coordinator.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if let Some(watch) = self.watch.as_mut() {
            watch.set_enabled(enabled);
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Immediately recook one exact manifest [`AssetId`] and its transitive
    /// reverse dependencies through the same path used by file watching.
    ///
    /// Returns `false` and records an explicit diagnostic when the complete
    /// ID is absent or rejected by manifest validation. No filename, category,
    /// or string-only alias is accepted.
    pub fn request_reload(&mut self, asset_id: &AssetId) -> bool {
        let batch = recook::recook_assets(
            recook::RecookTrigger::Asset(asset_id),
            &mut self.graph,
            &self.source_dir,
            &self.cooked_dir,
            &self.asset_type_registry,
        );
        let matched = batch.matched;
        self.apply_recook_batch(batch);
        matched
    }

    pub fn take_graph(&mut self) -> DependencyGraph {
        std::mem::take(&mut self.graph)
    }

    pub fn set_graph(&mut self, graph: DependencyGraph) {
        self.graph = graph;
    }
}

fn default_asset_type_registry() -> AssetTypeRegistry {
    let mut registry = AssetTypeRegistry::new();
    engine_scene::register_prefab_asset_type(&mut registry);
    registry
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cook::{AssetType, CookRules, SourceAssetEntry, SourceManifest};

    fn id(name: &str) -> AssetId {
        AssetId::new(name)
    }

    fn write_material_project(source: &Path, asset_id: AssetId) {
        std::fs::create_dir_all(source).unwrap();
        std::fs::write(
            source.join("data.material.json"),
            serde_json::to_vec(&serde_json::json!({
                "schema": crate::cook::MATERIAL_SOURCE_SCHEMA,
                "base_color": [1.0, 1.0, 1.0, 1.0],
                "metallic": 0.0,
                "roughness": 0.5,
                "ambient_occlusion": 1.0,
                "transparency": "Opaque",
                "double_sided": false
            }))
            .unwrap(),
        )
        .unwrap();
        let manifest = SourceManifest {
            schema_version: crate::cook::manifest::CURRENT_MANIFEST_VERSION,
            assets: vec![SourceAssetEntry {
                id: asset_id,
                asset_type: AssetType::Material,
                source_path: "data.material.json".into(),
                cook_rules: CookRules::default(),
            }],
        };
        std::fs::write(
            source.join("assets.manifest"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
    }

    fn write_material_manifest(source: &Path, entries: &[(&AssetId, &str)]) {
        std::fs::create_dir_all(source).unwrap();
        let assets = entries
            .iter()
            .map(|(asset_id, file_name)| {
                std::fs::write(
                    source.join(*file_name),
                    serde_json::to_vec(&serde_json::json!({
                        "schema": crate::cook::MATERIAL_SOURCE_SCHEMA,
                        "base_color": [1.0, 1.0, 1.0, 1.0],
                        "metallic": 0.0,
                        "roughness": 0.5,
                        "ambient_occlusion": 1.0,
                        "transparency": "Opaque",
                        "double_sided": false
                    }))
                    .unwrap(),
                )
                .unwrap();
                SourceAssetEntry {
                    id: (*asset_id).clone(),
                    asset_type: AssetType::Material,
                    source_path: (*file_name).into(),
                    cook_rules: CookRules::default(),
                }
            })
            .collect();
        let manifest = SourceManifest {
            schema_version: crate::cook::manifest::CURRENT_MANIFEST_VERSION,
            assets,
        };
        std::fs::write(
            source.join("assets.manifest"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn coordinator_starts_enabled_and_empty() {
        let root = tempfile::tempdir().unwrap();
        let coordinator = ReloadCoordinator::new(root.path(), root.path(), root.path()).unwrap();
        assert!(coordinator.is_enabled());
        assert!(coordinator.tracker().is_empty());
        assert!(coordinator.reload_diagnostics().is_empty());
    }

    #[test]
    fn coordinator_rejects_missing_watch_directory() {
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("missing");
        assert!(ReloadCoordinator::new(&missing, &missing, &missing).is_err());
    }

    #[test]
    fn disabled_poll_is_empty_and_graph_can_be_replaced() {
        let root = tempfile::tempdir().unwrap();
        let mut coordinator =
            ReloadCoordinator::new(root.path(), root.path(), root.path()).unwrap();
        coordinator.graph_mut().register(id("mesh-cube"));
        let graph = coordinator.take_graph();
        assert!(graph.contains(&id("mesh-cube")));
        coordinator.set_graph(graph);
        coordinator.set_enabled(false);
        assert!(!coordinator.is_enabled());
        assert!(coordinator.poll().is_empty());
    }

    #[test]
    fn unknown_request_returns_false_with_manifest_diagnostic() {
        let root = tempfile::tempdir().unwrap();
        let mut coordinator =
            ReloadCoordinator::new(root.path(), root.path(), root.path()).unwrap();
        assert!(!coordinator.request_reload(&id("nonexistent")));
        assert!(coordinator
            .reload_diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code == "RECOOK_ASSET_NOT_DECLARED"));
    }

    #[test]
    fn request_reload_tracks_exact_manifest_id_and_writes_artifact() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let cooked = root.path().join("cooked");
        let asset_id = AssetId::with_path("data-material", "materials/data");
        write_material_project(&source, asset_id.clone());
        let mut coordinator = ReloadCoordinator::new(&source, &source, &cooked).unwrap();

        assert!(!coordinator.request_reload(&id("data-material")));
        assert!(coordinator.request_reload(&asset_id));
        assert_eq!(
            coordinator.tracker().get(&asset_id).unwrap().state,
            ReloadState::Queued
        );
        assert!(cooked.join("data-material.cooked").is_file());
    }

    #[test]
    fn full_registry_is_retained_for_extension_recooks() {
        use engine_scene::registry::{AssetTypeExtension, AssetTypeMeta};

        let root = tempfile::tempdir().unwrap();
        let mut registry = AssetTypeRegistry::new();
        registry
            .register(AssetTypeExtension {
                meta: AssetTypeMeta {
                    type_id: "audio_clip",
                    source_extensions: vec!["wav"],
                    display_name: "Audio Clip",
                },
                cooker: None,
                loader: None,
            })
            .unwrap();
        let coordinator =
            ReloadCoordinator::new_with_registry(root.path(), root.path(), root.path(), registry)
                .unwrap();
        assert!(coordinator
            .asset_type_registry()
            .get("audio_clip")
            .is_some());
    }

    #[test]
    fn reverse_dependencies_share_tracking_and_recook_state() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let cooked = root.path().join("cooked");
        let base = id("base-material");
        let dependent = id("dependent-material");
        write_material_manifest(
            &source,
            &[
                (&base, "base.material.json"),
                (&dependent, "dependent.material.json"),
            ],
        );
        let mut coordinator = ReloadCoordinator::new(&source, &source, &cooked).unwrap();
        coordinator
            .graph_mut()
            .add_dependency(dependent.clone(), base.clone());

        assert!(coordinator.request_reload(&base));
        assert_eq!(
            coordinator.tracker().get(&base).unwrap().state,
            ReloadState::Queued
        );
        assert_eq!(
            coordinator.tracker().get(&dependent).unwrap().state,
            ReloadState::Queued
        );
        assert!(cooked.join("base-material.cooked").is_file());
        assert!(cooked.join("dependent-material.cooked").is_file());
    }
}
