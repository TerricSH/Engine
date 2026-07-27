//! Transactional project asset operations used by the editor's Project panel.
//!
//! Every operation prepares and cooks against a private copy of the project's
//! source tree. The live project is changed only after validation succeeds, and
//! every live file touched by a commit is snapshotted so an I/O failure can be
//! rolled back without leaving a manifest/source/cooked split brain.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use engine_asset::cook::logic_asset::{LogicCondition, LogicValue};
use engine_asset::cook::manifest::CURRENT_MANIFEST_VERSION;
use engine_asset::cook::{
    cook_orchestrate_checked_with_registry, read_cooked_artifact, AssetType, CookRules,
    DependencyGraph, LogicAsset, MaterialSource, SourceAssetEntry, SourceManifest,
    MATERIAL_SOURCE_SCHEMA,
};
use engine_asset::project::GameProject;
use engine_serialize::{AssetId, DiagnosticSeverity, Value};
use serde::{Deserialize, Serialize};

const TRASH_SCHEMA: &str = "EditorAssetTrash-v0";
static ASSET_OPERATION_MUTEX: Mutex<()> = Mutex::new(());

/// Values used to create a portable metallic-roughness material source asset.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MaterialTemplate {
    pub base_color: [f32; 4],
    pub metallic: f32,
    pub roughness: f32,
    pub ambient_occlusion: f32,
    pub emissive: [f32; 3],
    pub base_color_texture: Option<String>,
    pub normal_texture: Option<String>,
    pub metallic_roughness_texture: Option<String>,
    pub occlusion_texture: Option<String>,
    pub emissive_texture: Option<String>,
    pub transparency: String,
    pub alpha_cutoff: f32,
    pub double_sided: bool,
}

impl Default for MaterialTemplate {
    fn default() -> Self {
        Self {
            base_color: [1.0, 1.0, 1.0, 1.0],
            metallic: 0.0,
            roughness: 0.5,
            ambient_occlusion: 1.0,
            emissive: [0.0; 3],
            base_color_texture: None,
            normal_texture: None,
            metallic_roughness_texture: None,
            occlusion_texture: None,
            emissive_texture: None,
            transparency: "Opaque".into(),
            alpha_cutoff: 0.5,
            double_sided: false,
        }
    }
}

/// Paths and stable identity produced by a successful create/copy/move.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AssetMutation {
    pub asset_id: AssetId,
    pub manifest_path: PathBuf,
    pub source_path: PathBuf,
    pub cooked_path: PathBuf,
}

/// Recoverable location produced by a successful delete.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DeletedAsset {
    pub asset_id: AssetId,
    pub trash_directory: PathBuf,
    pub metadata_path: PathBuf,
}

/// Create exactly one new folder below `assets/source`.
///
/// The parent must already exist. Requiring that makes directory creation a
/// single atomic filesystem operation instead of a partially-created chain.
pub(crate) fn create_asset_folder(
    project_path: &Path,
    relative_folder: &Path,
) -> Result<PathBuf, String> {
    let _operation_guard = lock_asset_operations()?;
    let project = load_project(project_path)?;
    let relative = normalize_relative_path(relative_folder, "asset folder", false)?;
    let target = project.asset_source.join(&relative);
    ensure_parent_is_real_directory(&project.asset_source, &relative)?;
    ensure_destination_absent(&project.asset_source, &relative)?;
    std::fs::create_dir(&target).map_err(|error| {
        format!(
            "could not create asset folder {}: {error}",
            target.display()
        )
    })?;
    Ok(target)
}

/// Rename one folder below `assets/source` while preserving every declared
/// asset ID and updating source-manifest paths in the same operation.
///
/// Moving a folder to another parent is deliberately rejected. Authoring
/// formats such as glTF may contain relative sidecar references whose meaning
/// changes when their directory depth changes; the Project panel currently
/// exposes this operation as Rename, not Move.
pub(crate) fn rename_asset_folder(
    project_path: &Path,
    relative_folder: &Path,
    new_relative_folder: &Path,
) -> Result<PathBuf, String> {
    let _operation_guard = lock_asset_operations()?;
    let project = load_project(project_path)?;
    let relative = normalize_relative_path(relative_folder, "asset folder", false)?;
    let new_relative = normalize_relative_path(new_relative_folder, "new asset folder", false)?;
    if portable_path_key(&relative) == portable_path_key(&new_relative) {
        return Err(
            "asset folder already has that portable path; case-only renames are not portable"
                .into(),
        );
    }
    let old_parent = relative.parent().unwrap_or_else(|| Path::new(""));
    let new_parent = new_relative.parent().unwrap_or_else(|| Path::new(""));
    if portable_path_key(old_parent) != portable_path_key(new_parent) {
        return Err(
            "asset folders can only be renamed within their current parent; cross-folder moves could invalidate relative sidecar references"
                .into(),
        );
    }
    let relative_key = portable_path_key(&relative);
    let new_relative_key = portable_path_key(&new_relative);
    if new_relative_key.starts_with(&format!("{relative_key}/")) {
        return Err("an asset folder cannot be moved inside itself".into());
    }
    ensure_existing_folder(&project.asset_source, &relative)?;
    ensure_parent_is_real_directory(&project.asset_source, &new_relative)?;
    ensure_destination_absent(&project.asset_source, &new_relative)?;

    let old_path = project.asset_source.join(&relative);
    let new_path = project.asset_source.join(&new_relative);
    let mut catalog = ManifestCatalog::load(&project.asset_source)?;
    let mut manifest_backups = Vec::new();
    for document in &mut catalog.documents {
        let mut changed = false;
        for entry in &mut document.manifest.assets {
            let source = normalize_relative_path(
                Path::new(&entry.source_path),
                "manifest source path",
                false,
            )?;
            let Ok(suffix) = source.strip_prefix(&relative) else {
                continue;
            };
            entry.source_path = manifest_path_string(&new_relative.join(suffix))?;
            changed = true;
        }
        if changed {
            let updated_path = document
                .path
                .strip_prefix(&old_path)
                .map(|suffix| new_path.join(suffix))
                .unwrap_or_else(|_| document.path.clone());
            manifest_backups.push((
                updated_path,
                std::fs::read(&document.path).map_err(io_read(&document.path))?,
                serialize_manifest(&document.manifest)?,
            ));
        }
    }

    std::fs::rename(&old_path, &new_path).map_err(|error| {
        format!(
            "could not rename asset folder {} -> {}: {error}",
            old_path.display(),
            new_path.display()
        )
    })?;
    let update_result = (|| {
        for (path, _, updated) in &manifest_backups {
            super::project_cli::atomic_write_bytes(path, updated)?;
        }
        ManifestCatalog::load(&project.asset_source)?;
        Ok(())
    })();
    if let Err(error) = update_result {
        let mut rollback_errors = Vec::new();
        for (path, original, _) in manifest_backups.iter().rev() {
            if let Err(rollback_error) = super::project_cli::atomic_write_bytes(path, original) {
                rollback_errors.push(rollback_error);
            }
        }
        if let Err(rollback_error) = std::fs::rename(&new_path, &old_path) {
            rollback_errors.push(format!(
                "could not restore asset folder {} -> {}: {rollback_error}",
                new_path.display(),
                old_path.display()
            ));
        }
        return if rollback_errors.is_empty() {
            Err(error)
        } else {
            Err(format!(
                "{error}\nasset folder rollback also failed:\n{}",
                rollback_errors.join("\n")
            ))
        };
    }
    Ok(new_path)
}

/// Delete one empty folder below `assets/source`.
///
/// Recursive deletion is deliberately rejected: assets must first go through
/// the dependency-aware asset delete transaction, so folder deletion cannot
/// become a hidden bulk-delete path.
pub(crate) fn delete_asset_folder(
    project_path: &Path,
    relative_folder: &Path,
) -> Result<(), String> {
    let _operation_guard = lock_asset_operations()?;
    let project = load_project(project_path)?;
    let relative = normalize_relative_path(relative_folder, "asset folder", false)?;
    ensure_existing_folder(&project.asset_source, &relative)?;
    let folder = project.asset_source.join(&relative);
    let mut entries = std::fs::read_dir(&folder).map_err(|error| {
        format!(
            "could not enumerate asset folder {}: {error}",
            folder.display()
        )
    })?;
    if entries
        .next()
        .transpose()
        .map_err(|error| {
            format!(
                "could not enumerate asset folder {}: {error}",
                folder.display()
            )
        })?
        .is_some()
    {
        return Err(format!(
            "asset folder is not empty: {}; delete or move its assets and subfolders first",
            folder.display()
        ));
    }
    std::fs::remove_dir(&folder).map_err(|error| {
        format!(
            "could not delete asset folder {}: {error}",
            folder.display()
        )
    })
}

/// Create, validate, cook, and install a material from the editor template.
///
/// `folder` is relative to `assets/source`; an empty path means its root.
/// The returned `AssetId` is generated once, recorded in the source manifest,
/// and remains stable if the source is later renamed or moved.
pub(crate) fn create_material_asset(
    project_path: &Path,
    folder: &Path,
    requested_name: &str,
    template: &MaterialTemplate,
) -> Result<AssetMutation, String> {
    create_material_asset_impl(project_path, folder, requested_name, template, None)
}

/// Duplicate a declared asset and generate a new stable `AssetId` and source
/// name beside the original. Both names receive deterministic `-copy`,
/// `-copy-2`, ... suffixes selected from the current manifest state.
pub(crate) fn duplicate_project_asset(
    project_path: &Path,
    source_asset_id: &AssetId,
) -> Result<AssetMutation, String> {
    duplicate_project_asset_impl(project_path, source_asset_id, None)
}

/// Rename or move an asset's authoring file while preserving its stable ID.
/// The owning source manifest is updated and the cooked artifact is rebuilt
/// and atomically replaced before the old source file is removed.
pub(crate) fn move_project_asset(
    project_path: &Path,
    asset_id: &AssetId,
    new_relative_source_path: &Path,
) -> Result<AssetMutation, String> {
    move_project_asset_impl(project_path, asset_id, new_relative_source_path, None)
}

/// Move an asset, its cooked payload, and recovery metadata into the project's
/// `.engine/trash/assets` area, then remove its source-manifest declaration.
/// Assets referenced by a cataloged project scene or another material are not
/// deleted because doing so would knowingly create a broken project.
pub(crate) fn delete_project_asset(
    project_path: &Path,
    asset_id: &AssetId,
) -> Result<DeletedAsset, String> {
    delete_project_asset_impl(project_path, asset_id, None)
}

