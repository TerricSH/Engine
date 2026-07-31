use super::*;

pub(crate) fn create_project_scene(
    project_path: &Path,
    scene_id: &str,
    requested_name: Option<&str>,
) -> Result<PathBuf, String> {
    create_project_scene_from(project_path, scene_id, requested_name, None, None)
}

#[cfg(any(feature = "tooling-editor", test))]
pub(crate) fn create_project_scene_in_folder(
    project_path: &Path,
    scene_id: &str,
    requested_name: Option<&str>,
    relative_folder: &Path,
) -> Result<PathBuf, String> {
    create_project_scene_from(
        project_path,
        scene_id,
        requested_name,
        None,
        Some(relative_folder),
    )
}

#[cfg(any(feature = "tooling-editor", test))]
pub(crate) fn duplicate_project_scene(
    project_path: &Path,
    scene_id: &str,
    source: &Scene,
) -> Result<PathBuf, String> {
    create_project_scene_from(
        project_path,
        scene_id,
        Some(source.name.as_str()),
        Some(source),
        None,
    )
}

pub(crate) fn create_project_scene_from(
    project_path: &Path,
    scene_id: &str,
    requested_name: Option<&str>,
    source: Option<&Scene>,
    relative_folder: Option<&Path>,
) -> Result<PathBuf, String> {
    let (_operation_guard, project) = lock_project_scene_operations(project_path)?;
    validate_portable_scene_id(scene_id)?;
    if project
        .scenes()
        .iter()
        .any(|(existing, _)| existing.eq_ignore_ascii_case(scene_id))
    {
        return Err(format!("project scene ID already exists: '{scene_id}'"));
    }

    let relative_folder = portable_scene_subdirectory(relative_folder.unwrap_or(Path::new("")))?;
    let relative_path = if relative_folder.as_os_str().is_empty() {
        PathBuf::from(format!("assets/scenes/{scene_id}.scene.ron"))
    } else {
        PathBuf::from(format!(
            "assets/scenes/{}/{scene_id}.scene.ron",
            portable_path_string(&relative_folder)?
        ))
    };
    let mut manifest = project.manifest.clone();
    // Mutating a legacy project upgrades it to an explicit catalog without
    // changing the stable `main` ID synthesized by the loader.
    manifest.scenes = manifest.scene_catalog();
    manifest
        .scenes
        .insert(scene_id.to_string(), relative_path.clone());
    manifest
        .validate()
        .map_err(|error| format!("invalid scene catalog update: {error}"))?;

    let target = project.root.join(&relative_path);
    let file_name = target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("scene path has no portable file name: {}", target.display()))?;
    let parent = target
        .parent()
        .ok_or_else(|| format!("scene path has no parent: {}", target.display()))?;
    ensure_scene_directory_chain(&project.root, parent, true)?;
    if let Some(conflict) = find_case_insensitive_entry(parent, file_name)? {
        return Err(format!(
            "scene file already exists and will not be overwritten: {}",
            conflict.display()
        ));
    }

    let display_name = requested_name
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(scene_id);
    let mut scene = source
        .cloned()
        .unwrap_or_else(|| engine_scene::starter_scene(scene_id, display_name));
    scene.scene_id = scene_id.to_string();
    scene.name = display_name.to_string();
    commit_scene_transaction(
        &project.root,
        vec![
            SceneTransactionWrite::create(target.clone(), serialize_scene(&scene)?),
            SceneTransactionWrite::replace(
                project.manifest_path.clone(),
                serialize_project_manifest(&manifest)?,
            ),
        ],
        Vec::new(),
        None,
    )?;
    Ok(target)
}

/// Result of moving a project scene into the recoverable editor trash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DeletedProjectScene {
    pub scene_id: String,
    pub trash_directory: PathBuf,
    pub metadata_path: PathBuf,
    pub replacement_startup: Option<String>,
}

/// Rename a cataloged project scene and its authoring file as one transaction.
///
/// The serialized scene ID always follows the catalog ID. A display name that
/// was generated from the previous catalog/serialized ID follows the rename;
/// an explicitly-authored display name remains unchanged.
pub(crate) fn rename_project_scene(
    project_path: &Path,
    old_id: &str,
    new_id: &str,
) -> Result<PathBuf, String> {
    rename_project_scene_impl(project_path, old_id, new_id, None)
}

