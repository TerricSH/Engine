use super::*;

pub(crate) fn check_project(path: &Path, report_path: Option<&Path>) -> Result<(), String> {
    let project = GameProject::load(path).map_err(|error| error.to_string())?;
    let input_map = crate::project_input::load_project_input_map(&project)?;
    let input_binding_count = input_map
        .actions
        .iter()
        .map(|action| action.bindings.len())
        .sum::<usize>();
    let mut loaded_scenes = Vec::new();
    let mut scene_entities = BTreeMap::new();
    let mut total_entities = 0usize;
    let mut script_assembly = None;
    let mut script_components = 0usize;
    let mut strict_runtime_builder =
        engine_core::EngineRuntime::builder(engine_core::EngineConfig {
            application_name: format!("{}-project-check", project.manifest.name),
            gpu_timestamps: true,
        });
    engine_animation::loader::register_asset_types(
        strict_runtime_builder.asset_type_registry_mut(),
    );
    let mut strict_runtime = strict_runtime_builder.build();
    for (scene_id, scene_path) in project.scenes() {
        let scene = Scene::load_from_file(&scene_path).map_err(|error| {
            format!(
                "could not load project scene '{scene_id}' from {}: {error}",
                scene_path.display()
            )
        })?;
        let errors = validate_scene(&scene)
            .into_iter()
            .filter(|diagnostic| {
                matches!(
                    diagnostic.severity,
                    DiagnosticSeverity::Error | DiagnosticSeverity::Fatal
                )
            })
            .map(|diagnostic| format!("{}: {}", diagnostic.code, diagnostic.message))
            .collect::<Vec<_>>();
        if !errors.is_empty() {
            return Err(format!(
                "scene '{scene_id}' validation failed:\n{}",
                errors.join("\n")
            ));
        }
        total_entities += scene.entities.len();
        scene_entities.insert(scene_id.clone(), scene.entities.len());
        loaded_scenes.push((scene_id, scene_path, scene));
    }

    // The optional world partition manifest is validated against the scene
    // catalog when present, including the streaming compatibility rules the
    // runtime driver enforces (no scripts in cells, persistent entity IDs
    // unique across cells, no ID overlap with the startup scene unless the
    // cell references the startup scene itself).
    let partition = engine_asset::partition::WorldPartition::load_for_project(&project)
        .map_err(|error| format!("world partition validation failed: {error}"))?;
    if let Some(partition) = &partition {
        let scene_refs = loaded_scenes
            .iter()
            .map(|(scene_id, _, scene)| (scene_id.clone(), scene))
            .collect::<BTreeMap<String, &Scene>>();
        engine_core::cell_stream::validate_partition_cell_scenes(
            partition,
            project.startup_scene_id(),
            &scene_refs,
        )
        .map_err(|error| format!("world partition validation failed: {error}"))?;
    }
    let partition_cells = partition
        .map(|partition| partition.cells.len())
        .unwrap_or(0);

    for (scene_id, _, scene) in &loaded_scenes {
        let inspection = crate::project_scripts::inspect_project_scripts(&project, scene)
            .map_err(|error| format!("scene '{scene_id}' script validation failed: {error}"))?;
        script_assembly = inspection.assembly_id;
        script_components += inspection.component_count;

        let mut ecs_scene = (*scene).clone();
        for entity in &mut ecs_scene.entities {
            entity.components.remove("engine.script");
        }
        strict_runtime
            .load_scene(ecs_scene)
            .map_err(|diagnostics| {
                let messages = diagnostics
                    .into_iter()
                    .map(|diagnostic| format!("{}: {}", diagnostic.code, diagnostic.message))
                    .collect::<Vec<_>>()
                    .join("\n");
                format!("scene '{scene_id}' could not be restored into an ECS World:\n{messages}")
            })?;
    }

    let manifest_paths = source_manifest_paths(&project.asset_source)?;
    if manifest_paths.is_empty() {
        return Err(format!(
            "no .manifest files found in {}",
            project.asset_source.display()
        ));
    }

    let mut asset_ids = BTreeSet::new();
    let mut portable_asset_ids = BTreeSet::new();
    let mut declared_asset_types = BTreeMap::new();
    let mut prefab_sources: Vec<(String, String)> = Vec::new();
    let mut declared_asset_count = 0usize;
    for manifest_path in &manifest_paths {
        let content = std::fs::read_to_string(manifest_path)
            .map_err(|error| format!("could not read {}: {error}", manifest_path.display()))?;
        let manifest: SourceManifest = serde_json::from_str(&content)
            .map_err(|error| format!("invalid {}: {error}", manifest_path.display()))?;
        if manifest.schema_version != CURRENT_MANIFEST_VERSION {
            return Err(format!(
                "unsupported source manifest schema in {}",
                manifest_path.display()
            ));
        }
        for asset in manifest.assets {
            let portable_id = asset.id.id.to_ascii_lowercase();
            if asset.id.id.trim().is_empty() || !portable_asset_ids.insert(portable_id) {
                return Err(format!(
                    "empty, duplicate, or case-conflicting asset id '{}' in {}",
                    asset.id.id,
                    manifest_path.display()
                ));
            }
            asset_ids.insert(asset.id.id.clone());
            validate_source_path(&project.asset_source, &asset.source_path)?;
            validate_project_asset_type(&asset.asset_type, strict_runtime.asset_type_registry())
                .map_err(|error| {
                    format!(
                        "asset '{}' in {} cannot be cooked and loaded: {error}",
                        asset.id.id,
                        manifest_path.display()
                    )
                })?;
            if asset.asset_type == AssetType::Prefab {
                prefab_sources.push((asset.id.id.clone(), asset.source_path.clone()));
            }
            declared_asset_types.insert(asset.id.id, asset.asset_type);
            declared_asset_count += 1;
        }
    }

    let cooked_report =
        validate_existing_cooked_assets(&project, &declared_asset_types, &mut strict_runtime)?;

    let builtins = BTreeSet::from(["mesh-cube".to_string(), "mat-default".to_string()]);
    let prefab_count = validate_project_prefabs(
        &project,
        &prefab_sources,
        &asset_ids,
        &declared_asset_types,
        &builtins,
    )?;
    let mut all_scene_dependencies = BTreeSet::new();
    for (scene_id, _, scene) in &loaded_scenes {
        let scene_dependencies = scene
            .collect_asset_dependencies()
            .into_iter()
            .chain(scene.dependencies.iter().cloned())
            .collect::<BTreeSet<_>>();
        let missing = scene_dependencies
            .iter()
            .filter(|dependency| {
                !asset_ids.contains(&dependency.id) && !builtins.contains(&dependency.id)
            })
            .map(|dependency| dependency.id.clone())
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            return Err(format!(
                "scene '{scene_id}' references undeclared assets: {}",
                missing.join(", ")
            ));
        }
        all_scene_dependencies.extend(scene_dependencies);
    }

    let report = serde_json::to_string_pretty(&serde_json::json!({
        "schema": "ProjectCheckReport-v0",
        "project": project.manifest.name,
        "root": absolute_for_report(&project.root),
        "startup_scene_id": project.startup_scene_id(),
        "startup_scene": absolute_for_report(project.startup_scene_path()),
        "scenes": loaded_scenes.len(),
        "scene_entities": scene_entities,
        "entities": total_entities,
        "partition_cells": partition_cells,
        "source_manifests": manifest_paths.len(),
        "declared_assets": declared_asset_count,
        "prefabs": prefab_count,
        "cooked_assets": cooked_report.discovered_assets,
        "loaded_render_assets": cooked_report.loaded_render_assets(),
        "loaded_extension_assets": cooked_report.loaded_extension_assets,
        "skipped_cooked_assets": cooked_report.skipped_assets,
        "scene_asset_dependencies": all_scene_dependencies.len(),
        "input_actions": input_map.actions.len(),
        "input_bindings": input_binding_count,
        "script_assembly": script_assembly,
        "script_components": script_components,
        "passed": true
    }))
    .expect("JSON value serialization cannot fail");
    emit_report(&report, report_path)?;
    Ok(())
}