fn create_material_asset_impl(
    project_path: &Path,
    folder: &Path,
    requested_name: &str,
    template: &MaterialTemplate,
    fail_after_mutation: Option<usize>,
) -> Result<AssetMutation, String> {
    let _operation_guard = lock_asset_operations()?;
    let project = load_project(project_path)?;
    let folder = normalize_relative_path(folder, "material folder", true)?;
    ensure_existing_folder(&project.asset_source, &folder)?;
    let live_catalog = ManifestCatalog::load(&project.asset_source)?;
    let slug = portable_slug(requested_name)?;
    let (asset_id, relative_source) =
        next_material_identity(&project.asset_source, &live_catalog, &folder, &slug)?;

    let staging = StagedWorkspace::prepare(&project)?;
    let staged_source = staging.source_root.join(&relative_source);
    let material = MaterialSource {
        schema: MATERIAL_SOURCE_SCHEMA.into(),
        base_color: template.base_color,
        metallic: template.metallic,
        roughness: template.roughness,
        ambient_occlusion: template.ambient_occlusion,
        emissive: template.emissive,
        base_color_texture: template.base_color_texture.clone(),
        normal_texture: template.normal_texture.clone(),
        metallic_roughness_texture: template.metallic_roughness_texture.clone(),
        occlusion_texture: template.occlusion_texture.clone(),
        emissive_texture: template.emissive_texture.clone(),
        advanced: Default::default(),
        transparency: template.transparency.clone(),
        alpha_cutoff: template.alpha_cutoff,
        double_sided: template.double_sided,
    };
    let mut source_json = serde_json::to_string_pretty(&material)
        .map_err(|error| format!("could not serialize material template: {error}"))?;
    source_json.push('\n');
    super::project_cli::atomic_write_bytes(&staged_source, source_json.as_bytes())?;

    let mut staged_catalog = ManifestCatalog::load(&staging.source_root)?;
    let manifest_index = staged_catalog.ensure_game_manifest(&staging.source_root)?;
    staged_catalog.documents[manifest_index]
        .manifest
        .assets
        .push(SourceAssetEntry {
            id: asset_id.clone(),
            asset_type: AssetType::Material,
            source_path: manifest_path_string(&relative_source)?,
            cook_rules: CookRules::default(),
        });
    staged_catalog.sort_document(manifest_index);
    staged_catalog.write_document(manifest_index)?;

    let staged_cooked = cook_staged_asset(&project, &staging, &asset_id, &AssetType::Material)?;
    let staged_manifest = staged_catalog.documents[manifest_index].path.clone();
    let live_manifest = map_staged_path(
        &staging.source_root,
        &project.asset_source,
        &staged_manifest,
    )?;
    let live_source = project.asset_source.join(&relative_source);
    let live_cooked = cooked_path(&project, &asset_id);
    let writes = vec![
        CommitWrite::create(
            live_source.clone(),
            std::fs::read(&staged_source).map_err(io_read(&staged_source))?,
        ),
        CommitWrite::create(live_cooked.clone(), staged_cooked),
        CommitWrite::for_current_state(
            live_manifest.clone(),
            std::fs::read(&staged_manifest).map_err(io_read(&staged_manifest))?,
        ),
    ];
    commit_transaction(&project.root, writes, Vec::new(), fail_after_mutation)?;
    Ok(AssetMutation {
        asset_id,
        manifest_path: live_manifest,
        source_path: live_source,
        cooked_path: live_cooked,
    })
}

fn duplicate_project_asset_impl(
    project_path: &Path,
    source_asset_id: &AssetId,
    fail_after_mutation: Option<usize>,
) -> Result<AssetMutation, String> {
    let _operation_guard = lock_asset_operations()?;
    let project = load_project(project_path)?;
    let live_catalog = ManifestCatalog::load(&project.asset_source)?;
    let (document_index, asset_index) = live_catalog.locate(source_asset_id)?;
    let source_entry = live_catalog.documents[document_index].manifest.assets[asset_index].clone();
    let source_relative = normalize_relative_path(
        Path::new(&source_entry.source_path),
        "manifest source path",
        false,
    )?;
    let (new_id, new_relative) = next_duplicate_identity(
        &project.asset_source,
        &live_catalog,
        &source_entry.id,
        &source_relative,
    )?;

    let staging = StagedWorkspace::prepare(&project)?;
    let old_staged_source = staging.source_root.join(&source_relative);
    let new_staged_source = staging.source_root.join(&new_relative);
    copy_file_create_new(&old_staged_source, &new_staged_source)?;
    rewrite_duplicated_source_identity(&source_entry.asset_type, &new_staged_source, &new_id)?;

    let mut staged_catalog = ManifestCatalog::load(&staging.source_root)?;
    let (staged_document, staged_asset) = staged_catalog.locate(&source_entry.id)?;
    let mut new_entry =
        staged_catalog.documents[staged_document].manifest.assets[staged_asset].clone();
    new_entry.id = new_id.clone();
    new_entry.id.logical_path = None;
    new_entry.source_path = manifest_path_string(&new_relative)?;
    staged_catalog.documents[staged_document]
        .manifest
        .assets
        .push(new_entry);
    staged_catalog.sort_document(staged_document);
    staged_catalog.write_document(staged_document)?;

    let staged_cooked = cook_staged_asset(&project, &staging, &new_id, &source_entry.asset_type)?;
    let staged_manifest = staged_catalog.documents[staged_document].path.clone();
    let live_manifest = map_staged_path(
        &staging.source_root,
        &project.asset_source,
        &staged_manifest,
    )?;
    let live_source = project.asset_source.join(&new_relative);
    let live_cooked = cooked_path(&project, &new_id);
    let writes = vec![
        CommitWrite::create(
            live_source.clone(),
            std::fs::read(&new_staged_source).map_err(io_read(&new_staged_source))?,
        ),
        CommitWrite::create(live_cooked.clone(), staged_cooked),
        CommitWrite::replace(
            live_manifest.clone(),
            std::fs::read(&staged_manifest).map_err(io_read(&staged_manifest))?,
        ),
    ];
    commit_transaction(&project.root, writes, Vec::new(), fail_after_mutation)?;
    Ok(AssetMutation {
        asset_id: new_id,
        manifest_path: live_manifest,
        source_path: live_source,
        cooked_path: live_cooked,
    })
}

fn move_project_asset_impl(
    project_path: &Path,
    asset_id: &AssetId,
    new_relative_source_path: &Path,
    fail_after_mutation: Option<usize>,
) -> Result<AssetMutation, String> {
    let _operation_guard = lock_asset_operations()?;
    let project = load_project(project_path)?;
    let new_relative =
        normalize_relative_path(new_relative_source_path, "new asset source path", false)?;
    ensure_parent_is_real_directory(&project.asset_source, &new_relative)?;
    let live_catalog = ManifestCatalog::load(&project.asset_source)?;
    let (document_index, asset_index) = live_catalog.locate(asset_id)?;
    let entry = live_catalog.documents[document_index].manifest.assets[asset_index].clone();
    let old_relative =
        normalize_relative_path(Path::new(&entry.source_path), "manifest source path", false)?;
    if portable_path_key(&old_relative) == portable_path_key(&new_relative) {
        return Err(
            "asset source already has that portable path; case-only renames are not portable"
                .into(),
        );
    }
    ensure_destination_absent(&project.asset_source, &new_relative)?;
    if live_catalog.source_path_is_declared(&new_relative) {
        return Err(format!(
            "another source-manifest entry already uses '{}'",
            new_relative.display()
        ));
    }

    let staging = StagedWorkspace::prepare(&project)?;
    let old_staged_source = staging.source_root.join(&old_relative);
    let new_staged_source = staging.source_root.join(&new_relative);
    // Load the staged catalog while it still describes a self-consistent tree.
    // After the physical rename the old manifest path is intentionally absent
    // until the in-memory entry below is updated and written.
    let mut staged_catalog = ManifestCatalog::load(&staging.source_root)?;
    let (staged_document, staged_asset) = staged_catalog.locate(asset_id)?;
    std::fs::rename(&old_staged_source, &new_staged_source).map_err(|error| {
        format!(
            "could not stage asset move {} -> {}: {error}",
            old_staged_source.display(),
            new_staged_source.display()
        )
    })?;
    staged_catalog.documents[staged_document].manifest.assets[staged_asset].source_path =
        manifest_path_string(&new_relative)?;
    staged_catalog.sort_document(staged_document);
    staged_catalog.write_document(staged_document)?;

    let staged_cooked = cook_staged_asset(&project, &staging, asset_id, &entry.asset_type)?;
    let staged_manifest = staged_catalog.documents[staged_document].path.clone();
    let live_manifest = map_staged_path(
        &staging.source_root,
        &project.asset_source,
        &staged_manifest,
    )?;
    let live_source = project.asset_source.join(&new_relative);
    let old_live_source = project.asset_source.join(&old_relative);
    let live_cooked = cooked_path(&project, asset_id);
    let writes = vec![
        CommitWrite::create(
            live_source.clone(),
            std::fs::read(&new_staged_source).map_err(io_read(&new_staged_source))?,
        ),
        CommitWrite::for_current_state(live_cooked.clone(), staged_cooked),
        CommitWrite::replace(
            live_manifest.clone(),
            std::fs::read(&staged_manifest).map_err(io_read(&staged_manifest))?,
        ),
    ];
    commit_transaction(
        &project.root,
        writes,
        vec![old_live_source],
        fail_after_mutation,
    )?;
    Ok(AssetMutation {
        asset_id: entry.id,
        manifest_path: live_manifest,
        source_path: live_source,
        cooked_path: live_cooked,
    })
}

fn delete_project_asset_impl(
    project_path: &Path,
    asset_id: &AssetId,
    fail_after_mutation: Option<usize>,
) -> Result<DeletedAsset, String> {
    let _operation_guard = lock_asset_operations()?;
    let project = load_project(project_path)?;
    let live_catalog = ManifestCatalog::load(&project.asset_source)?;
    let (document_index, asset_index) = live_catalog.locate(asset_id)?;
    let entry = live_catalog.documents[document_index].manifest.assets[asset_index].clone();
    reject_known_references(&project, &live_catalog, &entry)?;
    let relative_source =
        normalize_relative_path(Path::new(&entry.source_path), "manifest source path", false)?;
    let live_source = project.asset_source.join(&relative_source);
    let live_cooked = cooked_path(&project, &entry.id);
    let live_manifest = live_catalog.documents[document_index].path.clone();

    let mut updated_manifest = live_catalog.documents[document_index].manifest.clone();
    updated_manifest.assets.remove(asset_index);
    let manifest_bytes = serialize_manifest(&updated_manifest)?;
    let trash_directory = allocate_trash_directory(&project, &entry.id)?;
    let trash_source = trash_directory.join("source").join(&relative_source);
    let trash_cooked = trash_directory
        .join("cooked")
        .join(format!("{}.cooked", entry.id.id));
    let metadata_path = trash_directory.join("trash-entry.json");
    let metadata = TrashMetadata {
        schema: TRASH_SCHEMA.into(),
        deleted_unix_nanos: unix_nanos(),
        manifest_path: project_relative_string(&project, &live_manifest)?,
        source_path: manifest_path_string(&relative_source)?,
        cooked_path: project_relative_string(&project, &live_cooked)?,
        entry: entry.clone(),
    };
    let mut metadata_bytes = serde_json::to_vec_pretty(&metadata)
        .map_err(|error| format!("could not serialize asset trash metadata: {error}"))?;
    metadata_bytes.push(b'\n');

    let mut writes = vec![
        CommitWrite::create(
            trash_source,
            std::fs::read(&live_source).map_err(io_read(&live_source))?,
        ),
        CommitWrite::create(metadata_path.clone(), metadata_bytes),
        CommitWrite::replace(live_manifest, manifest_bytes),
    ];
    let mut deletes = vec![live_source];
    if live_cooked.is_file() {
        writes.insert(
            1,
            CommitWrite::create(
                trash_cooked,
                std::fs::read(&live_cooked).map_err(io_read(&live_cooked))?,
            ),
        );
        deletes.push(live_cooked);
    } else if live_cooked.exists() {
        return Err(format!(
            "cooked asset path is not a regular file: {}",
            live_cooked.display()
        ));
    }
    commit_transaction(&project.root, writes, deletes, fail_after_mutation)?;
    Ok(DeletedAsset {
        asset_id: entry.id,
        trash_directory,
        metadata_path,
    })
}

