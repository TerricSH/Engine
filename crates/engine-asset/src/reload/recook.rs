//! Incremental re-cook pipeline for hot-reload.
//!
//! Takes file-system [`WatchEvent`]s, resolves them to assets in the
//! [`DependencyGraph`], and re-cooks each affected asset (including
//! reverse dependencies) through the appropriate cooker function.
//!
//! This is the "subset" equivalent of [`cook_orchestrate`] — rather than
//! scanning all manifests, it operates on the specific set of assets
//! identified by file-change events.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use engine_serialize::{AssetId, Diagnostic, DiagnosticSeverity, HashDigest};
use sha2::{Digest, Sha256};

use super::watch::WatchEvent;
use crate::cook::dependency::DependencyGraph;
use crate::cook::manifest::{AssetType, SourceAssetEntry, SourceManifest};
use crate::cook::{
    cook_mesh, cook_scene, cook_shader, cook_texture, write_cooked_artifact, CookResult,
};
use crate::hot_reload::path_to_asset_id;
use crate::registry::AssetRegistry;

// ---------------------------------------------------------------------------
// Incremental recook
// ---------------------------------------------------------------------------

/// Incrementally re-cook assets affected by a batch of file-change events.
///
/// # Algorithm
///
/// 1. For each [`WatchEvent`], convert the changed path to an [`AssetId`]
///    using the standard `path_to_asset_id` convention.
/// 2. Collect all directly affected assets plus their reverse dependencies
///    (assets that depend on the changed file) from the dependency graph.
/// 3. For each unique asset, determine the [`AssetType`] from its id
///    category prefix, find the source file, dispatch to the appropriate
///    cooker, and update the graph.
/// 4. Return a [`CookResult`] for each asset that was re-cooked (or
///    attempted).
///
/// # Parameters
///
/// * `events`     – debounced file-change events from the watch coordinator.
/// * `graph`      – mutable dependency graph (updated with new cook state).
/// * `source_dir` – directory containing source manifests and assets.
/// * `cooked_dir` – directory where cooked artifacts are written.
/// * `registry`   – asset registry for cache invalidation after cooking.
///
/// # Returns
///
/// A vector of [`CookResult`] values, one per attempted re-cook.  Failed
/// cooks are reflected in the result's `success` field and in the graph.
pub fn incremental_recook(
    events: &[WatchEvent],
    graph: &mut DependencyGraph,
    source_dir: &Path,
    cooked_dir: &Path,
    registry: &mut AssetRegistry,
) -> Vec<CookResult> {
    if events.is_empty() {
        return Vec::new();
    }

    // ── Step 1: Resolve paths to AssetIds ──────────────────────────────
    let mut affected = BTreeSet::new();

    for event in events {
        if let Some(asset_id) = path_to_asset_id(&event.path) {
            affected.insert(asset_id);
        } else {
            tracing::debug!(
                path = %event.path.display(),
                "watch event path could not be resolved to an AssetId"
            );
        }
    }

    // ── Step 2: Expand with reverse dependencies ───────────────────────
    // Collect all reverse deps from the graph for each directly-affected asset.
    let mut all_affected: BTreeSet<AssetId> = affected.clone();

    for id in &affected {
        let rev_deps = graph.get_reverse_dependencies(id);
        for rev in rev_deps {
            all_affected.insert(rev);
        }
    }

    if all_affected.is_empty() {
        tracing::debug!("no assets resolved from watch events");
        return Vec::new();
    }

    // ── Step 3: Scan manifests for source_path mappings ────────────────
    let manifest_entries = scan_manifests(source_dir);

    // ── Step 4: Recook each affected asset ─────────────────────────────
    let mut results: Vec<CookResult> = Vec::new();

    for id in &all_affected {
        // Determine asset type from category prefix.
        let asset_type = category_to_asset_type(&id.id);

        // Resolve source path from manifest if available, else fallback.
        let source_path = manifest_entries
            .get(id)
            .map(|entry| source_dir.join(&entry.source_path))
            .unwrap_or_else(|| resolve_source_fallback(id, source_dir));

        if !source_path.exists() {
            let err_msg = format!("source file not found: {:?}", source_path.display());
            tracing::error!(asset_id = %id.id, "{err_msg}");
            graph.mark_failed(id, err_msg.clone());
            results.push(CookResult {
                asset_id: id.id.clone(),
                asset_type: asset_type.clone(),
                output_path: PathBuf::new(),
                source_path,
                success: false,
                diagnostics: vec![Diagnostic::new(
                    "RECOOK_SOURCE_MISSING",
                    DiagnosticSeverity::Error,
                    "reload",
                    err_msg,
                )],
            });
            continue;
        }

        // Compute output path.
        let output_path = resolve_cooked_path(id, cooked_dir);

        // Dispatch to the appropriate cooker.
        let result = match &asset_type {
            AssetType::Mesh => cook_mesh(&source_path, &output_path),
            AssetType::Texture => cook_texture(&source_path, &output_path),
            AssetType::Shader => {
                let stage = determine_shader_stage(&source_path);
                cook_shader(&source_path, &output_path, 0, &stage)
            }
            AssetType::Scene => cook_scene(&source_path, &output_path, 0),
            // For material, pipeline, script, audio, font, animation,
            // skeleton, navmesh — use a generic pass-through cooker
            // that copies the source as-is.
            other => generic_cook(other, &source_path, &output_path),
        };

        match result {
            Ok(mut cook_result) => {
                // Mark the graph as cooked.
                let hash = compute_file_hash(&source_path);
                graph.mark_cooked(id, hash);

                // Invalidate the registry cache so the next load re-reads.
                if registry.contains(id) {
                    let _ = registry.reload(id);
                }

                tracing::info!(
                    asset_id = %id.id,
                    "incremental recook succeeded"
                );
                cook_result.success = true;
                results.push(cook_result);
            }
            Err(e) => {
                let err_msg = e.to_string();
                graph.mark_failed(id, err_msg.clone());
                tracing::error!(asset_id = %id.id, error = %err_msg, "incremental recook failed");
                results.push(CookResult {
                    asset_id: id.id.clone(),
                    asset_type: asset_type.clone(),
                    output_path,
                    source_path,
                    success: false,
                    diagnostics: vec![Diagnostic::new(
                        "RECOOK_FAILED",
                        DiagnosticSeverity::Error,
                        "reload",
                        format!("cook failed for {}: {err_msg}", id.id),
                    )],
                });
            }
        }
    }

    results
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Map the category prefix of an AssetId string to an [`AssetType`].
///
/// The category is the portion of the id before the first hyphen.
/// If the category matches a known asset type, that type is returned;
/// otherwise [`AssetType::Unknown`] is used.
fn category_to_asset_type(id_str: &str) -> AssetType {
    let category = id_str.split('-').next().unwrap_or(id_str);
    match category {
        "mesh" => AssetType::Mesh,
        "material" => AssetType::Material,
        "texture" => AssetType::Texture,
        "shader" => AssetType::Shader,
        "scene" => AssetType::Scene,
        "prefab" => AssetType::Mesh, // prefabs are mesh-like at the cook level
        "animation" => AssetType::Animation,
        "audio" => AssetType::Audio,
        "font" => AssetType::Font,
        "pipeline" => AssetType::Pipeline,
        "navmesh" => AssetType::NavMesh,
        "script" => AssetType::Script,
        "skeleton" => AssetType::Skeleton,
        "logic" => AssetType::Logic,
        _ => AssetType::Unknown,
    }
}

/// Scan all `.manifest` files in `source_dir` and build a map from
/// [`AssetId`] to [`SourceAssetEntry`].
pub(super) fn scan_manifests(
    source_dir: &Path,
) -> std::collections::BTreeMap<AssetId, SourceAssetEntry> {
    let mut entries = std::collections::BTreeMap::new();

    let dir = match std::fs::read_dir(source_dir) {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!(
                dir = %source_dir.display(),
                "cannot scan manifest directory: {e}"
            );
            return entries;
        }
    };

    for entry in dir.filter_map(|r| r.ok()) {
        let path = entry.path();
        if path.extension().map_or(true, |e| e != "manifest") {
            continue;
        }

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("failed to read manifest {:?}: {e}", path);
                continue;
            }
        };

        let manifest: SourceManifest = match serde_json::from_str(&content) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!("failed to parse manifest {:?}: {e}", path);
                continue;
            }
        };

        for asset_entry in &manifest.assets {
            entries.insert(asset_entry.id.clone(), asset_entry.clone());
        }
    }

    entries
}

