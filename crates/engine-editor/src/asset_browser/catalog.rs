use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use engine_asset::cook::manifest::CURRENT_MANIFEST_VERSION;
use engine_asset::cook::SourceManifest;
use engine_asset::{validate_asset_id, AssetRegistry};
use engine_renderer::{
    EnvironmentMapUpload, MaterialUpload, MeshUpload, MorphTargetSetUpload, TextureUpload,
};
use engine_serialize::AssetId;

use super::{
    AssetBrowserPanel, AssetBrowserRefreshError, AssetEntry, AssetKind, AssetRefreshSummary,
};

/// Refresh the complete project asset catalog from authoritative source
/// manifests and the live registry.
///
/// This uses the cooker's manifest discovery rules: only direct children of
/// `source_root` whose extension is `.manifest` (case-insensitive) are read,
/// and files are processed in a deterministic case-insensitive name order.
/// Every manifest is parsed and validated before the panel snapshot changes,
/// so a malformed manifest never leaves a partial catalog behind.
///
/// Manifest entries are authoritative for kind and source path. Cached assets
/// that are not declared by any manifest are then merged as registry-only
/// entries, including unknown/non-rendering types. Tool-owned `editor/*`
/// cache entries remain private and are not project content.
pub fn refresh_project_asset_list(
    panel: &mut AssetBrowserPanel,
    registry: &AssetRegistry,
    source_root: &Path,
) -> Result<AssetRefreshSummary, AssetBrowserRefreshError> {
    let source_folders = collect_source_folders(source_root)?;
    let directory = std::fs::read_dir(source_root).map_err(|source| {
        AssetBrowserRefreshError::SourceRootRead {
            path: source_root.to_path_buf(),
            source,
        }
    })?;

    let mut manifest_paths = Vec::new();
    for directory_entry in directory {
        let directory_entry =
            directory_entry.map_err(|source| AssetBrowserRefreshError::SourceEntryRead {
                path: source_root.to_path_buf(),
                source,
            })?;
        let path = directory_entry.path();
        if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("manifest"))
        {
            manifest_paths.push(path);
        }
    }
    manifest_paths.sort_by(|left, right| {
        let left_name = left.file_name().unwrap_or_default().to_string_lossy();
        let right_name = right.file_name().unwrap_or_default().to_string_lossy();
        left_name
            .to_ascii_lowercase()
            .cmp(&right_name.to_ascii_lowercase())
            .then_with(|| left_name.cmp(&right_name))
    });

    let cooked_root = source_root
        .parent()
        .map(|assets_root| assets_root.join("cooked"))
        .unwrap_or_else(|| source_root.join("cooked"));
    let mut entries = Vec::new();
    let mut portable_ids: BTreeMap<String, (String, PathBuf)> = BTreeMap::new();

    for manifest_path in &manifest_paths {
        let content = std::fs::read_to_string(manifest_path).map_err(|source| {
            AssetBrowserRefreshError::ManifestRead {
                path: manifest_path.clone(),
                source,
            }
        })?;
        let manifest: SourceManifest = serde_json::from_str(&content).map_err(|source| {
            AssetBrowserRefreshError::ManifestParse {
                path: manifest_path.clone(),
                source,
            }
        })?;
        if manifest.schema_version != CURRENT_MANIFEST_VERSION {
            return Err(AssetBrowserRefreshError::UnsupportedSchema {
                path: manifest_path.clone(),
                found: manifest.schema_version,
                expected: CURRENT_MANIFEST_VERSION,
            });
        }

        let mut manifest_assets = manifest.assets;
        manifest_assets.sort_by(|left, right| {
            left.id
                .id
                .cmp(&right.id.id)
                .then_with(|| left.source_path.cmp(&right.source_path))
        });
        for source_asset in manifest_assets {
            validate_manifest_asset_id(&source_asset.id).map_err(|detail| {
                AssetBrowserRefreshError::InvalidAssetId {
                    path: manifest_path.clone(),
                    id: source_asset.id.clone(),
                    detail,
                }
            })?;

            let portable_id = source_asset.id.id.to_ascii_lowercase();
            if let Some((first_id, first_manifest)) = portable_ids.get(&portable_id) {
                return Err(AssetBrowserRefreshError::DuplicateAssetId {
                    id: first_id.clone(),
                    first_manifest: first_manifest.clone(),
                    duplicate_manifest: manifest_path.clone(),
                });
            }
            portable_ids.insert(
                portable_id,
                (source_asset.id.id.clone(), manifest_path.clone()),
            );

            let mut entry = AssetEntry::new(
                source_asset.id.clone(),
                AssetKind::from(&source_asset.asset_type),
            );
            entry.source_path = Some(source_asset.source_path);
            entry.loaded = registry.contains(&entry.id);
            entry.cooked = cooked_artifact_path(&cooked_root, &entry.id).is_file();
            entry.manifest_declared = true;
            entries.push(entry);
        }
    }

    let declared_asset_count = entries.len();
    let mut registry_only_asset_count = 0;
    for id in registry.cached_ids() {
        if id.id.starts_with("editor/") || portable_ids.contains_key(&id.id.to_ascii_lowercase()) {
            continue;
        }
        let kind = registry_asset_kind(registry, &id);
        let mut entry = AssetEntry::new(id, kind);
        entry.loaded = true;
        entry.cooked = cooked_artifact_path(&cooked_root, &entry.id).is_file();
        entries.push(entry);
        registry_only_asset_count += 1;
    }

    panel.replace_registry_snapshot(entries, &source_folders);
    Ok(AssetRefreshSummary {
        manifest_count: manifest_paths.len(),
        declared_asset_count,
        registry_only_asset_count,
    })
}