pub(crate) fn rename_project_scene_impl(
    project_path: &Path,
    old_id: &str,
    new_id: &str,
    fail_after_mutation: Option<usize>,
) -> Result<PathBuf, String> {
    let (_operation_guard, project) = lock_project_scene_operations(project_path)?;
    if old_id == new_id {
        return Err(format!("project scene already has ID '{old_id}'"));
    }
    if old_id.eq_ignore_ascii_case(new_id) {
        return Err(format!(
            "project scene IDs cannot be renamed only by case: '{old_id}' -> '{new_id}'"
        ));
    }
    validate_portable_scene_id(new_id)?;

    let catalog = project.manifest.scene_catalog();
    let old_relative = exact_scene_catalog_path(&catalog, old_id)?.clone();
    if let Some((conflicting_id, _)) = catalog
        .iter()
        .find(|(existing, _)| existing.eq_ignore_ascii_case(new_id))
    {
        return Err(format!(
            "project scene ID '{new_id}' collides with existing scene '{conflicting_id}'"
        ));
    }
    let old_path = project.root.join(&old_relative);
    ensure_scene_file_is_regular(&project.root, &old_path)?;

    let scene_directory = old_relative.parent().ok_or_else(|| {
        format!(
            "cataloged scene path has no parent directory: {}",
            old_relative.display()
        )
    })?;
    let desired_relative = scene_directory.join(format!("{new_id}.scene.ron"));
    let same_portable_path =
        portable_scene_path_key(&old_relative) == portable_scene_path_key(&desired_relative);
    let new_relative = if same_portable_path {
        old_relative.clone()
    } else {
        desired_relative
    };
    let target = project.root.join(&new_relative);
    let target_parent = target
        .parent()
        .ok_or_else(|| format!("scene path has no parent: {}", target.display()))?;
    ensure_scene_directory_chain(&project.root, target_parent, true)?;
    if !same_portable_path {
        let file_name = target
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| format!("scene path has no portable file name: {}", target.display()))?;
        if let Some(conflict) = find_case_insensitive_entry(target_parent, file_name)? {
            return Err(format!(
                "renamed scene file already exists or differs only by case: {}",
                conflict.display()
            ));
        }
    }

    let mut scene = Scene::load_from_file(&old_path).map_err(|error| {
        format!(
            "could not load project scene '{old_id}' from {}: {error}",
            old_path.display()
        )
    })?;
    let display_name_follows_identity =
        scene.name.trim().is_empty() || scene.name == old_id || scene.name == scene.scene_id;
    scene.scene_id = new_id.to_string();
    if display_name_follows_identity {
        scene.name = new_id.to_string();
    }

    let mut manifest = project.manifest.clone();
    manifest.scenes = catalog;
    manifest
        .scenes
        .remove(old_id)
        .expect("the exact scene catalog entry was located above");
    manifest
        .scenes
        .insert(new_id.to_string(), new_relative.clone());
    if project.startup_scene_id() == old_id {
        manifest.startup_scene = PathBuf::from(new_id);
    }
    let manifest_bytes = serialize_project_manifest(&manifest)?;
    let scene_bytes = serialize_scene(&scene)?;

    let (writes, deletes) = if same_portable_path {
        (
            vec![
                SceneTransactionWrite::replace(old_path.clone(), scene_bytes),
                SceneTransactionWrite::replace(project.manifest_path.clone(), manifest_bytes),
            ],
            Vec::new(),
        )
    } else {
        (
            vec![
                SceneTransactionWrite::create(target.clone(), scene_bytes),
                SceneTransactionWrite::replace(project.manifest_path.clone(), manifest_bytes),
            ],
            vec![old_path],
        )
    };
    commit_scene_transaction(&project.root, writes, deletes, fail_after_mutation)?;
    Ok(target)
}

/// Move a scene into `.engine/trash/scenes` and remove its catalog entry.
///
/// The final project scene cannot be deleted. Deleting the startup scene also
/// requires an explicit, existing replacement ID; no implicit selection is
/// made from catalog order.
pub(crate) fn delete_project_scene(
    project_path: &Path,
    scene_id: &str,
    replacement_startup: Option<&str>,
) -> Result<DeletedProjectScene, String> {
    delete_project_scene_impl(project_path, scene_id, replacement_startup, None)
}