/// Fallback source path resolution when the asset is not found in any
/// manifest.
///
/// Uses the convention: `source_dir/{id_with_underscores}.source`.
fn resolve_source_fallback(id: &AssetId, source_dir: &Path) -> PathBuf {
    let mut buf = source_dir.to_path_buf();
    buf.push(format!("{}.source", id.id.replace('-', "_")));
    buf
}

/// Resolve the cooked artifact path for an asset under `cooked_dir`.
fn resolve_cooked_path(id: &AssetId, cooked_dir: &Path) -> PathBuf {
    let mut buf = cooked_dir.to_path_buf();
    buf.push(format!("{}.cooked", id.id.replace('-', "_")));
    buf
}

/// Determine the shader stage from a file extension.
pub(super) fn determine_shader_stage(path: &Path) -> String {
    match path.extension().and_then(|e| e.to_str()) {
        Some("vert") => "vertex".into(),
        Some("frag") => "fragment".into(),
        Some("comp") => "compute".into(),
        Some("geom") => "geometry".into(),
        Some("tesc") => "tess_control".into(),
        Some("tese") => "tess_eval".into(),
        _ => "vertex".into(),
    }
}

/// Compute a SHA-256 hash of a file's contents.
pub(super) fn compute_file_hash(path: &Path) -> HashDigest {
    let data = std::fs::read(path).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(&data);
    hasher.finalize().into()
}

