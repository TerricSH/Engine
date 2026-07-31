use super::*;

pub(super) fn selected_material_asset(
    scene: &Scene,
    selected: Option<&PersistentId>,
) -> Option<String> {
    let selected = selected?;
    let entity = scene
        .entities
        .iter()
        .find(|entity| &entity.persistent_id == selected)?;
    let renderable = entity.components.get("engine.renderable")?;
    match renderable.fields.get("material")? {
        Value::Asset(asset) => Some(asset.id.clone()),
        Value::Str(asset) => Some(asset.clone()),
        _ => None,
    }
}

pub(super) fn assign_material_to_selected_command(
    editor_scene: &EditorScene,
    material_asset: &str,
) -> Result<Box<dyn engine_editor::Command>, String> {
    let material_asset = material_asset.trim();
    if material_asset.is_empty() {
        return Err("No material is open in the Material Editor".to_string());
    }
    let entity_id = editor_scene
        .selected_entity
        .as_ref()
        .ok_or_else(|| "Select an entity with a Mesh Renderer component".to_string())?;
    let entity = editor_scene
        .scene
        .entities
        .iter()
        .find(|entity| &entity.persistent_id == entity_id)
        .ok_or_else(|| format!("Selected entity '{entity_id}' is no longer in the scene"))?;
    let renderable = entity
        .components
        .get("engine.renderable")
        .ok_or_else(|| format!("Entity '{entity_id}' does not have a Mesh Renderer component"))?;
    if !renderable.fields.contains_key("material") {
        return Err(format!(
            "Entity '{entity_id}' has no writable Mesh Renderer material field"
        ));
    }

    Ok(Box::new(engine_editor::SetComponentField::new(
        entity_id.clone(),
        "engine.renderable".to_string(),
        "material".to_string(),
        Value::Asset(AssetId::new(material_asset)),
    )))
}

#[derive(Clone, Debug)]
pub(super) struct ProjectMaterialSource {
    pub(super) source_path: PathBuf,
}

#[derive(Clone, Debug)]
pub(super) struct MaterialSaveOutcome {
    pub(super) source_path: PathBuf,
    pub(super) cooked_path: PathBuf,
}

pub(super) fn project_material_save_access(
    project: &GameProject,
    material_id: &str,
) -> MaterialSaveAccess {
    match resolve_project_material_source(project, material_id) {
        Ok(_) => MaterialSaveAccess::Writable,
        Err(reason) => MaterialSaveAccess::ReadOnly(reason),
    }
}

/// Resolve one exact project-owned material entry. Built-ins, unknown IDs,
/// duplicate declarations, non-material declarations, and unsafe source paths
/// are deliberately rejected before any file is written.
pub(super) fn resolve_project_material_source(
    project: &GameProject,
    material_id: &str,
) -> Result<ProjectMaterialSource, String> {
    if material_id == BUILTIN_DEFAULT_MATERIAL_ID {
        return Err(format!(
            "Built-in material '{BUILTIN_DEFAULT_MATERIAL_ID}' is read-only because it has no project source Material."
        ));
    }
    if material_id.trim().is_empty() {
        return Err("The selected material ID is empty and cannot be saved.".to_string());
    }

    let source_root = std::fs::canonicalize(&project.asset_source).map_err(|error| {
        format!(
            "Could not resolve project asset source {}: {error}",
            project.asset_source.display()
        )
    })?;
    if !source_root.is_dir() {
        return Err(format!(
            "Project asset source is not a directory: {}",
            source_root.display()
        ));
    }

    let mut manifest_paths = Vec::new();
    for entry in std::fs::read_dir(&source_root).map_err(|error| {
        format!(
            "Could not enumerate project source manifests in {}: {error}",
            source_root.display()
        )
    })? {
        let entry = entry.map_err(|error| {
            format!(
                "Could not enumerate project source manifests in {}: {error}",
                source_root.display()
            )
        })?;
        let path = entry.path();
        if path.is_file()
            && path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("manifest"))
        {
            manifest_paths.push(path);
        }
    }
    manifest_paths.sort();

    let mut matches: Vec<(PathBuf, SourceAssetEntry)> = Vec::new();
    for manifest_path in manifest_paths {
        let bytes = std::fs::read(&manifest_path).map_err(|error| {
            format!(
                "Could not read source manifest {}: {error}",
                manifest_path.display()
            )
        })?;
        let manifest: SourceManifest = serde_json::from_slice(&bytes).map_err(|error| {
            format!(
                "Invalid source manifest {}: {error}",
                manifest_path.display()
            )
        })?;
        if manifest.schema_version != CURRENT_MANIFEST_VERSION {
            return Err(format!(
                "Source manifest {} uses unsupported schema {}.{}.{}.",
                manifest_path.display(),
                manifest.schema_version.major,
                manifest.schema_version.minor,
                manifest.schema_version.patch
            ));
        }
        matches.extend(
            manifest
                .assets
                .into_iter()
                .filter(|entry| entry.id.id == material_id)
                .map(|entry| (manifest_path.clone(), entry)),
        );
    }

    if matches.is_empty() {
        return Err(format!(
            "Material '{material_id}' is not declared by a project source manifest and is read-only."
        ));
    }
    if matches.len() != 1 {
        let manifests = matches
            .iter()
            .map(|(path, _)| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "Material ID '{material_id}' is ambiguous because it is declared {} times: {manifests}",
            matches.len()
        ));
    }

    let (manifest_path, entry) = matches.pop().expect("one match was checked above");
    if entry.asset_type != AssetType::Material {
        return Err(format!(
            "Asset '{material_id}' in {} is {:?}, not a project source Material.",
            manifest_path.display(),
            entry.asset_type
        ));
    }

    let relative_source = Path::new(&entry.source_path);
    if relative_source.as_os_str().is_empty()
        || !relative_source
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(format!(
            "Material '{material_id}' has an unsafe source_path '{}'; expected a non-empty relative path without '.' or '..'.",
            entry.source_path
        ));
    }
    let source_path =
        std::fs::canonicalize(source_root.join(relative_source)).map_err(|error| {
            format!(
                "Could not resolve source for material '{material_id}' at '{}': {error}",
                entry.source_path
            )
        })?;
    if !source_path.starts_with(&source_root) || !source_path.is_file() {
        return Err(format!(
            "Material '{material_id}' source resolves outside the project source directory or is not a regular file: {}",
            source_path.display()
        ));
    }

    Ok(ProjectMaterialSource { source_path })
}

