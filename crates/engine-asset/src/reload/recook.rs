//! Manifest-driven incremental asset recooking.
//!
//! File-watch events and explicit reload requests are resolved through the
//! same validated source-manifest catalog. Both paths call the authoritative
//! single-asset cooker in [`crate::cook::cook_source_entry_atomic`].

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

use engine_scene::registry::AssetTypeRegistry;
use engine_serialize::{AssetId, Diagnostic, DiagnosticSeverity};

use super::watch::WatchEvent;
use crate::cook::dependency::DependencyGraph;
use crate::cook::manifest::{AssetType, SourceAssetEntry, SourceManifest};
use crate::cook::{
    cook_source_entry_atomic, resolve_source_path, validate_manifest_asset_id, CookResult,
};

/// Complete outcome of one incremental recook batch.
#[derive(Default)]
pub(super) struct RecookBatch {
    /// One result for every asset whose cooker was attempted.
    pub results: Vec<CookResult>,
    /// Manifest, path-resolution, and batch diagnostics not represented by a
    /// successful cooker result.
    pub diagnostics: Vec<Diagnostic>,
    /// Whether at least one manifest-declared asset matched the request.
    pub matched: bool,
    /// Exact manifest IDs corresponding to cook results. This preserves the
    /// logical path instead of reconstructing a lossy string-only ID.
    pub resolved_ids: BTreeMap<String, AssetId>,
}

#[derive(Default)]
struct ManifestCatalog {
    entries: BTreeMap<AssetId, SourceAssetEntry>,
    paths: BTreeMap<String, BTreeSet<AssetId>>,
    diagnostics: Vec<Diagnostic>,
}