/// Validate every prefab declared in the project's source manifests.
///
/// Each `.prefab.ron` source must parse and pass structural validation, every
/// `Value::Asset` referenced from component fields (including component
/// default overrides) must be a declared asset or an engine builtin, every
/// nested child prefab reference must point at another declared `Prefab`
/// asset, and the full nested graph must be free of missing children and
/// cycles. Returns the number of validated prefabs for the check report.
pub(crate) fn validate_project_prefabs(
    project: &GameProject,
    prefab_sources: &[(String, String)],
    asset_ids: &BTreeSet<String>,
    declared_asset_types: &BTreeMap<String, AssetType>,
    builtins: &BTreeSet<String>,
) -> Result<usize, String> {
    let mut parsed = Vec::with_capacity(prefab_sources.len());
    for (id, source_path) in prefab_sources {
        let path = project.asset_source.join(source_path);
        let bytes = std::fs::read(&path).map_err(|error| {
            format!(
                "could not read prefab source '{}' declared as '{id}': {error}",
                path.display()
            )
        })?;
        let prefab = engine_scene::parse_prefab_source(&bytes)
            .map_err(|error| format!("prefab '{id}' source is invalid: {error}"))?;
        parsed.push((id.clone(), prefab));
    }

    let mut registry = engine_scene::PrefabRegistry::new();
    for (id, prefab) in &parsed {
        let mut dependencies = BTreeSet::new();
        for entity in &prefab.hierarchy {
            for component in entity.components.values() {
                for value in component.fields.values() {
                    collect_prefab_value_asset_dependencies(value, &mut dependencies);
                }
            }
        }
        for defaults in prefab.component_defaults.values() {
            for value in defaults.values() {
                collect_prefab_value_asset_dependencies(value, &mut dependencies);
            }
        }
        let missing = dependencies
            .iter()
            .filter(|dependency| {
                !asset_ids.contains(&dependency.id) && !builtins.contains(&dependency.id)
            })
            .map(|dependency| dependency.id.clone())
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            return Err(format!(
                "prefab '{id}' references undeclared assets: {}",
                missing.join(", ")
            ));
        }

        for child_ref in &prefab.child_prefab_refs {
            let child_id = &child_ref.prefab_asset.id;
            match declared_asset_types.get(child_id) {
                Some(AssetType::Prefab) => {}
                Some(other) => {
                    return Err(format!(
                        "prefab '{id}' references nested prefab '{child_id}', but that asset is declared as {other:?}"
                    ));
                }
                None => {
                    return Err(format!(
                        "prefab '{id}' references undeclared nested prefab '{child_id}'"
                    ));
                }
            }
        }
        registry.register(id.clone(), prefab.clone());
    }

    for (id, prefab) in &parsed {
        engine_scene::validate_prefab(prefab, &registry).map_err(|errors| {
            let messages = errors
                .iter()
                .map(|error| error.to_string())
                .collect::<Vec<_>>()
                .join("; ");
            format!("prefab '{id}' failed graph validation: {messages}")
        })?;
    }
    Ok(parsed.len())
}