#[derive(Clone, Debug)]
struct ManifestDocument {
    path: PathBuf,
    manifest: SourceManifest,
}

#[derive(Clone, Debug, Default)]
struct ManifestCatalog {
    documents: Vec<ManifestDocument>,
}

impl ManifestCatalog {
    fn load(source_root: &Path) -> Result<Self, String> {
        if !source_root.is_dir() {
            return Err(format!(
                "asset source root is not a directory: {}",
                source_root.display()
            ));
        }
        let mut paths = Vec::new();
        for entry in std::fs::read_dir(source_root)
            .map_err(|error| format!("could not enumerate {}: {error}", source_root.display()))?
        {
            let path = entry
                .map_err(|error| format!("could not enumerate {}: {error}", source_root.display()))?
                .path();
            if path.is_file()
                && path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("manifest"))
            {
                paths.push(path);
            }
        }
        paths.sort();
        let mut documents = Vec::new();
        let mut ids = BTreeSet::new();
        let mut source_paths = BTreeMap::<String, String>::new();
        for path in paths {
            reject_symlink(&path)?;
            let bytes = std::fs::read(&path).map_err(io_read(&path))?;
            let manifest: SourceManifest = serde_json::from_slice(&bytes)
                .map_err(|error| format!("invalid source manifest {}: {error}", path.display()))?;
            if manifest.schema_version != CURRENT_MANIFEST_VERSION {
                return Err(format!(
                    "unsupported source manifest schema in {}",
                    path.display()
                ));
            }
            for asset in &manifest.assets {
                validate_asset_id(&asset.id)?;
                let id_key = asset.id.id.to_ascii_lowercase();
                if !ids.insert(id_key) {
                    return Err(format!(
                        "asset id '{}' is duplicated or differs only by case",
                        asset.id.id
                    ));
                }
                let relative = normalize_relative_path(
                    Path::new(&asset.source_path),
                    "manifest source path",
                    false,
                )?;
                let source_key = portable_path_key(&relative);
                if let Some(previous) = source_paths.insert(source_key, asset.id.id.clone()) {
                    return Err(format!(
                        "assets '{}' and '{}' share the same portable source path",
                        previous, asset.id.id
                    ));
                }
                let source = source_root.join(&relative);
                if !source.is_file() {
                    return Err(format!("source asset is missing: {}", source.display()));
                }
                reject_symlink(&source)?;
            }
            documents.push(ManifestDocument { path, manifest });
        }
        Ok(Self { documents })
    }

    fn locate(&self, asset_id: &AssetId) -> Result<(usize, usize), String> {
        validate_asset_id(asset_id)?;
        let mut match_location = None;
        for (document_index, document) in self.documents.iter().enumerate() {
            for (asset_index, entry) in document.manifest.assets.iter().enumerate() {
                if entry.id.id.eq_ignore_ascii_case(&asset_id.id) {
                    if entry.id.id != asset_id.id {
                        return Err(format!(
                            "asset id '{}' differs in case from declared id '{}'",
                            asset_id.id, entry.id.id
                        ));
                    }
                    match_location = Some((document_index, asset_index));
                }
            }
        }
        match_location.ok_or_else(|| format!("asset '{}' is not declared", asset_id.id))
    }

    fn contains_id(&self, id: &str) -> bool {
        self.documents.iter().any(|document| {
            document
                .manifest
                .assets
                .iter()
                .any(|entry| entry.id.id.eq_ignore_ascii_case(id))
        })
    }

    fn source_path_is_declared(&self, relative: &Path) -> bool {
        let key = portable_path_key(relative);
        self.documents.iter().any(|document| {
            document.manifest.assets.iter().any(|entry| {
                normalize_relative_path(Path::new(&entry.source_path), "source path", false)
                    .is_ok_and(|path| portable_path_key(&path) == key)
            })
        })
    }

    fn ensure_game_manifest(&mut self, source_root: &Path) -> Result<usize, String> {
        let matches = self
            .documents
            .iter()
            .enumerate()
            .filter(|(_, document)| {
                document
                    .path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.eq_ignore_ascii_case("game.manifest"))
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [index] => Ok(*index),
            [] => {
                let path = source_root.join("game.manifest");
                if path.exists() {
                    return Err(format!(
                        "game.manifest exists but is not a regular source manifest: {}",
                        path.display()
                    ));
                }
                self.documents.push(ManifestDocument {
                    path,
                    manifest: SourceManifest {
                        schema_version: CURRENT_MANIFEST_VERSION,
                        assets: Vec::new(),
                    },
                });
                Ok(self.documents.len() - 1)
            }
            _ => Err("multiple source manifests differ only by the name game.manifest".into()),
        }
    }

    fn sort_document(&mut self, index: usize) {
        self.documents[index]
            .manifest
            .assets
            .sort_by(|left, right| {
                left.id
                    .id
                    .to_ascii_lowercase()
                    .cmp(&right.id.id.to_ascii_lowercase())
                    .then_with(|| left.id.id.cmp(&right.id.id))
            });
    }

    fn write_document(&self, index: usize) -> Result<(), String> {
        let document = &self.documents[index];
        super::project_cli::atomic_write_bytes(
            &document.path,
            &serialize_manifest(&document.manifest)?,
        )
    }
}

struct StagedWorkspace {
    _directory: tempfile::TempDir,
    source_root: PathBuf,
    cooked_root: PathBuf,
}