/// The validated trigger for the one incremental recook implementation.
pub(super) enum RecookTrigger<'a> {
    WatchEvents(&'a [WatchEvent]),
    Asset(&'a AssetId),
}

/// Recook manifest-declared assets selected by a filesystem or explicit-ID trigger.
///
/// Both trigger kinds deliberately share manifest scanning, exact [`AssetId`]
/// resolution, reverse-dependency expansion, cooking, and result tracking.
pub(super) fn recook_assets(
    trigger: RecookTrigger<'_>,
    graph: &mut DependencyGraph,
    source_dir: &Path,
    cooked_dir: &Path,
    asset_type_registry: &AssetTypeRegistry,
) -> RecookBatch {
    if matches!(trigger, RecookTrigger::WatchEvents(events) if events.is_empty()) {
        return RecookBatch::default();
    }

    let catalog = scan_manifests(source_dir);
    let mut batch = RecookBatch {
        diagnostics: catalog.diagnostics.clone(),
        ..RecookBatch::default()
    };
    let mut directly_affected = BTreeSet::new();
    match trigger {
        RecookTrigger::WatchEvents(events) => {
            for event in events {
                let comparable = comparable_path(&event.path);
                if let Some(asset_ids) = catalog.paths.get(&comparable) {
                    directly_affected.extend(asset_ids.iter().cloned());
                } else {
                    let mut diagnostic = reload_diagnostic(
                        "RECOOK_EVENT_PATH_NOT_DECLARED",
                        DiagnosticSeverity::Warning,
                        format!(
                            "changed path '{}' is not declared by a valid source manifest",
                            event.path.display()
                        ),
                        None,
                    );
                    diagnostic.path = Some(event.path.display().to_string());
                    batch.diagnostics.push(diagnostic);
                }
            }
        }
        RecookTrigger::Asset(asset_id) => {
            if !catalog.entries.contains_key(asset_id) {
                batch.diagnostics.push(reload_diagnostic(
                    "RECOOK_ASSET_NOT_DECLARED",
                    DiagnosticSeverity::Error,
                    format!(
                        "asset '{}' with logical path {:?} is not declared by a valid source manifest",
                        asset_id.id, asset_id.logical_path
                    ),
                    Some(asset_id.clone()),
                ));
                return batch;
            }
            directly_affected.insert(asset_id.clone());
        }
    }
    batch.matched = !directly_affected.is_empty();
    recook_affected(
        directly_affected,
        &catalog.entries,
        graph,
        source_dir,
        cooked_dir,
        asset_type_registry,
        &mut batch,
    );
    batch
}

fn recook_affected(
    directly_affected: BTreeSet<AssetId>,
    entries: &BTreeMap<AssetId, SourceAssetEntry>,
    graph: &mut DependencyGraph,
    source_dir: &Path,
    cooked_dir: &Path,
    asset_type_registry: &AssetTypeRegistry,
    batch: &mut RecookBatch,
) {
    let all_affected = reverse_dependency_closure(graph, directly_affected);
    for asset_id in all_affected {
        batch
            .resolved_ids
            .insert(asset_id.id.clone(), asset_id.clone());
        let Some(entry) = entries.get(&asset_id) else {
            let message = format!(
                "asset '{}' is a reverse dependency but has no valid source-manifest entry",
                asset_id.id
            );
            graph.register(asset_id.clone());
            graph.mark_failed(&asset_id, message.clone());
            batch.results.push(failed_result(
                &asset_id,
                AssetType::Unknown,
                PathBuf::new(),
                PathBuf::new(),
                "RECOOK_MANIFEST_ENTRY_MISSING",
                message,
            ));
            continue;
        };

        graph.register(asset_id.clone());
        graph.mark_cooking(&asset_id);
        match cook_source_entry_atomic(source_dir, cooked_dir, entry, asset_type_registry) {
            Ok(cooked) => {
                graph.mark_cooked(&asset_id, cooked.source_hash);
                batch.results.push(cooked.result);
            }
            Err(error) => {
                graph.mark_failed(&asset_id, error.message.clone());
                let mut diagnostic = reload_diagnostic(
                    "RECOOK_ASSET_FAILED",
                    DiagnosticSeverity::Error,
                    format!("recook failed for '{}': {}", asset_id.id, error.message),
                    Some(asset_id.clone()),
                );
                diagnostic.path = Some(entry.source_path.clone());
                diagnostic
                    .fields
                    .insert("cook_code".into(), error.code.into());
                batch.results.push(CookResult {
                    asset_id: asset_id.id.clone(),
                    asset_type: entry.asset_type.clone(),
                    output_path: PathBuf::from(format!("{}.cooked", asset_id.id)),
                    source_path: PathBuf::from(&entry.source_path),
                    success: false,
                    diagnostics: vec![diagnostic],
                });
            }
        }
    }
}

fn reverse_dependency_closure(
    graph: &DependencyGraph,
    roots: BTreeSet<AssetId>,
) -> BTreeSet<AssetId> {
    let mut affected = roots;
    let mut queue = affected.iter().cloned().collect::<VecDeque<_>>();
    while let Some(asset_id) = queue.pop_front() {
        for dependent in graph.get_reverse_dependencies(&asset_id) {
            if affected.insert(dependent.clone()) {
                queue.push_back(dependent);
            }
        }
    }
    affected
}

fn scan_manifests(source_dir: &Path) -> ManifestCatalog {
    let mut catalog = ManifestCatalog::default();
    let read_dir = match std::fs::read_dir(source_dir) {
        Ok(read_dir) => read_dir,
        Err(error) => {
            catalog.diagnostics.push(reload_diagnostic(
                "RECOOK_MANIFEST_DIRECTORY_READ_FAILED",
                DiagnosticSeverity::Error,
                format!(
                    "could not read source manifest directory '{}': {error}",
                    source_dir.display()
                ),
                None,
            ));
            return catalog;
        }
    };

    let mut manifest_paths = read_dir
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("manifest"))
        })
        .collect::<Vec<_>>();
    manifest_paths.sort_by_key(|path| path.to_string_lossy().to_ascii_lowercase());

    let mut portable_ids = BTreeMap::<String, AssetId>::new();
    let mut rejected_ids = BTreeSet::new();
    let mut manifest_origins = BTreeMap::<AssetId, String>::new();
    for manifest_path in manifest_paths {
        let manifest_name = manifest_path.display().to_string();
        let comparable_manifest_path = comparable_path(&manifest_path);
        let bytes = match std::fs::read(&manifest_path) {
            Ok(bytes) => bytes,
            Err(error) => {
                catalog.diagnostics.push(reload_diagnostic(
                    "RECOOK_MANIFEST_READ_FAILED",
                    DiagnosticSeverity::Error,
                    format!("could not read manifest '{manifest_name}': {error}"),
                    None,
                ));
                continue;
            }
        };
        let mut manifest: SourceManifest = match serde_json::from_slice(&bytes) {
            Ok(manifest) => manifest,
            Err(error) => {
                catalog.diagnostics.push(reload_diagnostic(
                    "RECOOK_MANIFEST_PARSE_FAILED",
                    DiagnosticSeverity::Error,
                    format!("could not parse manifest '{manifest_name}': {error}"),
                    None,
                ));
                continue;
            }
        };
        if manifest.schema_version != crate::cook::manifest::CURRENT_MANIFEST_VERSION {
            catalog.diagnostics.push(reload_diagnostic(
                "RECOOK_MANIFEST_VERSION_UNSUPPORTED",
                DiagnosticSeverity::Error,
                format!(
                    "manifest '{manifest_name}' uses schema {}.{}.{}; expected {}.{}.{}",
                    manifest.schema_version.major,
                    manifest.schema_version.minor,
                    manifest.schema_version.patch,
                    crate::cook::manifest::CURRENT_MANIFEST_VERSION.major,
                    crate::cook::manifest::CURRENT_MANIFEST_VERSION.minor,
                    crate::cook::manifest::CURRENT_MANIFEST_VERSION.patch,
                ),
                None,
            ));
            continue;
        }
        manifest.assets.sort_by(|left, right| {
            left.id
                .id
                .cmp(&right.id.id)
                .then_with(|| left.source_path.cmp(&right.source_path))
        });
        for entry in manifest.assets {
            if let Err(message) = validate_manifest_asset_id(&entry.id.id) {
                catalog.diagnostics.push(reload_diagnostic(
                    "RECOOK_ASSET_ID_INVALID",
                    DiagnosticSeverity::Error,
                    message,
                    Some(entry.id),
                ));
                continue;
            }
            let portable_id = entry.id.id.to_ascii_lowercase();
            if rejected_ids.contains(&portable_id) {
                catalog.diagnostics.push(reload_diagnostic(
                    "RECOOK_ASSET_ID_DUPLICATE",
                    DiagnosticSeverity::Error,
                    format!(
                        "asset id '{}' is duplicated or differs only by case",
                        entry.id.id
                    ),
                    Some(entry.id),
                ));
                continue;
            }
            if let Some(previous_id) = portable_ids.remove(&portable_id) {
                rejected_ids.insert(portable_id);
                if let Some(previous) = catalog.entries.remove(&previous_id) {
                    if let Ok(previous_path) =
                        resolve_source_path(source_dir, &previous.source_path)
                    {
                        remove_path_mapping(
                            &mut catalog.paths,
                            &comparable_path(&previous_path),
                            &previous_id,
                        );
                    }
                }
                if let Some(previous_manifest) = manifest_origins.remove(&previous_id) {
                    remove_path_mapping(&mut catalog.paths, &previous_manifest, &previous_id);
                }
                catalog.diagnostics.push(reload_diagnostic(
                    "RECOOK_ASSET_ID_DUPLICATE",
                    DiagnosticSeverity::Error,
                    format!(
                        "asset id '{}' is duplicated or differs only by case",
                        entry.id.id
                    ),
                    Some(entry.id),
                ));
                continue;
            }
            let source_path = match resolve_source_path(source_dir, &entry.source_path) {
                Ok(source_path) => source_path,
                Err(error) => {
                    let mut diagnostic = reload_diagnostic(
                        "RECOOK_SOURCE_PATH_INVALID",
                        DiagnosticSeverity::Error,
                        error.to_string(),
                        Some(entry.id),
                    );
                    diagnostic.path = Some(entry.source_path);
                    catalog.diagnostics.push(diagnostic);
                    continue;
                }
            };
            let id = entry.id.clone();
            portable_ids.insert(portable_id, id.clone());
            catalog
                .paths
                .entry(comparable_path(&source_path))
                .or_default()
                .insert(id.clone());
            catalog
                .paths
                .entry(comparable_manifest_path.clone())
                .or_default()
                .insert(id.clone());
            manifest_origins.insert(id.clone(), comparable_manifest_path.clone());
            catalog.entries.insert(id, entry);
        }
    }
    catalog
}