/// Recursive companion to `Scene::collect_asset_dependencies` for prefab
/// component fields: collects every `Value::Asset`, including assets nested
/// in `Value::List` and `Value::Map`.
pub(crate) fn collect_prefab_value_asset_dependencies(
    value: &engine_serialize::Value,
    dependencies: &mut BTreeSet<AssetId>,
) {
    match value {
        engine_serialize::Value::Asset(asset) => {
            dependencies.insert(asset.clone());
        }
        engine_serialize::Value::List(values) => {
            for value in values {
                collect_prefab_value_asset_dependencies(value, dependencies);
            }
        }
        engine_serialize::Value::Map(values) => {
            for value in values.values() {
                collect_prefab_value_asset_dependencies(value, dependencies);
            }
        }
        _ => {}
    }
}

pub(crate) fn validate_project_asset_type(
    asset_type: &AssetType,
    registry: &engine_scene::registry::AssetTypeRegistry,
) -> Result<(), String> {
    match asset_type {
        AssetType::Mesh
        | AssetType::Texture
        | AssetType::Shader
        | AssetType::Scene
        | AssetType::Material
        | AssetType::EnvironmentMap
        | AssetType::MorphTargetSet
        | AssetType::Logic => Ok(()),
        _ => {
            let type_id = registered_asset_type_id(asset_type).ok_or_else(|| {
                format!("asset type {asset_type:?} has no supported project pipeline mapping")
            })?;
            let extension = registry.get(type_id).ok_or_else(|| {
                format!("required runtime extension '{type_id}' is not registered")
            })?;
            if extension.cooker.is_none() {
                return Err(format!(
                    "runtime extension '{type_id}' does not provide a cooker"
                ));
            }
            if extension.loader.is_none() {
                return Err(format!(
                    "runtime extension '{type_id}' does not provide a loader"
                ));
            }
            Ok(())
        }
    }
}