pub(super) fn read_optional_file(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("Could not read {}: {error}", path.display())),
    }
}

/// Flush a same-directory temporary file before atomically replacing `path`.
/// `tempfile::persist` uses the platform's replace-existing primitive, so the
/// original remains reachable if replacement fails.
pub(super) fn atomic_write_file(path: &Path, contents: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("Path has no parent: {}", path.display()))?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("Could not create {}: {error}", parent.display()))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|error| {
        format!(
            "Could not create a temporary file beside {}: {error}",
            path.display()
        )
    })?;
    temporary
        .write_all(contents)
        .map_err(|error| format!("Could not write temporary {}: {error}", path.display()))?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| format!("Could not flush temporary {}: {error}", path.display()))?;
    let persisted = temporary.persist(path).map_err(|error| {
        format!(
            "Could not atomically replace {}: {}",
            path.display(),
            error.error
        )
    })?;
    persisted
        .sync_all()
        .map_err(|error| format!("Could not flush {}: {error}", path.display()))?;
    Ok(())
}

pub(super) fn save_scene_atomically(scene: &Scene, path: &Path) -> Result<(), String> {
    let serialized = ron::ser::to_string_pretty(scene, ron::ser::PrettyConfig::default())
        .map_err(|error| format!("Could not serialize scene '{}': {error}", scene.scene_id))?;
    atomic_write_file(path, serialized.as_bytes())
}

