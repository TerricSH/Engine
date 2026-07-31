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
            crate::project_cli::atomic_write_bytes(path, updated)?;
        }
        ManifestCatalog::load(&project.asset_source)?;
        Ok(())
    })();
    if let Err(error) = update_result {
        let mut rollback_errors = Vec::new();
        for (path, original, _) in manifest_backups.iter().rev() {
            if let Err(rollback_error) = crate::project_cli::atomic_write_bytes(path, original) {
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
use super::*;