impl StagedWorkspace {
    fn prepare(project: &GameProject) -> Result<Self, String> {
        let transaction_root = project.root.join(".engine/asset-transactions");
        ensure_no_symlink_ancestors(&project.root, &transaction_root)?;
        std::fs::create_dir_all(&transaction_root).map_err(|error| {
            format!(
                "could not create asset transaction directory {}: {error}",
                transaction_root.display()
            )
        })?;
        let directory = tempfile::Builder::new()
            .prefix("asset-op-")
            .tempdir_in(&transaction_root)
            .map_err(|error| {
                format!(
                    "could not create asset transaction workspace in {}: {error}",
                    transaction_root.display()
                )
            })?;
        let source_root = directory.path().join("source");
        let cooked_root = directory.path().join("cooked");
        copy_directory_tree(&project.asset_source, &source_root)?;
        std::fs::create_dir(&cooked_root).map_err(|error| {
            format!(
                "could not create staged cooked directory {}: {error}",
                cooked_root.display()
            )
        })?;
        Ok(Self {
            _directory: directory,
            source_root,
            cooked_root,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WriteMode {
    Create,
    Replace,
    CurrentState,
}

struct CommitWrite {
    path: PathBuf,
    bytes: Vec<u8>,
    mode: WriteMode,
}

impl CommitWrite {
    fn create(path: PathBuf, bytes: Vec<u8>) -> Self {
        Self {
            path,
            bytes,
            mode: WriteMode::Create,
        }
    }

    fn replace(path: PathBuf, bytes: Vec<u8>) -> Self {
        Self {
            path,
            bytes,
            mode: WriteMode::Replace,
        }
    }

    fn for_current_state(path: PathBuf, bytes: Vec<u8>) -> Self {
        Self {
            path,
            bytes,
            mode: WriteMode::CurrentState,
        }
    }
}

struct FileSnapshot {
    path: PathBuf,
    bytes: Option<Vec<u8>>,
}

fn commit_transaction(
    project_root: &Path,
    writes: Vec<CommitWrite>,
    deletes: Vec<PathBuf>,
    fail_after_mutation: Option<usize>,
) -> Result<(), String> {
    let mut unique_paths = BTreeSet::new();
    let mut snapshots = Vec::new();
    for path in writes
        .iter()
        .map(|write| write.path.as_path())
        .chain(deletes.iter().map(PathBuf::as_path))
    {
        if !path.starts_with(project_root) {
            return Err(format!(
                "asset transaction target escapes project root: {}",
                path.display()
            ));
        }
        ensure_no_symlink_ancestors(project_root, path)?;
        if unique_paths.insert(path.to_path_buf()) {
            let bytes = if path.is_file() {
                Some(std::fs::read(path).map_err(io_read(path))?)
            } else if path.exists() {
                return Err(format!(
                    "asset transaction target is not a regular file: {}",
                    path.display()
                ));
            } else {
                None
            };
            snapshots.push(FileSnapshot {
                path: path.to_path_buf(),
                bytes,
            });
        }
    }
    for write in &writes {
        let existed = snapshots
            .iter()
            .find(|snapshot| snapshot.path == write.path)
            .and_then(|snapshot| snapshot.bytes.as_ref())
            .is_some();
        match write.mode {
            WriteMode::Create if existed => {
                return Err(format!(
                    "asset transaction will not overwrite existing file: {}",
                    write.path.display()
                ));
            }
            WriteMode::Replace if !existed => {
                return Err(format!(
                    "asset transaction expected an existing file: {}",
                    write.path.display()
                ));
            }
            _ => {}
        }
    }
    for delete in &deletes {
        if snapshots
            .iter()
            .find(|snapshot| snapshot.path == *delete)
            .and_then(|snapshot| snapshot.bytes.as_ref())
            .is_none()
        {
            return Err(format!(
                "asset transaction cannot delete missing file: {}",
                delete.display()
            ));
        }
    }

    let mut mutations = 0usize;
    let result = (|| {
        for write in &writes {
            if write.mode == WriteMode::Create {
                write_file_create_new(&write.path, &write.bytes)?;
            } else {
                super::project_cli::atomic_write_bytes(&write.path, &write.bytes)?;
            }
            mutations += 1;
            maybe_inject_commit_failure(fail_after_mutation, mutations)?;
        }
        for delete in &deletes {
            std::fs::remove_file(delete)
                .map_err(|error| format!("could not remove {}: {error}", delete.display()))?;
            mutations += 1;
            maybe_inject_commit_failure(fail_after_mutation, mutations)?;
        }
        Ok(())
    })();
    if let Err(failure) = result {
        let rollback_errors = restore_snapshots(&snapshots);
        remove_empty_created_parents(project_root, &snapshots);
        return if rollback_errors.is_empty() {
            Err(failure)
        } else {
            Err(format!(
                "{failure}\nasset transaction rollback also failed:\n{}",
                rollback_errors.join("\n")
            ))
        };
    }
    Ok(())
}

fn restore_snapshots(snapshots: &[FileSnapshot]) -> Vec<String> {
    let mut errors = Vec::new();
    for snapshot in snapshots.iter().rev() {
        let restored = match &snapshot.bytes {
            Some(bytes) => super::project_cli::atomic_write_bytes(&snapshot.path, bytes),
            None if snapshot.path.is_file() => std::fs::remove_file(&snapshot.path)
                .map_err(|error| format!("could not remove {}: {error}", snapshot.path.display())),
            None if snapshot.path.exists() => Err(format!(
                "rollback target became a non-file: {}",
                snapshot.path.display()
            )),
            None => Ok(()),
        };
        if let Err(error) = restored {
            errors.push(error);
        }
    }
    errors
}

fn maybe_inject_commit_failure(
    fail_after_mutation: Option<usize>,
    mutations: usize,
) -> Result<(), String> {
    if fail_after_mutation == Some(mutations) {
        Err(format!(
            "simulated asset transaction failure after mutation {mutations}"
        ))
    } else {
        Ok(())
    }
}

fn remove_empty_created_parents(project_root: &Path, snapshots: &[FileSnapshot]) {
    let mut directories = snapshots
        .iter()
        .filter(|snapshot| snapshot.bytes.is_none())
        .filter_map(|snapshot| snapshot.path.parent().map(Path::to_path_buf))
        .collect::<Vec<_>>();
    directories.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    directories.dedup();
    for mut directory in directories {
        while directory.starts_with(project_root) && directory != project_root {
            if std::fs::remove_dir(&directory).is_err() {
                break;
            }
            let Some(parent) = directory.parent() else {
                break;
            };
            directory = parent.to_path_buf();
        }
    }
}

fn cook_staged_asset(
    project: &GameProject,
    staging: &StagedWorkspace,
    asset_id: &AssetId,
    expected_type: &AssetType,
) -> Result<Vec<u8>, String> {
    let mut graph = DependencyGraph::new();
    let runtime_builder = engine_core::EngineRuntime::builder(engine_core::EngineConfig {
        application_name: format!("{}-editor-asset-op", project.manifest.name),
        gpu_timestamps: true,
    });
    let report = cook_orchestrate_checked_with_registry(
        &staging.source_root,
        &staging.cooked_root,
        &mut graph,
        runtime_builder.asset_type_registry(),
    );
    if !report.is_success() {
        return Err(cook_failure(&report));
    }
    if !report.results.iter().any(|result| {
        result.success && result.asset_id == asset_id.id && result.asset_type == *expected_type
    }) {
        return Err(format!(
            "asset cook succeeded without the expected {:?} result for '{}'",
            expected_type, asset_id.id
        ));
    }
    let output = staging.cooked_root.join(format!("{}.cooked", asset_id.id));
    let artifact = read_cooked_artifact(&output).map_err(|error| {
        format!(
            "staged cooked artifact is invalid {}: {error}",
            output.display()
        )
    })?;
    if artifact.header.asset_kind != expected_type.kind_code() {
        return Err(format!(
            "staged asset '{}' cooked as kind {}, expected {}",
            asset_id.id,
            artifact.header.asset_kind,
            expected_type.kind_code()
        ));
    }
    std::fs::read(&output).map_err(io_read(&output))
}

fn cook_failure(report: &engine_asset::cook::CookReport) -> String {
    let mut details = report
        .diagnostics
        .iter()
        .filter(|diagnostic| {
            matches!(
                diagnostic.severity,
                DiagnosticSeverity::Error | DiagnosticSeverity::Fatal
            )
        })
        .map(|diagnostic| format!("{}: {}", diagnostic.code, diagnostic.message))
        .collect::<Vec<_>>();
    details.extend(
        report
            .results
            .iter()
            .filter(|result| !result.success)
            .flat_map(|result| {
                result
                    .diagnostics
                    .iter()
                    .map(|diagnostic| format!("{}: {}", diagnostic.code, diagnostic.message))
            }),
    );
    details.sort();
    details.dedup();
    details.truncate(12);
    if details.is_empty() {
        "project asset cooking failed without a diagnostic".into()
    } else {
        format!("project asset cooking failed:\n{}", details.join("\n"))
    }
}

fn next_material_identity(
    source_root: &Path,
    catalog: &ManifestCatalog,
    folder: &Path,
    slug: &str,
) -> Result<(AssetId, PathBuf), String> {
    for ordinal in 1..=10_000usize {
        let suffix = if ordinal == 1 {
            String::new()
        } else {
            format!("-{ordinal}")
        };
        let id = fit_asset_id(slug, &suffix);
        let relative = folder.join(format!("{id}.material.json"));
        if !catalog.contains_id(&id)
            && !catalog.source_path_is_declared(&relative)
            && resolve_case_insensitive(source_root, &relative)?.is_none()
        {
            validate_asset_id(&AssetId::new(&id))?;
            return Ok((AssetId::new(id), relative));
        }
    }
    Err("could not allocate a unique material asset identity".into())
}

fn next_duplicate_identity(
    source_root: &Path,
    catalog: &ManifestCatalog,
    original_id: &AssetId,
    original_source: &Path,
) -> Result<(AssetId, PathBuf), String> {
    let file_name = original_source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            format!(
                "asset source file name is not UTF-8: {}",
                original_source.display()
            )
        })?;
    let (stem, extension) = split_portable_asset_file_name(file_name)?;
    let parent = original_source.parent().unwrap_or_else(|| Path::new(""));
    for ordinal in 1..=10_000usize {
        let ordinal_suffix = if ordinal == 1 {
            String::new()
        } else {
            format!("-{ordinal}")
        };
        let id_suffix = format!("-copy{ordinal_suffix}");
        let new_id_string = fit_asset_id(&original_id.id, &id_suffix);
        let file_name = format!("{stem}-copy{ordinal_suffix}{extension}");
        let relative = parent.join(file_name);
        if !catalog.contains_id(&new_id_string)
            && !catalog.source_path_is_declared(&relative)
            && resolve_case_insensitive(source_root, &relative)?.is_none()
        {
            let new_id = AssetId::new(new_id_string);
            validate_asset_id(&new_id)?;
            return Ok((new_id, relative));
        }
    }
    Err(format!(
        "could not allocate a unique duplicate identity for '{}'",
        original_id.id
    ))
}

fn split_portable_asset_file_name(file_name: &str) -> Result<(&str, &str), String> {
    let lower = file_name.to_ascii_lowercase();
    if lower.ends_with(".material.json") {
        let split = file_name.len() - ".material.json".len();
        return Ok((&file_name[..split], &file_name[split..]));
    }
    let path = Path::new(file_name);
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .ok_or_else(|| format!("asset source has no portable stem: '{file_name}'"))?;
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| &file_name[file_name.len() - extension.len() - 1..])
        .unwrap_or("");
    Ok((stem, extension))
}

fn fit_asset_id(base: &str, suffix: &str) -> String {
    let maximum_base = 128usize.saturating_sub(suffix.len()).max(1);
    let base = &base[..base.len().min(maximum_base)];
    format!("{base}{suffix}")
}

fn portable_slug(requested_name: &str) -> Result<String, String> {
    let mut slug = String::new();
    let mut separator_pending = false;
    for character in requested_name.trim().chars() {
        if character.is_ascii_alphanumeric() {
            if separator_pending && !slug.is_empty() {
                slug.push('-');
            }
            separator_pending = false;
            slug.push(character.to_ascii_lowercase());
        } else {
            separator_pending = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        return Err("material name must contain at least one ASCII letter or digit".into());
    }
    if slug.len() > 128 {
        slug.truncate(128);
    }
    validate_asset_id(&AssetId::new(&slug))?;
    Ok(slug)
}

fn rewrite_duplicated_source_identity(
    asset_type: &AssetType,
    duplicated_source: &Path,
    new_id: &AssetId,
) -> Result<(), String> {
    match asset_type {
        AssetType::Prefab => {
            let bytes = std::fs::read(duplicated_source).map_err(io_read(duplicated_source))?;
            let mut prefab = engine_scene::parse_prefab_source(&bytes).map_err(|error| {
                format!(
                    "could not rewrite duplicated prefab identity in {}: {error}",
                    duplicated_source.display()
                )
            })?;
            prefab.source_asset = new_id.clone();
            let source = engine_scene::serialize_prefab_source(&prefab).map_err(|error| {
                format!(
                    "could not serialize duplicated prefab identity in {}: {error}",
                    duplicated_source.display()
                )
            })?;
            super::project_cli::atomic_write_bytes(duplicated_source, source.as_bytes())
        }
        AssetType::Logic => {
            let bytes = std::fs::read(duplicated_source).map_err(io_read(duplicated_source))?;
            let mut logic: LogicAsset = serde_json::from_slice(&bytes).map_err(|error| {
                format!(
                    "could not rewrite duplicated logic identity in {}: {error}",
                    duplicated_source.display()
                )
            })?;
            logic.asset_id.clone_from(&new_id.id);
            let mut source = serde_json::to_vec_pretty(&logic).map_err(|error| {
                format!(
                    "could not serialize duplicated logic identity in {}: {error}",
                    duplicated_source.display()
                )
            })?;
            source.push(b'\n');
            super::project_cli::atomic_write_bytes(duplicated_source, &source)
        }
        AssetType::Scene => {
            let mut scene = engine_scene::Scene::load_from_file(duplicated_source).map_err(|error| {
                format!(
                    "could not rewrite duplicated scene identity in {}: {error}",
                    duplicated_source.display()
                )
            })?;
            scene.scene_id.clone_from(&new_id.id);
            scene.save_to_file(duplicated_source).map_err(|error| {
                format!(
                    "could not serialize duplicated scene identity in {}: {error}",
                    duplicated_source.display()
                )
            })
        }
        // These source formats have no engine-owned, embedded self identity.
        // Their external references intentionally continue to point at the
        // same assets after duplication.
        AssetType::Mesh
        | AssetType::Texture
        | AssetType::EnvironmentMap
        | AssetType::Shader
        | AssetType::Material
        | AssetType::Audio
        | AssetType::Animation
        | AssetType::Skeleton
        | AssetType::NavMesh => Ok(()),
        // Do not silently create a second identity path for formats whose
        // source identity contract is not defined. Adding support requires an
        // explicit arm above, which keeps future enum additions fail-closed.
        AssetType::MorphTargetSet
        | AssetType::Pipeline
        | AssetType::Script
        | AssetType::Font
        | AssetType::Unknown => Err(
            format!(
                "asset type {asset_type:?} has no declared duplicate identity policy; duplication is refused"
            ),
        ),
    }
}

fn collect_value_asset_dependencies(value: &Value, dependencies: &mut BTreeSet<AssetId>) {
    match value {
        Value::Asset(asset) => {
            dependencies.insert(asset.clone());
        }
        Value::List(values) => {
            for value in values {
                collect_value_asset_dependencies(value, dependencies);
            }
        }
        Value::Map(values) => {
            for value in values.values() {
                collect_value_asset_dependencies(value, dependencies);
            }
        }
        _ => {}
    }
}

fn collect_scene_asset_dependencies(
    scene: &engine_scene::Scene,
    dependencies: &mut BTreeSet<AssetId>,
) {
    dependencies.extend(scene.collect_asset_dependencies());
    dependencies.extend(scene.dependencies.iter().cloned());
}

fn collect_logic_value_dependency(value: &LogicValue, dependencies: &mut BTreeSet<AssetId>) {
    if let LogicValue::AssetRef(asset) = value {
        dependencies.insert(asset.clone());
    }
}

fn collect_logic_condition_dependencies(
    condition: &LogicCondition,
    dependencies: &mut BTreeSet<AssetId>,
) {
    match condition {
        LogicCondition::Comparison { value, .. } => {
            collect_logic_value_dependency(value, dependencies);
        }
        LogicCondition::And(conditions) | LogicCondition::Or(conditions) => {
            for condition in conditions {
                collect_logic_condition_dependencies(condition, dependencies);
            }
        }
        LogicCondition::Not(condition) => {
            collect_logic_condition_dependencies(condition, dependencies);
        }
        LogicCondition::Always | LogicCondition::Never | LogicCondition::BoolParam(_) => {}
    }
}

fn source_asset_dependencies(
    project: &GameProject,
    entry: &SourceAssetEntry,
) -> Result<BTreeSet<AssetId>, String> {
    let relative =
        normalize_relative_path(Path::new(&entry.source_path), "manifest source path", false)?;
    let path = project.asset_source.join(relative);
    let mut dependencies = BTreeSet::new();
    match &entry.asset_type {
        AssetType::Material => {
            let material: MaterialSource = serde_json::from_slice(
                &std::fs::read(&path).map_err(io_read(&path))?,
            )
            .map_err(|error| {
                format!(
                    "could not inspect material '{}' for references at {}: {error}",
                    entry.id.id,
                    path.display()
                )
            })?;
            for texture in [
                material.base_color_texture,
                material.normal_texture,
                material.metallic_roughness_texture,
                material.occlusion_texture,
                material.emissive_texture,
            ]
            .into_iter()
            .flatten()
            {
                dependencies.insert(AssetId::new(texture));
            }
        }
        AssetType::Scene => {
            let scene = engine_scene::Scene::load_from_file(&path).map_err(|error| {
                format!(
                    "could not inspect source scene '{}' for references at {}: {error}",
                    entry.id.id,
                    path.display()
                )
            })?;
            collect_scene_asset_dependencies(&scene, &mut dependencies);
        }
        AssetType::Prefab => {
            let bytes = std::fs::read(&path).map_err(io_read(&path))?;
            let prefab = engine_scene::parse_prefab_source(&bytes).map_err(|error| {
                format!(
                    "could not inspect prefab '{}' for references at {}: {error}",
                    entry.id.id,
                    path.display()
                )
            })?;
            for entity in &prefab.hierarchy {
                for component in entity.components.values() {
                    for value in component.fields.values() {
                        collect_value_asset_dependencies(value, &mut dependencies);
                    }
                }
            }
            for fields in prefab.component_defaults.values() {
                for value in fields.values() {
                    collect_value_asset_dependencies(value, &mut dependencies);
                }
            }
            dependencies.extend(
                prefab
                    .child_prefab_refs
                    .iter()
                    .map(|reference| reference.prefab_asset.clone()),
            );
        }
        AssetType::Logic => {
            let logic: LogicAsset = serde_json::from_slice(
                &std::fs::read(&path).map_err(io_read(&path))?,
            )
            .map_err(|error| {
                format!(
                    "could not inspect logic asset '{}' for references at {}: {error}",
                    entry.id.id,
                    path.display()
                )
            })?;
            for node in &logic.nodes {
                for value in node.properties.values() {
                    collect_logic_value_dependency(value, &mut dependencies);
                }
                for transition in &node.transitions {
                    if let Some(condition) = &transition.condition {
                        collect_logic_condition_dependencies(condition, &mut dependencies);
                    }
                }
            }
            for parameter in logic.parameters.values() {
                if let Some(default) = &parameter.default {
                    collect_logic_value_dependency(default, &mut dependencies);
                }
            }
        }
        AssetType::Mesh
        | AssetType::Texture
        | AssetType::EnvironmentMap
        | AssetType::MorphTargetSet
        | AssetType::Shader
        | AssetType::Pipeline
        | AssetType::Script
        | AssetType::Audio
        | AssetType::Font
        | AssetType::Animation
        | AssetType::Skeleton
        | AssetType::NavMesh
        | AssetType::Unknown => {}
    }
    Ok(dependencies)
}

fn reject_known_references(
    project: &GameProject,
    catalog: &ManifestCatalog,
    target: &SourceAssetEntry,
) -> Result<(), String> {
    let mut references = Vec::new();
    for (scene_id, scene_path) in project.scenes() {
        let scene = engine_scene::Scene::load_from_file(&scene_path).map_err(|error| {
            format!(
                "could not check scene '{scene_id}' for asset references at {}: {error}",
                scene_path.display()
            )
        })?;
        let mut dependencies = BTreeSet::new();
        collect_scene_asset_dependencies(&scene, &mut dependencies);
        if dependencies
            .iter()
            .any(|dependency| dependency.id == target.id.id)
        {
            references.push(format!("scene:{scene_id}"));
        }
    }
    for document in &catalog.documents {
        for entry in &document.manifest.assets {
            if entry.id.id == target.id.id {
                continue;
            }
            let dependencies = source_asset_dependencies(project, entry)?;
            if dependencies
                .iter()
                .any(|dependency| dependency.id == target.id.id)
            {
                references.push(format!(
                    "{}:{}",
                    match &entry.asset_type {
                        AssetType::Scene => "source-scene",
                        AssetType::Material => "material",
                        AssetType::Prefab => "prefab",
                        AssetType::Logic => "logic",
                        _ => "asset",
                    },
                    entry.id.id
                ));
            }
        }
    }
    references.sort();
    references.dedup();
    if references.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "asset '{}' is still referenced by {}; remove those references before deleting it",
            target.id.id,
            references.join(", ")
        ))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct TrashMetadata {
    schema: String,
    deleted_unix_nanos: u128,
    manifest_path: String,
    source_path: String,
    cooked_path: String,
    entry: SourceAssetEntry,
}

fn allocate_trash_directory(project: &GameProject, asset_id: &AssetId) -> Result<PathBuf, String> {
    let root = project.root.join(".engine/trash/assets");
    ensure_no_symlink_ancestors(&project.root, &root)?;
    let timestamp = unix_nanos();
    for attempt in 0..100usize {
        let path = root.join(format!(
            "{timestamp}-{}-{}-{attempt}",
            asset_id.id,
            std::process::id()
        ));
        if !path.exists() {
            return Ok(path);
        }
    }
    Err(format!(
        "could not allocate a unique trash directory below {}",
        root.display()
    ))
}

fn load_project(project_path: &Path) -> Result<GameProject, String> {
    GameProject::load(project_path).map_err(|error| error.to_string())
}

fn lock_asset_operations() -> Result<MutexGuard<'static, ()>, String> {
    ASSET_OPERATION_MUTEX
        .lock()
        .map_err(|_| "asset operation lock was poisoned by a prior panic".to_string())
}

