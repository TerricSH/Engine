use super::*;

pub(crate) fn import_project_asset(request: &ProjectImportRequest) -> Result<(), String> {
    let project = GameProject::load(&request.project).map_err(|error| error.to_string())?;
    validate_import_asset_id(&request.asset_id)?;
    let import_folder = normalize_existing_import_folder(&project.asset_source, &request.folder)?;

    let source_file = std::fs::canonicalize(&request.source_file).map_err(|error| {
        format!(
            "could not resolve import source {}: {error}",
            request.source_file.display()
        )
    })?;
    if !source_file.is_file() {
        return Err(format!(
            "import source is not a regular file: {}",
            source_file.display()
        ));
    }
    let asset_type = resolve_import_asset_type(&source_file, request.asset_type.as_ref())?;
    let source_name = source_file
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| {
            format!(
                "import source has no portable UTF-8 file name: {}",
                source_file.display()
            )
        })?;
    let relative_source = import_folder.join(source_name);
    let copied_source = project.asset_source.join(&relative_source);
    let external_sources = if asset_type == AssetType::Mesh {
        gltf_external_source_files(&source_file)?
            .into_iter()
            .map(|(source, relative)| {
                let target = project.asset_source.join(import_folder.join(relative));
                (source, target)
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };

    let mut prepared_assets = Vec::<SourceAssetEntry>::new();
    let mut generated_sources = Vec::<(PathBuf, Vec<u8>)>::new();
    if asset_type == AssetType::Mesh {
        let scene = engine_asset::gltf::load_gltf_scene(&source_file)
            .map_err(|error| format!("could not inspect glTF import: {error}"))?;
        if scene.primitives.is_empty() {
            return Err("glTF import contains no mesh primitives".into());
        }
        for primitive_index in 0..scene.primitives.len() {
            let id = if primitive_index == 0 {
                request.asset_id.clone()
            } else {
                format!("{}.mesh.{primitive_index}", request.asset_id)
            };
            validate_import_asset_id(&id)?;
            let mut cook_rules = CookRules::default();
            if scene.primitives.len() > 1 {
                cook_rules.gltf_primitive_index = Some(primitive_index as u32);
            }
            prepared_assets.push(SourceAssetEntry {
                id: AssetId::new(&id),
                asset_type: AssetType::Mesh,
                source_path: manifest_source_path(&relative_source),
                cook_rules: cook_rules.clone(),
            });
            if !scene.primitives[primitive_index].morph_targets.is_empty() {
                let morph_id = format!("{id}.morphs");
                validate_import_asset_id(&morph_id)?;
                prepared_assets.push(SourceAssetEntry {
                    id: AssetId::new(morph_id),
                    asset_type: AssetType::MorphTargetSet,
                    source_path: manifest_source_path(&relative_source),
                    cook_rules,
                });
            }
        }

        let imported_skins = engine_animation::import_gltf_animation_assets(&scene)?;
        for imported_skin in imported_skins {
            let skeleton_id = format!(
                "{}.skeleton.{}",
                request.asset_id, imported_skin.source_skin_index
            );
            validate_import_asset_id(&skeleton_id)?;
            let skeleton_name = format!(
                "{}.skin{}.skel",
                request.asset_id, imported_skin.source_skin_index
            );
            let skeleton_relative = import_folder.join(&skeleton_name);
            let skeleton_bytes = bincode::serialize(&imported_skin.skeleton)
                .map_err(|error| format!("could not serialize imported skeleton: {error}"))?;
            generated_sources.push((
                project.asset_source.join(&skeleton_relative),
                skeleton_bytes,
            ));
            prepared_assets.push(SourceAssetEntry {
                id: AssetId::new(skeleton_id),
                asset_type: AssetType::Skeleton,
                source_path: manifest_source_path(&skeleton_relative),
                cook_rules: CookRules::default(),
            });

            for (animation_index, animation) in imported_skin.animations.iter().enumerate() {
                let animation_id = format!(
                    "{}.animation.{}.{}",
                    request.asset_id, imported_skin.source_skin_index, animation_index
                );
                validate_import_asset_id(&animation_id)?;
                let animation_name = format!(
                    "{}.skin{}.animation{}.anim",
                    request.asset_id, imported_skin.source_skin_index, animation_index
                );
                let animation_relative = import_folder.join(&animation_name);
                let animation_bytes = bincode::serialize(animation).map_err(|error| {
                    format!(
                        "could not serialize imported animation '{}': {error}",
                        animation.name
                    )
                })?;
                generated_sources.push((
                    project.asset_source.join(&animation_relative),
                    animation_bytes,
                ));
                prepared_assets.push(SourceAssetEntry {
                    id: AssetId::new(animation_id),
                    asset_type: AssetType::Animation,
                    source_path: manifest_source_path(&animation_relative),
                    cook_rules: CookRules::default(),
                });
            }
        }
    } else {
        prepared_assets.push(SourceAssetEntry {
            id: AssetId::new(request.asset_id.clone()),
            asset_type: asset_type.clone(),
            source_path: manifest_source_path(&relative_source),
            cook_rules: CookRules::default(),
        });
    }

    let mut planned_source_names = BTreeSet::new();
    for target in std::iter::once(&copied_source)
        .chain(external_sources.iter().map(|(_, target)| target))
        .chain(generated_sources.iter().map(|(path, _)| path))
    {
        let name = target
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                format!(
                    "generated import path is not portable: {}",
                    target.display()
                )
            })?;
        let portable_target = target.to_string_lossy().to_ascii_lowercase();
        if !planned_source_names.insert(portable_target) {
            return Err(format!(
                "import would generate duplicate source path '{}'",
                target.display()
            ));
        }
        let parent = target
            .parent()
            .ok_or_else(|| format!("import target has no parent: {}", target.display()))?;
        if let Some(conflict) = find_case_insensitive_entry(parent, name)? {
            return Err(format!(
                "source asset target already exists and will not be overwritten: {}",
                conflict.display()
            ));
        }
    }

    let requested_ids = prepared_assets
        .iter()
        .map(|entry| entry.id.id.clone())
        .collect::<Vec<_>>();
    let mut cooked_targets = Vec::with_capacity(prepared_assets.len());
    for entry in &prepared_assets {
        let cooked_name = format!("{}.cooked", entry.id.id);
        if let Some(conflict) = find_case_insensitive_entry(&project.cooked_assets, &cooked_name)? {
            return Err(format!(
                "cooked asset target already exists and will not be overwritten: {}",
                conflict.display()
            ));
        }
        cooked_targets.push(project.cooked_assets.join(cooked_name));
    }

    let (manifest_path, mut manifest) = load_import_manifest(&project, &requested_ids)?;
    let original_manifest =
        if manifest_path.is_file() {
            Some(std::fs::read(&manifest_path).map_err(|error| {
                format!("could not back up {}: {error}", manifest_path.display())
            })?)
        } else if manifest_path.exists() {
            return Err(format!(
                "source manifest target is not a regular file: {}",
                manifest_path.display()
            ));
        } else {
            None
        };

    manifest.assets.extend(prepared_assets.iter().cloned());
    manifest.assets.sort_by(|left, right| {
        left.id
            .id
            .to_ascii_lowercase()
            .cmp(&right.id.id.to_ascii_lowercase())
            .then_with(|| left.id.id.cmp(&right.id.id))
    });
    let mut manifest_json = serde_json::to_string_pretty(&manifest)
        .map_err(|error| format!("could not serialize source manifest: {error}"))?;
    manifest_json.push('\n');
    let staging_dir = import_staging_directory(&project)?;

    let mut created_sources =
        Vec::with_capacity(generated_sources.len() + external_sources.len() + 1);
    copy_file_create_new(&source_file, &copied_source)?;
    created_sources.push(copied_source.clone());
    for (source, target) in &external_sources {
        if let Some(parent) = target.parent() {
            if let Err(error) = std::fs::create_dir_all(parent) {
                cleanup_import_files(&created_sources);
                return Err(format!(
                    "could not create glTF dependency directory {}: {error}",
                    parent.display()
                ));
            }
        }
        if let Err(error) = copy_file_create_new(source, target) {
            cleanup_import_files(&created_sources);
            return Err(error);
        }
        created_sources.push(target.clone());
    }
    for (target, bytes) in &generated_sources {
        if let Err(error) = write_bytes_create_new(target, bytes) {
            cleanup_import_files(&created_sources);
            return Err(error);
        }
        created_sources.push(target.clone());
    }
    if let Err(error) = write_text(&manifest_path, &manifest_json) {
        return Err(rollback_import_failure(
            error,
            &manifest_path,
            original_manifest.as_deref(),
            &created_sources,
            &[],
        ));
    }

    let mut graph = DependencyGraph::new();
    let mut runtime_builder = engine_core::EngineRuntime::builder(engine_core::EngineConfig {
        application_name: format!("{}-asset-import", project.manifest.name),
        gpu_timestamps: true,
    });
    engine_animation::loader::register_asset_types(runtime_builder.asset_type_registry_mut());
    let cook_report = cook_orchestrate_checked_with_registry(
        &project.asset_source,
        &staging_dir,
        &mut graph,
        runtime_builder.asset_type_registry(),
    );
    let cook_result = if !cook_report.is_success() {
        Err(cook_report_failure(&cook_report))
    } else {
        prepared_assets.iter().try_for_each(|entry| {
            if !cook_report
                .results
                .iter()
                .any(|result| result.success && result.asset_id == entry.id.id)
            {
                return Err(format!(
                    "cook succeeded without reporting imported asset '{}'",
                    entry.id.id
                ));
            }
            let staged_cooked = staging_dir.join(format!("{}.cooked", entry.id.id));
            let artifact = read_cooked_artifact(&staged_cooked).map_err(|error| {
                format!(
                    "imported asset did not produce a valid cooked artifact {}: {error}",
                    staged_cooked.display()
                )
            })?;
            if artifact.header.asset_kind != entry.asset_type.kind_code() {
                return Err(format!(
                    "imported asset '{}' cooked as kind {}, expected {}",
                    entry.id.id,
                    artifact.header.asset_kind,
                    entry.asset_type.kind_code()
                ));
            }
            Ok(())
        })
    };
    if let Err(error) = cook_result {
        remove_import_staging(&project, &staging_dir);
        return Err(rollback_import_failure(
            error,
            &manifest_path,
            original_manifest.as_deref(),
            &created_sources,
            &[],
        ));
    }

    if let Err(error) = std::fs::create_dir_all(&project.cooked_assets) {
        remove_import_staging(&project, &staging_dir);
        return Err(rollback_import_failure(
            format!(
                "could not create cooked asset directory {}: {error}",
                project.cooked_assets.display()
            ),
            &manifest_path,
            original_manifest.as_deref(),
            &created_sources,
            &[],
        ));
    }
    let mut installed_cooked = Vec::with_capacity(prepared_assets.len());
    for (entry, cooked_target) in prepared_assets.iter().zip(&cooked_targets) {
        let staged_cooked = staging_dir.join(format!("{}.cooked", entry.id.id));
        if let Err(error) = copy_file_create_new(&staged_cooked, cooked_target) {
            remove_import_staging(&project, &staging_dir);
            return Err(rollback_import_failure(
                error,
                &manifest_path,
                original_manifest.as_deref(),
                &created_sources,
                &installed_cooked,
            ));
        }
        installed_cooked.push(cooked_target.clone());
        if let Err(error) = read_cooked_artifact(cooked_target) {
            remove_import_staging(&project, &staging_dir);
            return Err(rollback_import_failure(
                format!(
                    "installed cooked artifact {} failed validation: {error}",
                    cooked_target.display()
                ),
                &manifest_path,
                original_manifest.as_deref(),
                &created_sources,
                &installed_cooked,
            ));
        }
    }
    remove_import_staging(&project, &staging_dir);

    let imported_assets = prepared_assets
        .iter()
        .zip(&cooked_targets)
        .map(|(entry, cooked)| {
            serde_json::json!({
                "asset_id": entry.id.id,
                "asset_type": import_asset_type_label(&entry.asset_type),
                "source": absolute_for_report(&project.asset_source.join(&entry.source_path)),
                "cooked": absolute_for_report(cooked),
            })
        })
        .collect::<Vec<_>>();
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "schema": "ProjectImportReport-v0",
            "project": project.manifest.name,
            "asset_id": request.asset_id,
            "asset_type": import_asset_type_label(&asset_type),
            "source": absolute_for_report(&copied_source),
            "manifest": absolute_for_report(&manifest_path),
            "cooked": absolute_for_report(&cooked_targets[0]),
            "assets": imported_assets,
            "cooked_assets_checked": cook_report.succeeded_asset_count,
            "imported": true
        }))
        .expect("JSON value serialization cannot fail")
    );
    Ok(())
}

mod filesystem;
mod manifest;

pub(crate) use filesystem::*;
pub(crate) use manifest::*;