pub(crate) fn delete_project_scene_impl(
    project_path: &Path,
    scene_id: &str,
    replacement_startup: Option<&str>,
    fail_after_mutation: Option<usize>,
) -> Result<DeletedProjectScene, String> {
    let (_operation_guard, project) = lock_project_scene_operations(project_path)?;
    let catalog = project.manifest.scene_catalog();
    if catalog.len() <= 1 {
        return Err("a project must retain at least one scene".into());
    }
    let old_relative = exact_scene_catalog_path(&catalog, scene_id)?.clone();
    let old_path = project.root.join(&old_relative);
    ensure_scene_file_is_regular(&project.root, &old_path)?;
    let scene = Scene::load_from_file(&old_path).map_err(|error| {
        format!(
            "could not load project scene '{scene_id}' from {}: {error}",
            old_path.display()
        )
    })?;

    let deleting_startup = project.startup_scene_id() == scene_id;
    let replacement_startup = if deleting_startup {
        let replacement = replacement_startup.ok_or_else(|| {
            format!(
                "deleting startup scene '{scene_id}' requires an explicit replacement startup scene"
            )
        })?;
        if replacement == scene_id || replacement.eq_ignore_ascii_case(scene_id) {
            return Err("the deleted scene cannot replace itself as the startup scene".into());
        }
        exact_scene_catalog_path(&catalog, replacement)?;
        Some(replacement.to_string())
    } else {
        None
    };

    let mut manifest = project.manifest.clone();
    manifest.scenes = catalog;
    manifest
        .scenes
        .remove(scene_id)
        .expect("the exact scene catalog entry was located above");
    if let Some(replacement) = &replacement_startup {
        manifest.startup_scene = PathBuf::from(replacement);
    }
    let manifest_bytes = serialize_project_manifest(&manifest)?;

    let deleted_unix_nanos = unix_nanos()?;
    let trash_directory = allocate_scene_trash_directory(&project, scene_id, deleted_unix_nanos)?;
    let trash_scene_path = trash_directory.join("scene.scene.ron");
    let metadata_path = trash_directory.join("metadata.json");
    let metadata = SceneTrashMetadata {
        schema: SCENE_TRASH_SCHEMA.to_string(),
        deleted_unix_nanos,
        scene_id: scene_id.to_string(),
        scene_name: scene.name,
        original_scene_path: portable_project_relative_path(&project.root, &old_path)?,
        original_startup_scene: portable_path_string(&project.manifest.startup_scene)?,
        was_startup: deleting_startup,
        replacement_startup: replacement_startup.clone(),
    };
    let mut metadata_bytes = serde_json::to_vec_pretty(&metadata)
        .map_err(|error| format!("could not serialize scene trash metadata: {error}"))?;
    metadata_bytes.push(b'\n');
    let scene_bytes = std::fs::read(&old_path)
        .map_err(|error| format!("could not read {}: {error}", old_path.display()))?;
    std::fs::create_dir(&trash_directory).map_err(|error| {
        format!(
            "could not create scene trash directory {}: {error}",
            trash_directory.display()
        )
    })?;

    let result = commit_scene_transaction(
        &project.root,
        vec![
            SceneTransactionWrite::create(trash_scene_path, scene_bytes),
            SceneTransactionWrite::create(metadata_path.clone(), metadata_bytes),
            SceneTransactionWrite::replace(project.manifest_path.clone(), manifest_bytes),
        ],
        vec![old_path],
        fail_after_mutation,
    );
    if let Err(error) = result {
        let cleanup_error = match std::fs::remove_dir(&trash_directory) {
            Ok(()) => None,
            Err(cleanup_error) if cleanup_error.kind() == std::io::ErrorKind::NotFound => None,
            Err(cleanup_error) => Some(cleanup_error),
        };
        return match cleanup_error {
            None => Err(error),
            Some(cleanup_error) => Err(format!(
                "{error}\nscene trash rollback could not remove {}: {cleanup_error}",
                trash_directory.display()
            )),
        };
    }

    Ok(DeletedProjectScene {
        scene_id: scene_id.to_string(),
        trash_directory,
        metadata_path,
        replacement_startup,
    })
}

pub(crate) fn set_project_startup_scene(
    project_path: &Path,
    scene_id: &str,
) -> Result<PathBuf, String> {
    let (_operation_guard, project) = lock_project_scene_operations(project_path)?;
    let catalog = project.manifest.scene_catalog();
    let relative = exact_scene_catalog_path(&catalog, scene_id)?;
    let scene_path = project.root.join(relative);
    let mut manifest = project.manifest.clone();
    manifest.scenes = catalog;
    manifest.startup_scene = PathBuf::from(scene_id);
    atomic_write_project_manifest(&manifest, &project.manifest_path)?;
    Ok(scene_path)
}

mod catalog;
mod transaction;

pub(crate) use catalog::*;
pub(crate) use transaction::*;