pub(crate) fn validate_existing_cooked_assets(
    project: &GameProject,
    declared_asset_types: &BTreeMap<String, AssetType>,
    runtime: &mut engine_core::EngineRuntime,
) -> Result<engine_core::CookedAssetLoadReport, String> {
    if !project.cooked_assets.exists() {
        return Ok(engine_core::CookedAssetLoadReport::default());
    }
    if !project.cooked_assets.is_dir() {
        return Err(format!(
            "configured cooked asset path is not a directory: {}",
            project.cooked_assets.display()
        ));
    }

    let mut paths = Vec::new();
    for entry in std::fs::read_dir(&project.cooked_assets).map_err(|error| {
        format!(
            "could not enumerate {}: {error}",
            project.cooked_assets.display()
        )
    })? {
        let path = entry
            .map_err(|error| {
                format!(
                    "could not enumerate {}: {error}",
                    project.cooked_assets.display()
                )
            })?
            .path();
        if path.is_file()
            && path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("cooked"))
        {
            paths.push(path);
        }
    }
    paths.sort();

    for path in &paths {
        let id = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .filter(|stem| !stem.is_empty())
            .ok_or_else(|| format!("cooked asset has no portable UTF-8 ID: {}", path.display()))?;
        let declared_type = declared_asset_types.get(id).ok_or_else(|| {
            format!(
                "cooked artifact '{}' is not declared by any source manifest",
                path.display()
            )
        })?;
        let artifact = read_cooked_artifact(path)
            .map_err(|error| format!("invalid cooked artifact {}: {error}", path.display()))?;
        if artifact.header.asset_kind != declared_type.kind_code() {
            return Err(format!(
                "cooked artifact '{}' has kind {}, but its manifest declares {:?} (kind {})",
                path.display(),
                artifact.header.asset_kind,
                declared_type,
                declared_type.kind_code()
            ));
        }
    }

    runtime
        .load_cooked_assets(&project.cooked_assets)
        .map_err(|diagnostics| {
            let details = diagnostics
                .into_iter()
                .map(|diagnostic| {
                    format!(
                        "{}: {}{}",
                        diagnostic.code,
                        diagnostic.message,
                        diagnostic
                            .path
                            .as_deref()
                            .map(|path| format!(" ({path})"))
                            .unwrap_or_default()
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            format!("cooked asset validation failed:\n{details}")
        })
}