fn cooked_path(project: &GameProject, asset_id: &AssetId) -> PathBuf {
    project
        .cooked_assets
        .join(format!("{}.cooked", asset_id.id))
}

fn validate_asset_id(asset_id: &AssetId) -> Result<(), String> {
    engine_asset::validate_asset_id(asset_id).map_err(|error| error.to_string())?;
    if asset_id.id.len() > 128 {
        return Err("asset id may not exceed 128 bytes".into());
    }
    if !asset_id
        .id
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.'))
    {
        return Err(format!(
            "asset id '{}' must use ASCII letters, digits, hyphens, underscores, or dots",
            asset_id.id
        ));
    }
    Ok(())
}

fn normalize_relative_path(path: &Path, label: &str, allow_empty: bool) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty() {
        return if allow_empty {
            Ok(PathBuf::new())
        } else {
            Err(format!("{label} may not be empty"))
        };
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(component) => {
                let component = component
                    .to_str()
                    .ok_or_else(|| format!("{label} contains non-UTF-8 text"))?;
                validate_portable_component(component, label)?;
                normalized.push(component);
            }
            Component::CurDir => {
                return Err(format!("{label} may not contain '.' path components"));
            }
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!("{label} must be a portable project-relative path"));
            }
        }
    }
    if normalized.as_os_str().is_empty() && !allow_empty {
        Err(format!("{label} may not be empty"))
    } else {
        Ok(normalized)
    }
}

fn validate_portable_component(component: &str, label: &str) -> Result<(), String> {
    if component.is_empty()
        || component.ends_with([' ', '.'])
        || component.chars().any(|character| {
            character.is_control()
                || matches!(
                    character,
                    '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
                )
        })
    {
        return Err(format!(
            "{label} contains a non-portable path component '{component}'"
        ));
    }
    let stem = component
        .split('.')
        .next()
        .unwrap_or(component)
        .to_ascii_uppercase();
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit()
            && stem.as_bytes()[3] != b'0')
    {
        return Err(format!(
            "{label} uses reserved portable file name '{component}'"
        ));
    }
    Ok(())
}

fn manifest_path_string(path: &Path) -> Result<String, String> {
    let path = normalize_relative_path(path, "manifest source path", false)?;
    path.components()
        .map(|component| match component {
            Component::Normal(value) => value
                .to_str()
                .map(str::to_owned)
                .ok_or_else(|| "manifest source path contains non-UTF-8 text".to_string()),
            _ => Err("manifest source path is not normalized".into()),
        })
        .collect::<Result<Vec<_>, _>>()
        .map(|parts| parts.join("/"))
}

fn project_relative_string(project: &GameProject, path: &Path) -> Result<String, String> {
    let relative = path.strip_prefix(&project.root).map_err(|_| {
        format!(
            "path is outside project root and cannot be recorded in trash metadata: {}",
            path.display()
        )
    })?;
    manifest_path_string(relative)
}

fn portable_path_key(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(value) => value.to_str(),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
        .to_ascii_lowercase()
}

fn ensure_existing_folder(root: &Path, relative: &Path) -> Result<(), String> {
    if relative.as_os_str().is_empty() {
        return Ok(());
    }
    let path = root.join(relative);
    if !path.is_dir() {
        return Err(format!("asset folder does not exist: {}", path.display()));
    }
    reject_symlink(&path)
}

fn ensure_parent_is_real_directory(root: &Path, relative: &Path) -> Result<(), String> {
    let parent = relative.parent().unwrap_or_else(|| Path::new(""));
    ensure_existing_folder(root, parent)?;
    if let Some(case_match) = resolve_case_insensitive(root, parent)? {
        let requested = root.join(parent);
        if case_match != requested {
            return Err(format!(
                "asset path parent differs only by case from existing folder: {}",
                case_match.display()
            ));
        }
    }
    Ok(())
}

fn ensure_destination_absent(root: &Path, relative: &Path) -> Result<(), String> {
    if let Some(conflict) = resolve_case_insensitive(root, relative)? {
        return Err(format!(
            "asset destination already exists or differs only by case: {}",
            conflict.display()
        ));
    }
    Ok(())
}

fn resolve_case_insensitive(root: &Path, relative: &Path) -> Result<Option<PathBuf>, String> {
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Err(format!("path is not normalized: {}", relative.display()));
        };
        if !current.is_dir() {
            return Ok(None);
        }
        let requested = component.to_string_lossy();
        let mut found = None;
        for entry in std::fs::read_dir(&current)
            .map_err(|error| format!("could not enumerate {}: {error}", current.display()))?
        {
            let entry = entry
                .map_err(|error| format!("could not enumerate {}: {error}", current.display()))?;
            if entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.eq_ignore_ascii_case(&requested))
            {
                found = Some(entry.path());
                break;
            }
        }
        let Some(path) = found else {
            return Ok(None);
        };
        current = path;
    }
    Ok(Some(current))
}

fn reject_symlink(path: &Path) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("could not inspect {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() {
        Err(format!(
            "asset operations do not follow symbolic links: {}",
            path.display()
        ))
    } else {
        Ok(())
    }
}