pub(super) fn checked_cook_material_bytes(
    project: &GameProject,
    material_id: &str,
) -> Result<Vec<u8>, String> {
    let staging_parent = project
        .cooked_assets
        .parent()
        .unwrap_or(project.root.as_path());
    if !staging_parent.starts_with(&project.root) {
        return Err(format!(
            "Cook staging directory would escape the project root: {}",
            staging_parent.display()
        ));
    }
    std::fs::create_dir_all(staging_parent).map_err(|error| {
        format!(
            "Could not create material cook staging parent {}: {error}",
            staging_parent.display()
        )
    })?;
    let staging = tempfile::Builder::new()
        .prefix(".material-save-cook-")
        .tempdir_in(staging_parent)
        .map_err(|error| format!("Could not create material cook staging directory: {error}"))?;
    let mut graph = DependencyGraph::new();
    let runtime_builder = EngineRuntime::builder(EngineConfig {
        application_name: format!("{}-material-cook", project.manifest.name),
        gpu_timestamps: true,
    });
    let report = cook_orchestrate_checked_with_registry(
        &project.asset_source,
        staging.path(),
        &mut graph,
        runtime_builder.asset_type_registry(),
    );
    if !report.is_success() {
        let details = report
            .diagnostics
            .iter()
            .filter(|diagnostic| {
                matches!(
                    diagnostic.severity,
                    DiagnosticSeverity::Error | DiagnosticSeverity::Fatal
                )
            })
            .take(8)
            .map(|diagnostic| format!("{}: {}", diagnostic.code, diagnostic.message))
            .collect::<Vec<_>>()
            .join("\n");
        return Err(if details.is_empty() {
            "Checked project asset cook failed without an error diagnostic.".to_string()
        } else {
            format!("Checked project asset cook failed:\n{details}")
        });
    }
    let result = report
        .results
        .iter()
        .find(|result| result.success && result.asset_id == material_id)
        .ok_or_else(|| {
            format!("Checked cook did not report material asset '{material_id}' as successful.")
        })?;
    if result.asset_type != AssetType::Material {
        return Err(format!(
            "Checked cook reported asset '{material_id}' as {:?}, not Material.",
            result.asset_type
        ));
    }
    let staging_root = std::fs::canonicalize(staging.path()).map_err(|error| {
        format!(
            "Could not resolve material cook staging directory {}: {error}",
            staging.path().display()
        )
    })?;
    let staged_path =
        std::fs::canonicalize(staging.path().join(&result.output_path)).map_err(|error| {
            format!(
                "Could not resolve staged material artifact {}: {error}",
                result.output_path.display()
            )
        })?;
    if !staged_path.starts_with(&staging_root) || !staged_path.is_file() {
        return Err(format!(
            "Checked cook produced an unsafe material artifact path: {}",
            staged_path.display()
        ));
    }
    let artifact = read_cooked_artifact(&staged_path).map_err(|error| {
        format!(
            "Staged material artifact {} failed header validation: {error}",
            staged_path.display()
        )
    })?;
    decode_cooked_material(&artifact).map_err(|error| {
        format!(
            "Staged material artifact {} failed payload validation: {error}",
            staged_path.display()
        )
    })?;
    std::fs::read(&staged_path).map_err(|error| {
        format!(
            "Could not read staged material artifact {}: {error}",
            staged_path.display()
        )
    })
}

pub(super) fn rollback_material_source(
    failure: String,
    source_path: &Path,
    original_source: &[u8],
    cooked_restore: Option<(&Path, Option<&[u8]>)>,
) -> String {
    let mut rollback_errors = Vec::new();
    if let Err(error) = atomic_write_file(source_path, original_source) {
        rollback_errors.push(format!("source restore failed: {error}"));
    }
    if let Some((cooked_path, original_cooked)) = cooked_restore {
        let restore = match original_cooked {
            Some(bytes) => atomic_write_file(cooked_path, bytes),
            None if cooked_path.exists() => std::fs::remove_file(cooked_path).map_err(|error| {
                format!(
                    "Could not remove new cooked artifact {}: {error}",
                    cooked_path.display()
                )
            }),
            None => Ok(()),
        };
        if let Err(error) = restore {
            rollback_errors.push(format!("cooked artifact restore failed: {error}"));
        }
    }

    if rollback_errors.is_empty() {
        format!("{failure}\nThe original material source was restored.")
    } else {
        format!(
            "{failure}\nMaterial save rollback also failed:\n{}",
            rollback_errors.join("\n")
        )
    }
}

pub(super) fn save_project_material(
    runtime: &mut EngineRuntime,
    project: &GameProject,
    request: &MaterialSaveRequest,
) -> Result<MaterialSaveOutcome, String> {
    let resolved = resolve_project_material_source(project, &request.material_asset)?;
    let original_source = std::fs::read(&resolved.source_path).map_err(|error| {
        format!(
            "Could not back up material source {}: {error}",
            resolved.source_path.display()
        )
    })?;
    let mut source_json = serde_json::to_vec_pretty(&request.source)
        .map_err(|error| format!("Could not serialize MaterialSource-v0: {error}"))?;
    source_json.push(b'\n');
    atomic_write_file(&resolved.source_path, &source_json)?;

    let cooked_bytes = match checked_cook_material_bytes(project, &request.material_asset) {
        Ok(bytes) => bytes,
        Err(error) => {
            return Err(rollback_material_source(
                error,
                &resolved.source_path,
                &original_source,
                None,
            ));
        }
    };

    let cooked_path = project
        .cooked_assets
        .join(format!("{}.cooked", request.material_asset));
    let original_cooked = match read_optional_file(&cooked_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            return Err(rollback_material_source(
                error,
                &resolved.source_path,
                &original_source,
                None,
            ));
        }
    };
    if let Err(error) = atomic_write_file(&cooked_path, &cooked_bytes) {
        return Err(rollback_material_source(
            error,
            &resolved.source_path,
            &original_source,
            None,
        ));
    }

    if let Err(error) = super::super::project_app::load_project_assets(runtime, project) {
        return Err(rollback_material_source(
            format!("Runtime registry refresh rejected the saved material: {error}"),
            &resolved.source_path,
            &original_source,
            Some((&cooked_path, original_cooked.as_deref())),
        ));
    }

    Ok(MaterialSaveOutcome {
        source_path: resolved.source_path,
        cooked_path,
    })
}
