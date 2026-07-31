use super::*;

pub(super) fn cook_staged_asset(
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

pub(super) fn next_material_identity(
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

pub(super) fn next_duplicate_identity(
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

pub(super) fn portable_slug(requested_name: &str) -> Result<String, String> {
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

pub(super) fn rewrite_duplicated_source_identity(
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
            crate::project_cli::atomic_write_bytes(duplicated_source, source.as_bytes())
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
            crate::project_cli::atomic_write_bytes(duplicated_source, &source)
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

pub(super) fn collect_scene_asset_dependencies(
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
        LogicCondition::HasAsset { asset } => {
            dependencies.insert(asset.clone());
        }
        LogicCondition::Always | LogicCondition::Never | LogicCondition::BoolParam(_) => {}
    }
}

pub(super) fn source_asset_dependencies(
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
