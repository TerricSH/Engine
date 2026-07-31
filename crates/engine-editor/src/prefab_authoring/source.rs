use std::collections::BTreeSet;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Component as PathComponent, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use engine_asset::cook::manifest::CURRENT_MANIFEST_VERSION;
use engine_asset::cook::{AssetType, CookRules, SourceAssetEntry, SourceManifest};
use engine_asset::validate_asset_id;
use engine_scene::{
    serialize_prefab_source, validate_prefab_structure, Component, Prefab, PrefabInstanceRef, Scene,
};
use engine_serialize::{AssetId, PersistentId, Value};

use crate::commands::EntityClipboard;

use super::error::join_validation_errors;
use super::PrefabAuthoringError;

static PREFAB_ASSET_WRITE_LOCK: Mutex<()> = Mutex::new(());
pub(super) static PREFAB_TRANSACTION_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Canonical source suffix for prefab assets.
pub const PREFAB_SOURCE_SUFFIX: &str = ".prefab.ron";

/// Explicit filesystem targets for creating one prefab source asset.
///
/// `manifest_path` must be a top-level manifest inside `source_root`, matching
/// the canonical cook scanner. `relative_source_path` must end in
/// `.prefab.ron` and cannot escape `source_root`.
pub struct PrefabAssetCreateRequest<'a> {
    pub source_root: &'a Path,
    pub manifest_path: &'a Path,
    pub relative_source_path: &'a Path,
    pub asset_id: AssetId,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CreatedPrefabAsset {
    pub asset_id: AssetId,
    pub source_path: PathBuf,
    pub manifest_path: PathBuf,
    pub prefab: Prefab,
}

/// Build a prefab document from one scene entity and its complete subtree.
///
/// Existing prefab-instance linkage is stripped so a newly-authored asset does
/// not accidentally point back to another prefab. References to entities
/// outside the captured subtree are rejected instead of silently becoming
/// dangling references.
pub fn prefab_from_scene_subtree(
    scene: &Scene,
    root_entity_id: &PersistentId,
    asset_id: AssetId,
) -> Result<Prefab, PrefabAuthoringError> {
    validate_asset_id(&asset_id)
        .map_err(|error| PrefabAuthoringError::InvalidRequest(error.to_string()))?;
    let clipboard = EntityClipboard::capture(scene, std::slice::from_ref(root_entity_id))?;
    let captured_ids = clipboard
        .entities()
        .iter()
        .map(|record| record.persistent_id.clone())
        .collect::<BTreeSet<_>>();
    let mut hierarchy = clipboard.entities().to_vec();
    for record in &mut hierarchy {
        record.components.remove(PrefabInstanceRef::TYPE_ID);
        if &record.persistent_id == root_entity_id {
            record.parent = None;
            if let Some(transform) = record.components.get_mut("engine.transform") {
                transform.fields.remove("parent");
            }
        }
        for component in record.components.values() {
            for value in component.fields.values() {
                reject_external_entity_reference(value, &captured_ids, &record.persistent_id)?;
            }
        }
    }

    let mut prefab = Prefab::new(asset_id);
    prefab.hierarchy = hierarchy;
    validate_prefab_structure(&prefab)
        .map_err(|errors| PrefabAuthoringError::InvalidPrefab(join_validation_errors(errors)))?;
    Ok(prefab)
}