/// Generic cooker for asset types that don't have a specialised cooker.
///
/// Reads the source file and writes it as a cooked artifact with the
/// appropriate kind code.
fn generic_cook(
    asset_type: &AssetType,
    source: &Path,
    output: &Path,
) -> Result<CookResult, crate::cook::error::CookError> {
    let payload = std::fs::read(source)?;
    let kind_code = asset_type.kind_code();
    let result = write_cooked_artifact(output, kind_code, &payload, Default::default())?;
    Ok(result)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cook::DependencyGraph;

    fn id(name: &str) -> AssetId {
        AssetId::new(name)
    }

    #[test]
    fn category_to_asset_type_mesh() {
        assert_eq!(category_to_asset_type("mesh-cube"), AssetType::Mesh);
    }

    #[test]
    fn category_to_asset_type_texture() {
        assert_eq!(category_to_asset_type("texture-floor"), AssetType::Texture);
    }

    #[test]
    fn category_to_asset_type_shader() {
        assert_eq!(category_to_asset_type("shader-std"), AssetType::Shader);
    }

    #[test]
    fn category_to_asset_type_scene() {
        assert_eq!(category_to_asset_type("scene-level1"), AssetType::Scene);
    }

    #[test]
    fn category_to_asset_type_logic() {
        assert_eq!(category_to_asset_type("logic-enemy_bt"), AssetType::Logic);
    }

    #[test]
    fn category_to_asset_type_no_hyphen() {
        assert_eq!(category_to_asset_type("simple"), AssetType::Unknown);
    }

    #[test]
    fn category_to_asset_type_prefab_maps_to_mesh() {
        assert_eq!(category_to_asset_type("prefab-enemy"), AssetType::Mesh);
    }

    #[test]
    fn incremental_recook_empty_events() {
        let mut graph = DependencyGraph::new();
        let mut registry = AssetRegistry::new();
        let source_dir = Path::new("assets/source");
        let cooked_dir = Path::new("assets/cooked");

        let results = incremental_recook(&[], &mut graph, source_dir, cooked_dir, &mut registry);
        assert!(results.is_empty());
    }

    #[test]
    fn resolve_source_fallback_convention() {
        let asset_id = id("mesh-cube");
        let path = resolve_source_fallback(&asset_id, Path::new("source"));
        assert!(path.to_string_lossy().contains("mesh_cube.source"));
    }

    #[test]
    fn resolve_cooked_path_convention() {
        let asset_id = id("mesh-cube");
        let path = resolve_cooked_path(&asset_id, Path::new("cooked"));
        assert!(path.to_string_lossy().contains("mesh_cube.cooked"));
    }

    #[test]
    fn scan_manifests_empty_dir() {
        let dir = std::env::temp_dir().join("recook_test_empty");
        let _ = std::fs::create_dir_all(&dir);
        let entries = scan_manifests(&dir);
        assert!(entries.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn determine_shader_stage_by_extension() {
        assert_eq!(determine_shader_stage(Path::new("shader.vert")), "vertex");
        assert_eq!(determine_shader_stage(Path::new("shader.frag")), "fragment");
        assert_eq!(determine_shader_stage(Path::new("shader.comp")), "compute");
    }
}