fn ensure_no_symlink_ancestors(root: &Path, path: &Path) -> Result<(), String> {
    let relative = path.strip_prefix(root).map_err(|_| {
        format!(
            "asset operation path escapes project root: {}",
            path.display()
        )
    })?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component);
        if current.exists() {
            reject_symlink(&current)?;
        }
    }
    Ok(())
}

fn copy_directory_tree(source: &Path, destination: &Path) -> Result<(), String> {
    reject_symlink(source)?;
    std::fs::create_dir(destination).map_err(|error| {
        format!(
            "could not create staged source directory {}: {error}",
            destination.display()
        )
    })?;
    for entry in std::fs::read_dir(source)
        .map_err(|error| format!("could not enumerate {}: {error}", source.display()))?
    {
        let entry =
            entry.map_err(|error| format!("could not enumerate {}: {error}", source.display()))?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let metadata = std::fs::symlink_metadata(&source_path)
            .map_err(|error| format!("could not inspect {}: {error}", source_path.display()))?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "asset operations do not copy symbolic links: {}",
                source_path.display()
            ));
        }
        if metadata.is_dir() {
            copy_directory_tree(&source_path, &destination_path)?;
        } else if metadata.is_file() {
            copy_file_create_new(&source_path, &destination_path)?;
        } else {
            return Err(format!(
                "unsupported source-tree entry: {}",
                source_path.display()
            ));
        }
    }
    Ok(())
}

fn copy_file_create_new(source: &Path, destination: &Path) -> Result<(), String> {
    let bytes = std::fs::read(source).map_err(io_read(source))?;
    write_file_create_new(destination, &bytes)
}

fn write_file_create_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("path has no parent: {}", path.display()))?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| {
            format!(
                "could not create {} without overwriting an existing file: {error}",
                path.display()
            )
        })?;
    if let Err(error) = file.write_all(bytes).and_then(|()| file.sync_all()) {
        drop(file);
        let _ = std::fs::remove_file(path);
        return Err(format!("could not write {}: {error}", path.display()));
    }
    Ok(())
}

fn serialize_manifest(manifest: &SourceManifest) -> Result<Vec<u8>, String> {
    let mut bytes = serde_json::to_vec_pretty(manifest)
        .map_err(|error| format!("could not serialize source manifest: {error}"))?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn map_staged_path(staged_root: &Path, live_root: &Path, path: &Path) -> Result<PathBuf, String> {
    path.strip_prefix(staged_root)
        .map(|relative| live_root.join(relative))
        .map_err(|_| format!("staged path escapes source workspace: {}", path.display()))
}

fn io_read(path: &Path) -> impl FnOnce(std::io::Error) -> String + '_ {
    move |error| format!("could not read {}: {error}", path.display())
}