fn collect_source_folders(source_root: &Path) -> Result<Vec<String>, AssetBrowserRefreshError> {
    fn visit(
        source_root: &Path,
        directory: &Path,
        folders: &mut Vec<String>,
    ) -> Result<(), AssetBrowserRefreshError> {
        let entries = std::fs::read_dir(directory).map_err(|source| {
            AssetBrowserRefreshError::SourceEntryRead {
                path: directory.to_path_buf(),
                source,
            }
        })?;
        for entry in entries {
            let entry = entry.map_err(|source| AssetBrowserRefreshError::SourceEntryRead {
                path: directory.to_path_buf(),
                source,
            })?;
            let file_type =
                entry
                    .file_type()
                    .map_err(|source| AssetBrowserRefreshError::SourceEntryRead {
                        path: entry.path(),
                        source,
                    })?;
            if !file_type.is_dir() || file_type.is_symlink() {
                continue;
            }
            let path = entry.path();
            let relative = path.strip_prefix(source_root).map_err(|source| {
                AssetBrowserRefreshError::SourceEntryRead {
                    path: path.clone(),
                    source: std::io::Error::other(source.to_string()),
                }
            })?;
            let folder = relative
                .components()
                .map(|component| component.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            folders.push(format!("/{folder}"));
            visit(source_root, &path, folders)?;
        }
        Ok(())
    }

    let mut folders = Vec::new();
    visit(source_root, source_root, &mut folders)?;
    Ok(folders)
}

fn registry_asset_kind(registry: &AssetRegistry, id: &AssetId) -> AssetKind {
    if registry.get::<MeshUpload>(id).is_some() {
        AssetKind::Mesh
    } else if registry.get::<MaterialUpload>(id).is_some() {
        AssetKind::Material
    } else if registry.get::<TextureUpload>(id).is_some() {
        AssetKind::Texture
    } else if registry.get::<EnvironmentMapUpload>(id).is_some() {
        AssetKind::EnvironmentMap
    } else if registry.get::<MorphTargetSetUpload>(id).is_some() {
        AssetKind::MorphTargetSet
    } else {
        AssetKind::Unknown
    }
}

fn cooked_artifact_path(cooked_root: &Path, id: &AssetId) -> PathBuf {
    cooked_root.join(format!("{}.cooked", id.id))
}

fn validate_manifest_asset_id(id: &AssetId) -> Result<(), String> {
    if id.id.is_empty() || id.id.len() > 128 {
        return Err("asset id must contain between 1 and 128 ASCII characters".into());
    }
    if !id
        .id
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_alphanumeric())
    {
        return Err("asset id must start with an ASCII letter or digit".into());
    }
    if !id
        .id
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.'))
    {
        return Err(
            "asset id may contain only ASCII letters, digits, hyphens, underscores, and dots"
                .into(),
        );
    }
    validate_asset_id(id).map_err(|error| error.to_string())
}