/// Create a prefab source and its manifest declaration as one recoverable
/// filesystem transaction.
pub fn create_prefab_asset_from_scene(
    scene: &Scene,
    root_entity_id: &PersistentId,
    request: PrefabAssetCreateRequest<'_>,
) -> Result<CreatedPrefabAsset, PrefabAuthoringError> {
    let _guard = PREFAB_ASSET_WRITE_LOCK
        .lock()
        .map_err(|_| PrefabAuthoringError::Io("prefab asset write lock was poisoned".into()))?;
    if !request.source_root.is_dir() {
        return Err(PrefabAuthoringError::InvalidRequest(format!(
            "source root is not a directory: {}",
            request.source_root.display()
        )));
    }
    let relative_source = validate_relative_prefab_path(request.relative_source_path)?;
    reject_symlink_ancestors(request.source_root, &relative_source)?;
    let source_path = request.source_root.join(&relative_source);
    if source_path.exists() {
        return Err(PrefabAuthoringError::InvalidRequest(format!(
            "prefab source already exists: {}",
            source_path.display()
        )));
    }
    let manifest_path = resolve_manifest_path(request.source_root, request.manifest_path)?;
    if manifest_path.exists() {
        let metadata = std::fs::symlink_metadata(&manifest_path).map_err(|error| {
            PrefabAuthoringError::Manifest(format!(
                "could not inspect {}: {error}",
                manifest_path.display()
            ))
        })?;
        if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            return Err(PrefabAuthoringError::Manifest(format!(
                "manifest is not a regular project file: {}",
                manifest_path.display()
            )));
        }
    }
    let prefab = prefab_from_scene_subtree(scene, root_entity_id, request.asset_id.clone())?;
    let mut source_text =
        serialize_prefab_source(&prefab).map_err(PrefabAuthoringError::InvalidPrefab)?;
    source_text.push('\n');

    let mut manifest = if manifest_path.exists() {
        let bytes = std::fs::read(&manifest_path).map_err(|error| {
            PrefabAuthoringError::Manifest(format!(
                "could not read {}: {error}",
                manifest_path.display()
            ))
        })?;
        serde_json::from_slice::<SourceManifest>(&bytes).map_err(|error| {
            PrefabAuthoringError::Manifest(format!(
                "could not parse {}: {error}",
                manifest_path.display()
            ))
        })?
    } else {
        SourceManifest {
            schema_version: CURRENT_MANIFEST_VERSION,
            assets: Vec::new(),
        }
    };
    if manifest.schema_version != CURRENT_MANIFEST_VERSION {
        return Err(PrefabAuthoringError::Manifest(format!(
            "{} uses unsupported manifest schema",
            manifest_path.display()
        )));
    }
    let relative_source_string = portable_path(&relative_source)?;
    let requested_id_key = request.asset_id.id.to_ascii_lowercase();
    let requested_path_key = relative_source_string.to_ascii_lowercase();
    if manifest
        .assets
        .iter()
        .any(|entry| entry.id.id.to_ascii_lowercase() == requested_id_key)
    {
        return Err(PrefabAuthoringError::Manifest(format!(
            "asset ID '{}' is already declared",
            request.asset_id.id
        )));
    }
    if manifest.assets.iter().any(|entry| {
        entry.source_path.replace('\\', "/").to_ascii_lowercase() == requested_path_key
    }) {
        return Err(PrefabAuthoringError::Manifest(format!(
            "source path '{}' is already declared",
            relative_source_string
        )));
    }
    manifest.assets.push(SourceAssetEntry {
        id: request.asset_id.clone(),
        asset_type: AssetType::Prefab,
        source_path: relative_source_string,
        cook_rules: CookRules::default(),
    });
    manifest
        .assets
        .sort_by(|left, right| left.id.id.cmp(&right.id.id));
    let mut manifest_bytes = serde_json::to_vec_pretty(&manifest).map_err(|error| {
        PrefabAuthoringError::Manifest(format!("could not serialize source manifest: {error}"))
    })?;
    manifest_bytes.push(b'\n');

    if let Some(parent) = source_path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            PrefabAuthoringError::Io(format!("could not create {}: {error}", parent.display()))
        })?;
    }
    commit_new_source_and_manifest(
        &source_path,
        source_text.as_bytes(),
        &manifest_path,
        &manifest_bytes,
    )?;

    Ok(CreatedPrefabAsset {
        asset_id: request.asset_id,
        source_path,
        manifest_path,
        prefab,
    })
}