fn unix_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        directory: tempfile::TempDir,
    }

    impl Fixture {
        fn new() -> Self {
            let directory = tempfile::tempdir().expect("temporary project");
            super::super::project_cli::create_project(
                directory.path(),
                Some("Asset Operations"),
                false,
            )
            .expect("create project");
            Self { directory }
        }

        fn root(&self) -> &Path {
            self.directory.path()
        }

        fn manifest_path(&self) -> PathBuf {
            self.root().join("assets/source/game.manifest")
        }

        fn manifest(&self) -> SourceManifest {
            serde_json::from_slice(
                &std::fs::read(self.manifest_path()).expect("read source manifest"),
            )
            .expect("parse source manifest")
        }

        fn create_material(&self, name: &str) -> AssetMutation {
            create_material_asset(
                self.root(),
                Path::new(""),
                name,
                &MaterialTemplate::default(),
            )
            .expect("create material")
        }

        fn declare_asset(
            &self,
            id: &str,
            asset_type: AssetType,
            relative_source: &str,
            bytes: &[u8],
        ) -> SourceAssetEntry {
            let source = self.root().join("assets/source").join(relative_source);
            if let Some(parent) = source.parent() {
                std::fs::create_dir_all(parent).expect("create declared source parent");
            }
            std::fs::write(&source, bytes).expect("write declared source");
            let entry = SourceAssetEntry {
                id: AssetId::new(id),
                asset_type,
                source_path: relative_source.to_string(),
                cook_rules: CookRules::default(),
            };
            let mut manifest = self.manifest();
            manifest.assets.push(entry.clone());
            manifest
                .assets
                .sort_by(|left, right| left.id.id.cmp(&right.id.id));
            std::fs::write(
                self.manifest_path(),
                serde_json::to_vec_pretty(&manifest).expect("serialize source manifest"),
            )
            .expect("write source manifest");
            entry
        }
    }

    fn prefab_source(
        self_id: &str,
        hierarchy_asset: &str,
        default_asset: &str,
        child_asset: &str,
    ) -> String {
        let root_id = "prefab-root".to_string();
        let mut prefab = engine_scene::Prefab::new(AssetId::new(self_id));
        prefab.add_entity(engine_scene::EntityRecord {
            persistent_id: root_id.clone(),
            parent: None,
            name: Some("Prefab Root".into()),
            enabled: true,
            components: BTreeMap::from([(
                "test.component".into(),
                engine_scene::ComponentRecord {
                    schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
                    enabled: true,
                    fields: BTreeMap::from([(
                        "asset".into(),
                        Value::List(vec![Value::Map(BTreeMap::from([(
                            "nested".into(),
                            Value::Asset(AssetId::new(hierarchy_asset)),
                        )]))]),
                    )]),
                },
            )]),
        });
        prefab.set_default(
            "test.component",
            "default_asset",
            Value::Asset(AssetId::new(default_asset)),
        );
        prefab.child_prefab_refs.push(engine_scene::PrefabChildRef {
            entity_persistent_id: root_id,
            prefab_asset: AssetId::new(child_asset),
        });
        engine_scene::serialize_prefab_source(&prefab).expect("serialize prefab source")
    }

    fn logic_source(
        self_id: &str,
        property_asset: &str,
        default_asset: &str,
        condition_asset: &str,
    ) -> Vec<u8> {
        use engine_asset::cook::logic_asset::{
            ComparisonOp, LogicAssetKind, LogicMetadata, LogicNode, LogicParam, LogicParamType,
            LogicTransition,
        };

        let logic = LogicAsset {
            schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
            asset_id: self_id.into(),
            kind: LogicAssetKind::BehaviorTree,
            nodes: vec![LogicNode {
                id: "root".into(),
                node_type: "action".into(),
                label: Some("Root".into()),
                transitions: vec![LogicTransition {
                    target_node: "root".into(),
                    condition: Some(LogicCondition::Not(Box::new(LogicCondition::And(vec![
                        LogicCondition::Comparison {
                            param: "asset".into(),
                            op: ComparisonOp::Equal,
                            value: LogicValue::AssetRef(AssetId::new(condition_asset)),
                        },
                    ])))),
                    priority: 0,
                }],
                properties: BTreeMap::from([(
                    "asset".into(),
                    LogicValue::AssetRef(AssetId::new(property_asset)),
                )]),
                children: Vec::new(),
            }],
            parameters: BTreeMap::from([(
                "asset".into(),
                LogicParam {
                    name: "asset".into(),
                    param_type: LogicParamType::AssetRef,
                    default: Some(LogicValue::AssetRef(AssetId::new(default_asset))),
                    description: None,
                },
            )]),
            metadata: LogicMetadata {
                author: None,
                description: None,
                tags: Vec::new(),
                version: "1.0.0".into(),
            },
        };
        serde_json::to_vec_pretty(&logic).expect("serialize logic source")
    }

    #[test]
    fn folder_creation_is_single_step_and_refuses_conflicts() {
        let fixture = Fixture::new();
        let materials =
            create_asset_folder(fixture.root(), Path::new("Materials")).expect("create folder");
        assert!(materials.is_dir());
        let original_entries = std::fs::read_dir(&materials)
            .expect("read materials")
            .count();
        let error = create_asset_folder(fixture.root(), Path::new("materials"))
            .expect_err("case collision must fail");
        assert!(error.contains("differs only by case") || error.contains("already exists"));
        assert_eq!(
            std::fs::read_dir(materials)
                .expect("read unchanged materials")
                .count(),
            original_entries
        );
    }

    #[test]
    fn folder_rename_updates_declared_source_paths_and_preserves_ids() {
        let fixture = Fixture::new();
        create_asset_folder(fixture.root(), Path::new("Materials")).expect("source folder");
        let material = create_material_asset(
            fixture.root(),
            Path::new("Materials"),
            "Ground",
            &MaterialTemplate::default(),
        )
        .expect("material");

        let renamed = rename_asset_folder(
            fixture.root(),
            Path::new("Materials"),
            Path::new("Environment"),
        )
        .expect("rename folder");

        assert!(renamed.ends_with("assets/source/Environment"));
        assert!(!fixture.root().join("assets/source/Materials").exists());
        assert!(fixture
            .root()
            .join("assets/source/Environment/ground.material.json")
            .is_file());
        let entry = fixture
            .manifest()
            .assets
            .into_iter()
            .find(|entry| entry.id == material.asset_id)
            .expect("renamed manifest entry");
        assert_eq!(entry.source_path, "Environment/ground.material.json");
        assert!(material.cooked_path.is_file());
    }

    #[test]
    fn folder_rename_rejects_escape_self_and_case_collisions() {
        let fixture = Fixture::new();
        create_asset_folder(fixture.root(), Path::new("Models")).expect("models folder");
        create_asset_folder(fixture.root(), Path::new("Other")).expect("other folder");

        assert!(
            rename_asset_folder(fixture.root(), Path::new("Models"), Path::new("../outside"),)
                .expect_err("escape must fail")
                .contains("project-relative")
        );
        assert!(rename_asset_folder(
            fixture.root(),
            Path::new("Models"),
            Path::new("mOdElS/Nested"),
        )
        .expect_err("self move must fail")
        .contains("current parent"));
        assert!(
            rename_asset_folder(fixture.root(), Path::new("Models"), Path::new("other"),)
                .expect_err("case collision must fail")
                .contains("already exists")
        );
        assert!(fixture.root().join("assets/source/Models").is_dir());
    }

    #[test]
    fn folder_rename_rejects_cross_parent_moves_to_protect_relative_sidecars() {
        let fixture = Fixture::new();
        create_asset_folder(fixture.root(), Path::new("Models")).expect("models folder");
        create_asset_folder(fixture.root(), Path::new("Packages")).expect("packages folder");

        let error = rename_asset_folder(
            fixture.root(),
            Path::new("Models"),
            Path::new("Packages/RenamedModels"),
        )
        .expect_err("cross-parent move must fail");

        assert!(error.contains("current parent"));
        assert!(fixture.root().join("assets/source/Models").is_dir());
        assert!(!fixture
            .root()
            .join("assets/source/Packages/RenamedModels")
            .exists());
    }

    #[test]
    fn folder_rename_moves_nested_manifest_files_without_recreating_old_folder() {
        let fixture = Fixture::new();
        create_asset_folder(fixture.root(), Path::new("Packages")).expect("package folder");
        let nested_manifest = fixture.root().join("assets/source/Packages/local.manifest");
        std::fs::write(
            &nested_manifest,
            serde_json::to_vec_pretty(&SourceManifest {
                schema_version: CURRENT_MANIFEST_VERSION,
                assets: Vec::new(),
            })
            .expect("serialize nested manifest"),
        )
        .expect("write nested manifest");

        rename_asset_folder(fixture.root(), Path::new("Packages"), Path::new("Vendor"))
            .expect("rename folder with nested manifest file");

        assert!(!fixture.root().join("assets/source/Packages").exists());
        assert!(fixture
            .root()
            .join("assets/source/Vendor/local.manifest")
            .is_file());
    }

    #[test]
    fn folder_delete_only_removes_empty_non_root_folders() {
        let fixture = Fixture::new();
        create_asset_folder(fixture.root(), Path::new("Empty")).expect("empty folder");
        delete_asset_folder(fixture.root(), Path::new("Empty")).expect("delete empty folder");
        assert!(!fixture.root().join("assets/source/Empty").exists());

        create_asset_folder(fixture.root(), Path::new("Occupied")).expect("occupied folder");
        std::fs::write(
            fixture.root().join("assets/source/Occupied/readme.txt"),
            b"keep",
        )
        .expect("write occupant");
        let error = delete_asset_folder(fixture.root(), Path::new("Occupied"))
            .expect_err("non-empty folder must not be recursively deleted");
        assert!(error.contains("not empty"));
        assert!(fixture
            .root()
            .join("assets/source/Occupied/readme.txt")
            .is_file());
        assert!(delete_asset_folder(fixture.root(), Path::new(""))
            .expect_err("source root is protected")
            .contains("may not be empty"));
    }

    #[test]
    fn material_create_writes_manifest_source_and_valid_cooked_artifact() {
        let fixture = Fixture::new();
        create_asset_folder(fixture.root(), Path::new("Materials")).expect("material folder");
        let created = create_material_asset(
            fixture.root(),
            Path::new("Materials"),
            "Hero Surface",
            &MaterialTemplate::default(),
        )
        .expect("create material");

        assert_eq!(created.asset_id.id, "hero-surface");
        assert!(created
            .source_path
            .ends_with("Materials/hero-surface.material.json"));
        assert!(created.source_path.is_file());
        let artifact = read_cooked_artifact(&created.cooked_path).expect("valid cooked material");
        assert_eq!(artifact.header.asset_kind, AssetType::Material.kind_code());
        let manifest = fixture.manifest();
        let entry = manifest
            .assets
            .iter()
            .find(|entry| entry.id == created.asset_id)
            .expect("manifest entry");
        assert_eq!(entry.source_path, "Materials/hero-surface.material.json");
    }

    #[test]
    fn failed_material_cook_rolls_back_every_live_file() {
        let fixture = Fixture::new();
        let original_manifest = std::fs::read(fixture.manifest_path()).expect("manifest snapshot");
        let invalid = MaterialTemplate {
            metallic: 2.0,
            ..MaterialTemplate::default()
        };
        let error =
            create_material_asset(fixture.root(), Path::new(""), "Broken Material", &invalid)
                .expect_err("invalid material must not commit");
        assert!(error.contains("cooking failed"));
        assert_eq!(
            std::fs::read(fixture.manifest_path()).expect("manifest after failure"),
            original_manifest
        );
        assert!(!fixture
            .root()
            .join("assets/source/broken-material.material.json")
            .exists());
        assert!(!fixture
            .root()
            .join("build/cooked/broken-material.cooked")
            .exists());
    }

    #[test]
    fn duplicate_generates_unique_stable_ids_and_cooked_assets() {
        let fixture = Fixture::new();
        let original = fixture.create_material("Ground");
        let first =
            duplicate_project_asset(fixture.root(), &original.asset_id).expect("first duplicate");
        let second =
            duplicate_project_asset(fixture.root(), &original.asset_id).expect("second duplicate");

        assert_eq!(first.asset_id.id, "ground-copy");
        assert_eq!(second.asset_id.id, "ground-copy-2");
        assert_ne!(first.source_path, second.source_path);
        assert!(first.source_path.is_file());
        assert!(second.source_path.is_file());
        read_cooked_artifact(&first.cooked_path).expect("first cooked duplicate");
        read_cooked_artifact(&second.cooked_path).expect("second cooked duplicate");
        let ids = fixture
            .manifest()
            .assets
            .into_iter()
            .map(|entry| entry.id.id)
            .collect::<BTreeSet<_>>();
        assert!(ids.contains("ground"));
        assert!(ids.contains("ground-copy"));
        assert!(ids.contains("ground-copy-2"));
    }

    #[test]
    fn duplicate_rewrites_prefab_self_identity_and_preserves_external_references() {
        let fixture = Fixture::new();
        let source = prefab_source(
            "prefab-original",
            "mesh-external",
            "material-external",
            "prefab-child",
        );
        let original = fixture.declare_asset(
            "prefab-original",
            AssetType::Prefab,
            "prefab-original.prefab.ron",
            source.as_bytes(),
        );

        let duplicate = duplicate_project_asset(fixture.root(), &original.id).expect("duplicate");
        let duplicated = engine_scene::parse_prefab_source(
            &std::fs::read(&duplicate.source_path).expect("read duplicated prefab"),
        )
        .expect("parse duplicated prefab");

        assert_eq!(duplicated.source_asset, duplicate.asset_id);
        let component = &duplicated.hierarchy[0].components["test.component"];
        let Value::List(values) = &component.fields["asset"] else {
            panic!("nested prefab dependency must remain a list");
        };
        let Value::Map(values) = &values[0] else {
            panic!("nested prefab dependency must remain a map");
        };
        assert_eq!(
            values.get("nested"),
            Some(&Value::Asset(AssetId::new("mesh-external")))
        );
        assert_eq!(
            duplicated.component_defaults["test.component"]["default_asset"],
            Value::Asset(AssetId::new("material-external"))
        );
        assert_eq!(
            duplicated.child_prefab_refs[0].prefab_asset,
            AssetId::new("prefab-child")
        );
    }

    #[test]
    fn duplicate_rewrites_logic_self_identity_and_preserves_asset_references() {
        let fixture = Fixture::new();
        let original = fixture.declare_asset(
            "logic-original",
            AssetType::Logic,
            "logic-original.logic.json",
            &logic_source(
                "logic-original",
                "property-external",
                "default-external",
                "condition-external",
            ),
        );

        let duplicate = duplicate_project_asset(fixture.root(), &original.id).expect("duplicate");
        let duplicated: LogicAsset = serde_json::from_slice(
            &std::fs::read(&duplicate.source_path).expect("read duplicated logic"),
        )
        .expect("parse duplicated logic");

        assert_eq!(duplicated.asset_id, duplicate.asset_id.id);
        assert!(matches!(
            duplicated.nodes[0].properties.get("asset"),
            Some(LogicValue::AssetRef(asset)) if asset.id == "property-external"
        ));
        assert!(matches!(
            duplicated.parameters["asset"].default.as_ref(),
            Some(LogicValue::AssetRef(asset)) if asset.id == "default-external"
        ));
        let Some(LogicCondition::Not(condition)) = &duplicated.nodes[0].transitions[0].condition
        else {
            panic!("logic condition structure must be preserved");
        };
        let LogicCondition::And(conditions) = condition.as_ref() else {
            panic!("logic and condition must be preserved");
        };
        assert!(matches!(
            &conditions[0],
            LogicCondition::Comparison { value: LogicValue::AssetRef(asset), .. }
                if asset.id == "condition-external"
        ));
    }

    #[test]
    fn duplicate_rewrites_scene_identity_and_preserves_asset_dependencies() {
        let fixture = Fixture::new();
        let mut scene = engine_scene::Scene::load_from_file(
            &fixture.root().join("assets/scenes/main.scene.ron"),
        )
        .expect("load fixture scene");
        scene.scene_id = "scene-original".into();
        scene.dependencies = vec![AssetId::new("external-dependency")];
        scene.scene_settings.environment_map = Some(AssetId::new("external-environment"));
        let source = fixture
            .root()
            .join("assets/source/scene-original.scene.ron");
        scene.save_to_file(&source).expect("write scene source");
        let original = fixture.declare_asset(
            "scene-original",
            AssetType::Scene,
            "scene-original.scene.ron",
            &std::fs::read(&source).expect("read scene source"),
        );

        let duplicate = duplicate_project_asset(fixture.root(), &original.id).expect("duplicate");
        let duplicated =
            engine_scene::Scene::load_from_file(&duplicate.source_path).expect("load duplicate");

        assert_eq!(duplicated.scene_id, duplicate.asset_id.id);
        assert_eq!(duplicated.dependencies, scene.dependencies);
        assert_eq!(
            duplicated.scene_settings.environment_map,
            scene.scene_settings.environment_map
        );
    }

    #[test]
    fn duplicate_rejects_assets_without_an_explicit_identity_policy() {
        let fixture = Fixture::new();
        let original = fixture.declare_asset(
            "unknown-original",
            AssetType::Unknown,
            "unknown-original.data",
            b"opaque identity-bearing payload",
        );
        let manifest_before = std::fs::read(fixture.manifest_path()).expect("manifest snapshot");

        let error = duplicate_project_asset(fixture.root(), &original.id)
            .expect_err("unknown identity policy must fail closed");

        assert!(error.contains("no declared duplicate identity policy"));
        assert_eq!(
            std::fs::read(fixture.manifest_path()).expect("unchanged manifest"),
            manifest_before
        );
        assert!(!fixture
            .root()
            .join("assets/source/unknown-original-copy.data")
            .exists());
    }

    #[test]
    fn move_preserves_asset_id_and_updates_source_manifest() {
        let fixture = Fixture::new();
        create_asset_folder(fixture.root(), Path::new("Materials")).expect("folder");
        let original = fixture.create_material("Movable");
        let old_source = original.source_path.clone();
        let moved = move_project_asset(
            fixture.root(),
            &original.asset_id,
            Path::new("Materials/Renamed.material.json"),
        )
        .expect("move asset");

        assert_eq!(moved.asset_id, original.asset_id);
        assert!(!old_source.exists());
        assert!(moved.source_path.is_file());
        assert!(moved.cooked_path.is_file());
        let entry = fixture
            .manifest()
            .assets
            .into_iter()
            .find(|entry| entry.id == original.asset_id)
            .expect("moved manifest entry");
        assert_eq!(entry.source_path, "Materials/Renamed.material.json");
    }

    #[test]
    fn move_commit_failure_restores_manifest_source_and_cooked_bytes() {
        let fixture = Fixture::new();
        create_asset_folder(fixture.root(), Path::new("Materials")).expect("folder");
        let original = fixture.create_material("Rollback Move");
        let manifest_before = std::fs::read(fixture.manifest_path()).expect("manifest snapshot");
        let cooked_before = std::fs::read(&original.cooked_path).expect("cooked snapshot");

        let error = move_project_asset_impl(
            fixture.root(),
            &original.asset_id,
            Path::new("Materials/Rollback.material.json"),
            Some(2),
        )
        .expect_err("injected move commit failure");
        assert!(error.contains("simulated asset transaction failure"));
        assert!(original.source_path.is_file());
        assert!(!fixture
            .root()
            .join("assets/source/Materials/Rollback.material.json")
            .exists());
        assert_eq!(
            std::fs::read(fixture.manifest_path()).expect("restored manifest"),
            manifest_before
        );
        assert_eq!(
            std::fs::read(&original.cooked_path).expect("restored cooked"),
            cooked_before
        );
    }

    #[test]
    fn delete_moves_asset_to_project_trash_with_recovery_metadata() {
        let fixture = Fixture::new();
        let material = fixture.create_material("Disposable");
        let deleted =
            delete_project_asset(fixture.root(), &material.asset_id).expect("delete asset");

        assert!(!material.source_path.exists());
        assert!(!material.cooked_path.exists());
        assert!(deleted.trash_directory.starts_with(fixture.root()));
        assert!(deleted.metadata_path.is_file());
        assert!(deleted
            .trash_directory
            .join("source/disposable.material.json")
            .is_file());
        assert!(deleted
            .trash_directory
            .join("cooked/disposable.cooked")
            .is_file());
        let metadata: TrashMetadata =
            serde_json::from_slice(&std::fs::read(deleted.metadata_path).expect("read metadata"))
                .expect("parse metadata");
        assert_eq!(metadata.schema, TRASH_SCHEMA);
        assert_eq!(metadata.entry.id, material.asset_id);
        assert!(fixture.manifest().assets.is_empty());
    }

    #[test]
    fn delete_commit_failure_restores_live_state_and_removes_trash_payloads() {
        let fixture = Fixture::new();
        let material = fixture.create_material("Keep Me");
        let manifest_before = std::fs::read(fixture.manifest_path()).expect("manifest snapshot");
        let source_before = std::fs::read(&material.source_path).expect("source snapshot");
        let cooked_before = std::fs::read(&material.cooked_path).expect("cooked snapshot");

        // Four writes install the trash payloads and updated manifest; fail
        // after removing the live source so rollback must restore real project
        // state, not merely clean up an uncommitted staging area.
        let error = delete_project_asset_impl(fixture.root(), &material.asset_id, Some(5))
            .expect_err("injected delete failure");
        assert!(error.contains("simulated asset transaction failure"));
        assert_eq!(
            std::fs::read(fixture.manifest_path()).expect("restored manifest"),
            manifest_before
        );
        assert_eq!(
            std::fs::read(&material.source_path).expect("restored source"),
            source_before
        );
        assert_eq!(
            std::fs::read(&material.cooked_path).expect("restored cooked"),
            cooked_before
        );
        let trash_root = fixture.root().join(".engine/trash/assets");
        let remaining_files = if trash_root.exists() {
            walk_files(&trash_root)
        } else {
            Vec::new()
        };
        assert!(
            remaining_files.is_empty(),
            "trash payloads: {remaining_files:?}"
        );
    }

    #[test]
    fn delete_refuses_scene_references_without_mutating_asset() {
        let fixture = Fixture::new();
        let material = fixture.create_material("Referenced");
        let scene_path = fixture.root().join("assets/scenes/main.scene.ron");
        let mut scene = engine_scene::Scene::load_from_file(&scene_path).expect("load scene");
        scene.dependencies.push(material.asset_id.clone());
        scene
            .save_to_file(&scene_path)
            .expect("save referenced scene");
        let manifest_before = std::fs::read(fixture.manifest_path()).expect("manifest snapshot");

        let error = delete_project_asset(fixture.root(), &material.asset_id)
            .expect_err("referenced asset must be retained");
        assert!(error.contains("scene:main"));
        assert!(material.source_path.is_file());
        assert!(material.cooked_path.is_file());
        assert_eq!(
            std::fs::read(fixture.manifest_path()).expect("unchanged manifest"),
            manifest_before
        );
    }

    #[test]
    fn source_dependency_extractor_covers_every_identity_bearing_source_field() {
        let fixture = Fixture::new();
        let material_source = MaterialSource {
            schema: MATERIAL_SOURCE_SCHEMA.into(),
            base_color: [1.0; 4],
            metallic: 0.0,
            roughness: 0.5,
            ambient_occlusion: 1.0,
            emissive: [0.0; 3],
            base_color_texture: Some("material-texture".into()),
            normal_texture: None,
            metallic_roughness_texture: None,
            occlusion_texture: None,
            emissive_texture: None,
            advanced: Default::default(),
            transparency: "Opaque".into(),
            alpha_cutoff: 0.5,
            double_sided: false,
        };
        let material = fixture.declare_asset(
            "material-reference-source",
            AssetType::Material,
            "material-reference-source.material.json",
            &serde_json::to_vec_pretty(&material_source).expect("serialize material"),
        );

        let mut scene = engine_scene::Scene::load_from_file(
            &fixture.root().join("assets/scenes/main.scene.ron"),
        )
        .expect("load fixture scene");
        scene.dependencies = vec![AssetId::new("scene-explicit")];
        scene.scene_settings.environment_map = Some(AssetId::new("scene-environment"));
        scene.entities[0].components.insert(
            "test.asset".into(),
            engine_scene::ComponentRecord {
                schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: BTreeMap::from([(
                    "asset".into(),
                    Value::Asset(AssetId::new("scene-component")),
                )]),
            },
        );
        let scene_path = fixture.root().join("assets/source/reference.scene.ron");
        scene.save_to_file(&scene_path).expect("save source scene");
        let scene = fixture.declare_asset(
            "scene-reference-source",
            AssetType::Scene,
            "reference.scene.ron",
            &std::fs::read(scene_path).expect("read source scene"),
        );

        let prefab_source = prefab_source(
            "prefab-reference-source",
            "prefab-hierarchy",
            "prefab-default",
            "prefab-child",
        );
        let prefab = fixture.declare_asset(
            "prefab-reference-source",
            AssetType::Prefab,
            "prefab-reference-source.prefab.ron",
            prefab_source.as_bytes(),
        );
        let logic = fixture.declare_asset(
            "logic-reference-source",
            AssetType::Logic,
            "logic-reference-source.logic.json",
            &logic_source(
                "logic-reference-source",
                "logic-property",
                "logic-default",
                "logic-condition",
            ),
        );
        let project = load_project(fixture.root()).expect("load project");

        assert_eq!(
            source_asset_dependencies(&project, &material).expect("material dependencies"),
            BTreeSet::from([AssetId::new("material-texture")])
        );
        let scene_dependencies =
            source_asset_dependencies(&project, &scene).expect("scene dependencies");
        for id in ["scene-explicit", "scene-environment", "scene-component"] {
            assert!(
                scene_dependencies.contains(&AssetId::new(id)),
                "missing {id}"
            );
        }
        assert_eq!(
            source_asset_dependencies(&project, &prefab).expect("prefab dependencies"),
            BTreeSet::from([
                AssetId::new("prefab-hierarchy"),
                AssetId::new("prefab-default"),
                AssetId::new("prefab-child"),
            ])
        );
        assert_eq!(
            source_asset_dependencies(&project, &logic).expect("logic dependencies"),
            BTreeSet::from([
                AssetId::new("logic-property"),
                AssetId::new("logic-default"),
                AssetId::new("logic-condition"),
            ])
        );
    }

    #[test]
    fn delete_refuses_manifest_material_references() {
        let fixture = Fixture::new();
        let target = fixture.create_material("Referenced");
        let source = MaterialSource {
            schema: MATERIAL_SOURCE_SCHEMA.into(),
            base_color: [1.0; 4],
            metallic: 0.0,
            roughness: 0.5,
            ambient_occlusion: 1.0,
            emissive: [0.0; 3],
            base_color_texture: Some(target.asset_id.id.clone()),
            normal_texture: None,
            metallic_roughness_texture: None,
            occlusion_texture: None,
            emissive_texture: None,
            advanced: Default::default(),
            transparency: "Opaque".into(),
            alpha_cutoff: 0.5,
            double_sided: false,
        };
        fixture.declare_asset(
            "referencing-material",
            AssetType::Material,
            "referencing-material.material.json",
            &serde_json::to_vec_pretty(&source).expect("serialize material"),
        );

        let error = delete_project_asset(fixture.root(), &target.asset_id)
            .expect_err("material dependency must block delete");

        assert!(error.contains("material:referencing-material"));
        assert!(target.source_path.is_file());
    }

    #[test]
    fn delete_refuses_manifest_scene_references() {
        let fixture = Fixture::new();
        let target = fixture.create_material("Referenced");
        let mut scene = engine_scene::Scene::load_from_file(
            &fixture.root().join("assets/scenes/main.scene.ron"),
        )
        .expect("load scene");
        scene.scene_settings.environment_map = Some(target.asset_id.clone());
        let source = fixture.root().join("assets/source/referencing.scene.ron");
        scene.save_to_file(&source).expect("save source scene");
        fixture.declare_asset(
            "referencing-scene",
            AssetType::Scene,
            "referencing.scene.ron",
            &std::fs::read(source).expect("read source scene"),
        );

        let error = delete_project_asset(fixture.root(), &target.asset_id)
            .expect_err("source scene dependency must block delete");

        assert!(error.contains("source-scene:referencing-scene"));
        assert!(target.source_path.is_file());
    }

    #[test]
    fn delete_refuses_prefab_hierarchy_default_and_child_references() {
        let fixture = Fixture::new();
        let target = fixture.create_material("Referenced");
        let source = prefab_source(
            "referencing-prefab",
            &target.asset_id.id,
            &target.asset_id.id,
            &target.asset_id.id,
        );
        fixture.declare_asset(
            "referencing-prefab",
            AssetType::Prefab,
            "referencing-prefab.prefab.ron",
            source.as_bytes(),
        );

        let error = delete_project_asset(fixture.root(), &target.asset_id)
            .expect_err("prefab dependency must block delete");

        assert!(error.contains("prefab:referencing-prefab"));
        assert!(target.source_path.is_file());
    }

    #[test]
    fn delete_refuses_logic_property_default_and_condition_references() {
        let fixture = Fixture::new();
        let target = fixture.create_material("Referenced");
        fixture.declare_asset(
            "referencing-logic",
            AssetType::Logic,
            "referencing-logic.logic.json",
            &logic_source(
                "referencing-logic",
                &target.asset_id.id,
                &target.asset_id.id,
                &target.asset_id.id,
            ),
        );

        let error = delete_project_asset(fixture.root(), &target.asset_id)
            .expect_err("logic dependency must block delete");

        assert!(error.contains("logic:referencing-logic"));
        assert!(target.source_path.is_file());
    }

    #[test]
    fn delete_fails_closed_when_a_dependency_source_cannot_be_parsed() {
        let fixture = Fixture::new();
        let target = fixture.create_material("Keep Safe");
        fixture.declare_asset(
            "broken-prefab",
            AssetType::Prefab,
            "broken.prefab.ron",
            b"(not valid prefab source)",
        );

        let error = delete_project_asset(fixture.root(), &target.asset_id)
            .expect_err("uninspectable dependency source must block delete");

        assert!(error.contains("could not inspect prefab 'broken-prefab'"));
        assert!(target.source_path.is_file());
        assert!(target.cooked_path.is_file());
    }

    fn walk_files(root: &Path) -> Vec<PathBuf> {
        let mut files = Vec::new();
        let mut pending = vec![root.to_path_buf()];
        while let Some(directory) = pending.pop() {
            for entry in std::fs::read_dir(directory).expect("enumerate trash") {
                let path = entry.expect("trash entry").path();
                if path.is_dir() {
                    pending.push(path);
                } else {
                    files.push(path);
                }
            }
        }
        files
    }
}