fn remove_path_mapping(
    paths: &mut BTreeMap<String, BTreeSet<AssetId>>,
    path: &str,
    asset_id: &AssetId,
) {
    let remove_path = if let Some(asset_ids) = paths.get_mut(path) {
        asset_ids.remove(asset_id);
        asset_ids.is_empty()
    } else {
        false
    };
    if remove_path {
        paths.remove(path);
    }
}

fn comparable_path(path: &Path) -> String {
    let normalized = std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .replace('\\', "/");
    if cfg!(windows) {
        normalized.to_ascii_lowercase()
    } else {
        normalized
    }
}

fn failed_result(
    asset_id: &AssetId,
    asset_type: AssetType,
    output_path: PathBuf,
    source_path: PathBuf,
    code: &str,
    message: String,
) -> CookResult {
    CookResult {
        asset_id: asset_id.id.clone(),
        asset_type,
        output_path,
        source_path,
        success: false,
        diagnostics: vec![reload_diagnostic(
            code,
            DiagnosticSeverity::Error,
            message,
            Some(asset_id.clone()),
        )],
    }
}

fn reload_diagnostic(
    code: &str,
    severity: DiagnosticSeverity,
    message: String,
    asset: Option<AssetId>,
) -> Diagnostic {
    let mut diagnostic = Diagnostic::new(code, severity, "reload", message);
    diagnostic.asset = asset;
    diagnostic
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cook::{
        decode_cooked_material, read_cooked_artifact, CookRules, MATERIAL_SOURCE_SCHEMA,
    };
    use crate::reload::watch::WatchEventKind;
    use engine_scene::registry::{AssetTypeExtension, AssetTypeMeta};

    fn material_source(roughness: f32) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "schema": MATERIAL_SOURCE_SCHEMA,
            "base_color": [1.0, 1.0, 1.0, 1.0],
            "metallic": 0.0,
            "roughness": roughness,
            "ambient_occlusion": 1.0,
            "transparency": "Opaque",
            "double_sided": false
        }))
        .unwrap()
    }

    fn manifest_entry(id: AssetId, source_path: &str, asset_type: AssetType) -> SourceAssetEntry {
        SourceAssetEntry {
            id,
            asset_type,
            source_path: source_path.into(),
            cook_rules: CookRules::default(),
        }
    }

    fn write_manifest(source_dir: &Path, entries: Vec<SourceAssetEntry>) {
        let manifest = SourceManifest {
            schema_version: crate::cook::manifest::CURRENT_MANIFEST_VERSION,
            assets: entries,
        };
        std::fs::write(
            source_dir.join("assets.manifest"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
    }

    fn default_registry() -> AssetTypeRegistry {
        let mut registry = AssetTypeRegistry::new();
        engine_scene::register_prefab_asset_type(&mut registry);
        registry
    }

    fn extension_cooker(source: &[u8], output: &mut Vec<u8>) -> Result<(), String> {
        output.extend_from_slice(b"registered:");
        output.extend_from_slice(source);
        Ok(())
    }

    fn extension_loader(cooked: &[u8]) -> Result<Box<dyn std::any::Any + Send + Sync>, String> {
        cooked
            .starts_with(b"registered:")
            .then(|| Box::new(cooked.to_vec()) as Box<dyn std::any::Any + Send + Sync>)
            .ok_or_else(|| "registered payload prefix is missing".to_string())
    }

    fn complete_extension_registry() -> AssetTypeRegistry {
        let mut registry = default_registry();
        for type_id in ["audio_clip", "animation_clip", "skeleton", "navmesh"] {
            registry
                .register(AssetTypeExtension {
                    meta: AssetTypeMeta {
                        type_id,
                        source_extensions: vec!["source"],
                        display_name: type_id,
                    },
                    cooker: Some(extension_cooker),
                    loader: Some(extension_loader),
                })
                .unwrap();
        }
        registry
    }

    #[test]
    fn empty_watch_trigger_is_a_noop() {
        let mut graph = DependencyGraph::new();
        let batch = recook_assets(
            RecookTrigger::WatchEvents(&[]),
            &mut graph,
            Path::new("unused"),
            Path::new("unused"),
            &default_registry(),
        );
        assert!(batch.results.is_empty());
        assert!(batch.diagnostics.is_empty());
    }

    #[test]
    fn event_and_explicit_reload_share_the_material_cooker() {
        let root = tempfile::tempdir().unwrap();
        let source_dir = root.path().join("source");
        let cooked_dir = root.path().join("cooked");
        std::fs::create_dir_all(&source_dir).unwrap();
        let source_path = source_dir.join("sample.material.json");
        std::fs::write(&source_path, material_source(0.5)).unwrap();
        let asset_id = AssetId::new("sample-material");
        write_manifest(
            &source_dir,
            vec![manifest_entry(
                asset_id.clone(),
                "sample.material.json",
                AssetType::Material,
            )],
        );
        let registry = default_registry();
        let mut graph = DependencyGraph::new();

        let watched = recook_assets(
            RecookTrigger::WatchEvents(&[WatchEvent {
                path: source_path.clone(),
                kind: WatchEventKind::Modified,
            }]),
            &mut graph,
            &source_dir,
            &cooked_dir,
            &registry,
        );
        assert!(watched.matched);
        assert_eq!(watched.results.len(), 1);
        assert!(watched.results[0].success);

        std::fs::write(&source_path, material_source(0.25)).unwrap();
        let requested = recook_assets(
            RecookTrigger::Asset(&asset_id),
            &mut graph,
            &source_dir,
            &cooked_dir,
            &registry,
        );
        assert!(requested.matched);
        assert!(requested.results[0].success);
        let artifact = read_cooked_artifact(&cooked_dir.join("sample-material.cooked")).unwrap();
        assert_eq!(decode_cooked_material(&artifact).unwrap().roughness, 0.25);
    }

    #[test]
    fn manifest_change_recooks_every_valid_entry_in_that_manifest() {
        let root = tempfile::tempdir().unwrap();
        let source_dir = root.path().join("source");
        let cooked_dir = root.path().join("cooked");
        std::fs::create_dir_all(&source_dir).unwrap();
        std::fs::write(source_dir.join("one.material.json"), material_source(0.1)).unwrap();
        std::fs::write(source_dir.join("two.material.json"), material_source(0.2)).unwrap();
        write_manifest(
            &source_dir,
            vec![
                manifest_entry(
                    AssetId::new("material-one"),
                    "one.material.json",
                    AssetType::Material,
                ),
                manifest_entry(
                    AssetId::new("material-two"),
                    "two.material.json",
                    AssetType::Material,
                ),
            ],
        );
        let mut graph = DependencyGraph::new();

        let batch = recook_assets(
            RecookTrigger::WatchEvents(&[WatchEvent {
                path: source_dir.join("assets.manifest"),
                kind: WatchEventKind::Modified,
            }]),
            &mut graph,
            &source_dir,
            &cooked_dir,
            &default_registry(),
        );

        assert!(batch.matched);
        assert_eq!(batch.results.len(), 2);
        assert!(batch.results.iter().all(|result| result.success));
    }

    #[test]
    fn failed_recook_preserves_last_valid_artifact() {
        let root = tempfile::tempdir().unwrap();
        let source_dir = root.path().join("source");
        let cooked_dir = root.path().join("cooked");
        std::fs::create_dir_all(&source_dir).unwrap();
        let source_path = source_dir.join("sample.material.json");
        std::fs::write(&source_path, material_source(0.75)).unwrap();
        let asset_id = AssetId::new("stable-material");
        write_manifest(
            &source_dir,
            vec![manifest_entry(
                asset_id.clone(),
                "sample.material.json",
                AssetType::Material,
            )],
        );
        let registry = default_registry();
        let mut graph = DependencyGraph::new();
        assert!(
            recook_assets(
                RecookTrigger::Asset(&asset_id),
                &mut graph,
                &source_dir,
                &cooked_dir,
                &registry,
            )
            .results[0]
                .success
        );
        let prior = std::fs::read(cooked_dir.join("stable-material.cooked")).unwrap();

        std::fs::write(&source_path, b"not valid material json").unwrap();
        let failed = recook_assets(
            RecookTrigger::Asset(&asset_id),
            &mut graph,
            &source_dir,
            &cooked_dir,
            &registry,
        );
        assert!(!failed.results[0].success);
        assert_eq!(
            std::fs::read(cooked_dir.join("stable-material.cooked")).unwrap(),
            prior
        );
    }

    #[test]
    fn exact_asset_id_is_required_and_unknown_is_diagnostic() {
        let root = tempfile::tempdir().unwrap();
        let source_dir = root.path().join("source");
        std::fs::create_dir_all(&source_dir).unwrap();
        std::fs::write(
            source_dir.join("sample.material.json"),
            material_source(0.5),
        )
        .unwrap();
        write_manifest(
            &source_dir,
            vec![manifest_entry(
                AssetId::with_path("material", "materials/source"),
                "sample.material.json",
                AssetType::Material,
            )],
        );
        let mut graph = DependencyGraph::new();
        let requested_id = AssetId::new("material");
        let batch = recook_assets(
            RecookTrigger::Asset(&requested_id),
            &mut graph,
            &source_dir,
            &root.path().join("cooked"),
            &default_registry(),
        );
        assert!(!batch.matched);
        assert!(batch
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "RECOOK_ASSET_NOT_DECLARED"));
    }

    #[test]
    fn duplicate_case_insensitive_ids_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        let source_dir = root.path().join("source");
        std::fs::create_dir_all(&source_dir).unwrap();
        std::fs::write(source_dir.join("one.material.json"), material_source(0.1)).unwrap();
        std::fs::write(source_dir.join("two.material.json"), material_source(0.2)).unwrap();
        write_manifest(
            &source_dir,
            vec![
                manifest_entry(
                    AssetId::new("Duplicate"),
                    "one.material.json",
                    AssetType::Material,
                ),
                manifest_entry(
                    AssetId::new("duplicate"),
                    "two.material.json",
                    AssetType::Material,
                ),
            ],
        );
        let catalog = scan_manifests(&source_dir);
        assert!(catalog.entries.is_empty());
        assert!(catalog
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "RECOOK_ASSET_ID_DUPLICATE"));
    }

    #[test]
    fn reverse_dependencies_are_transitive() {
        let mut graph = DependencyGraph::new();
        let texture = AssetId::new("texture");
        let material = AssetId::new("material");
        let scene = AssetId::new("scene");
        graph.add_dependency(material.clone(), texture.clone());
        graph.add_dependency(scene.clone(), material.clone());
        assert_eq!(
            reverse_dependency_closure(&graph, BTreeSet::from([texture])),
            BTreeSet::from([material, scene, AssetId::new("texture")])
        );
    }

    #[test]
    fn unified_recook_dispatches_every_registry_owned_asset_type() {
        let root = tempfile::tempdir().unwrap();
        let source_dir = root.path().join("source");
        let cooked_dir = root.path().join("cooked");
        std::fs::create_dir_all(&source_dir).unwrap();
        let entries = [
            ("sound", AssetType::Audio),
            ("motion", AssetType::Animation),
            ("rig", AssetType::Skeleton),
            ("navigation", AssetType::NavMesh),
        ]
        .into_iter()
        .map(|(id, asset_type)| {
            let source_path = format!("{id}.source");
            std::fs::write(source_dir.join(&source_path), id.as_bytes()).unwrap();
            manifest_entry(AssetId::new(id), &source_path, asset_type)
        })
        .collect::<Vec<_>>();
        write_manifest(&source_dir, entries.clone());

        let registry = complete_extension_registry();
        let mut graph = DependencyGraph::new();
        for entry in entries {
            let batch = recook_assets(
                RecookTrigger::Asset(&entry.id),
                &mut graph,
                &source_dir,
                &cooked_dir,
                &registry,
            );
            assert!(batch.matched);
            assert!(batch.results[0].success);
            let artifact =
                read_cooked_artifact(&cooked_dir.join(format!("{}.cooked", entry.id.id))).unwrap();
            assert_eq!(artifact.header.asset_kind, entry.asset_type.kind_code());
            assert!(artifact.payload.starts_with(b"registered:"));
        }
    }
}