/// Read and validate one canonical prefab source document.
pub fn load_prefab_source(path: &Path) -> Result<Prefab, PrefabAuthoringError> {
    let bytes = std::fs::read(path).map_err(|error| {
        PrefabAuthoringError::Io(format!("could not read {}: {error}", path.display()))
    })?;
    engine_scene::parse_prefab_source(&bytes).map_err(PrefabAuthoringError::InvalidPrefab)
}

fn reject_external_entity_reference(
    value: &Value,
    captured_ids: &BTreeSet<PersistentId>,
    owner: &str,
) -> Result<(), PrefabAuthoringError> {
    match value {
        Value::Entity(entity_id) if !captured_ids.contains(entity_id) => {
            return Err(PrefabAuthoringError::InvalidPrefab(format!(
                "entity '{owner}' references external entity '{entity_id}'"
            )));
        }
        Value::List(values) => {
            for value in values {
                reject_external_entity_reference(value, captured_ids, owner)?;
            }
        }
        Value::Map(values) => {
            for value in values.values() {
                reject_external_entity_reference(value, captured_ids, owner)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_relative_prefab_path(path: &Path) -> Result<PathBuf, PrefabAuthoringError> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|component| !matches!(component, PathComponent::Normal(_)))
    {
        return Err(PrefabAuthoringError::InvalidRequest(format!(
            "prefab source path must be portable and relative: {}",
            path.display()
        )));
    }
    let portable = portable_path(path)?;
    if !portable
        .to_ascii_lowercase()
        .ends_with(PREFAB_SOURCE_SUFFIX)
    {
        return Err(PrefabAuthoringError::InvalidRequest(format!(
            "prefab source path must end with '{PREFAB_SOURCE_SUFFIX}'"
        )));
    }
    Ok(path.to_path_buf())
}

fn portable_path(path: &Path) -> Result<String, PrefabAuthoringError> {
    let parts = path
        .components()
        .map(|component| match component {
            PathComponent::Normal(value) => value.to_str().map(str::to_owned).ok_or_else(|| {
                PrefabAuthoringError::InvalidRequest("path is not valid UTF-8".into())
            }),
            _ => Err(PrefabAuthoringError::InvalidRequest(
                "path is not portable and relative".into(),
            )),
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(parts.join("/"))
}

fn resolve_manifest_path(
    source_root: &Path,
    requested: &Path,
) -> Result<PathBuf, PrefabAuthoringError> {
    let path = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        source_root.join(requested)
    };
    let file_name = path.file_name().ok_or_else(|| {
        PrefabAuthoringError::InvalidRequest("manifest path has no file name".into())
    })?;
    if path.parent() != Some(source_root)
        || !file_name
            .to_string_lossy()
            .to_ascii_lowercase()
            .ends_with(".manifest")
    {
        return Err(PrefabAuthoringError::InvalidRequest(format!(
            "manifest must be a top-level .manifest file inside {}",
            source_root.display()
        )));
    }
    Ok(path)
}

fn commit_new_source_and_manifest(
    source_path: &Path,
    source_bytes: &[u8],
    manifest_path: &Path,
    manifest_bytes: &[u8],
) -> Result<(), PrefabAuthoringError> {
    let source_temp = write_transaction_temp(source_path, "source", source_bytes)?;
    let manifest_temp = match write_transaction_temp(manifest_path, "manifest", manifest_bytes) {
        Ok(path) => path,
        Err(error) => {
            let _ = std::fs::remove_file(&source_temp);
            return Err(error);
        }
    };
    let manifest_existed = manifest_path.exists();
    let manifest_backup = transaction_sibling(manifest_path, "backup");
    if manifest_existed {
        if let Err(error) = std::fs::rename(manifest_path, &manifest_backup) {
            let _ = std::fs::remove_file(&source_temp);
            let _ = std::fs::remove_file(&manifest_temp);
            return Err(PrefabAuthoringError::Io(format!(
                "could not stage manifest replacement {}: {error}",
                manifest_path.display()
            )));
        }
    }
    if let Err(error) = std::fs::rename(&source_temp, source_path) {
        if manifest_existed {
            let _ = std::fs::rename(&manifest_backup, manifest_path);
        }
        let _ = std::fs::remove_file(&manifest_temp);
        return Err(PrefabAuthoringError::Io(format!(
            "could not install prefab source {}: {error}",
            source_path.display()
        )));
    }
    if let Err(error) = std::fs::rename(&manifest_temp, manifest_path) {
        let _ = std::fs::remove_file(source_path);
        if manifest_existed {
            let _ = std::fs::rename(&manifest_backup, manifest_path);
        }
        return Err(PrefabAuthoringError::Io(format!(
            "could not install source manifest {}: {error}",
            manifest_path.display()
        )));
    }
    if manifest_existed {
        if let Err(error) = std::fs::remove_file(&manifest_backup) {
            let _ = std::fs::remove_file(source_path);
            let _ = std::fs::remove_file(manifest_path);
            let restored = std::fs::rename(&manifest_backup, manifest_path);
            return Err(PrefabAuthoringError::Io(match restored {
                Ok(()) => format!(
                    "could not remove transaction backup {}; changes were rolled back: {error}",
                    manifest_backup.display()
                ),
                Err(restore_error) => format!(
                    "could not remove transaction backup {} ({error}) and rollback failed: {restore_error}",
                    manifest_backup.display()
                ),
            }));
        }
    }
    Ok(())
}

fn write_transaction_temp(
    target: &Path,
    role: &str,
    bytes: &[u8],
) -> Result<PathBuf, PrefabAuthoringError> {
    for _ in 0..32 {
        let path = transaction_sibling(target, role);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                if let Err(error) = file.write_all(bytes).and_then(|()| file.sync_all()) {
                    let _ = std::fs::remove_file(&path);
                    return Err(PrefabAuthoringError::Io(format!(
                        "could not write transaction file {}: {error}",
                        path.display()
                    )));
                }
                return Ok(path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(PrefabAuthoringError::Io(format!(
                    "could not create transaction file {}: {error}",
                    path.display()
                )));
            }
        }
    }
    Err(PrefabAuthoringError::Io(
        "could not allocate a unique prefab transaction file".into(),
    ))
}

fn transaction_sibling(target: &Path, role: &str) -> PathBuf {
    let counter = PREFAB_TRANSACTION_COUNTER.fetch_add(1, Ordering::Relaxed);
    let name = target.file_name().unwrap_or_default().to_string_lossy();
    target.with_file_name(format!(
        ".{name}.prefab-txn-{}-{counter}.{role}",
        std::process::id()
    ))
}

fn reject_symlink_ancestors(
    source_root: &Path,
    relative_source: &Path,
) -> Result<(), PrefabAuthoringError> {
    let root_metadata = std::fs::symlink_metadata(source_root).map_err(|error| {
        PrefabAuthoringError::Io(format!(
            "could not inspect source root {}: {error}",
            source_root.display()
        ))
    })?;
    if root_metadata.file_type().is_symlink() {
        return Err(PrefabAuthoringError::InvalidRequest(format!(
            "source root cannot be a symbolic link: {}",
            source_root.display()
        )));
    }
    let mut cursor = source_root.to_path_buf();
    if let Some(parent) = relative_source.parent() {
        for component in parent.components() {
            let PathComponent::Normal(component) = component else {
                return Err(PrefabAuthoringError::InvalidRequest(
                    "prefab source parent is not portable".into(),
                ));
            };
            cursor.push(component);
            match std::fs::symlink_metadata(&cursor) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(PrefabAuthoringError::InvalidRequest(format!(
                        "prefab source path crosses symbolic link {}",
                        cursor.display()
                    )));
                }
                Ok(metadata) if !metadata.is_dir() => {
                    return Err(PrefabAuthoringError::InvalidRequest(format!(
                        "prefab source parent is not a directory: {}",
                        cursor.display()
                    )));
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(PrefabAuthoringError::Io(format!(
                        "could not inspect {}: {error}",
                        cursor.display()
                    )));
                }
            }
        }
    }
    Ok(())
}
