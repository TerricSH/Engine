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
    crate::project_cli::atomic_write_bytes(&staged_source, source_json.as_bytes())?;

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

pub(super) fn move_project_asset_impl(
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

pub(super) fn delete_project_asset_impl(
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
use super::*;
