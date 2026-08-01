use super::*;

pub(super) struct PreparedGltfImport {
    pub assets: Vec<SourceAssetEntry>,
    pub generated_sources: Vec<(PathBuf, Vec<u8>)>,
}

pub(super) fn prepare_gltf_import(
    request: &ProjectImportRequest,
    source_file: &Path,
    relative_source: &Path,
    project: &GameProject,
    import_folder: &Path,
) -> Result<PreparedGltfImport, String> {
    let scene = engine_asset::gltf::load_gltf_scene(source_file)
        .map_err(|error| format!("could not inspect glTF import: {error}"))?;
    if scene.primitives.is_empty() {
        return Err("glTF import contains no mesh primitives".into());
    }
    if request.merge_primitives
        && scene
            .primitives
            .iter()
            .any(|primitive| !primitive.morph_targets.is_empty())
    {
        return Err(
            "merged glTF import does not support morph-target primitives; use --separate-primitives"
                .into(),
        );
    }

    let mut assets = Vec::new();
    let mut generated_sources = Vec::new();
    prepare_meshes(request, relative_source, &scene, &mut assets)?;
    prepare_materials_and_textures(
        request,
        project,
        import_folder,
        &scene,
        &mut assets,
        &mut generated_sources,
    )?;
    prepare_animation_assets(
        request,
        project,
        import_folder,
        &scene,
        &mut assets,
        &mut generated_sources,
    )?;

    Ok(PreparedGltfImport {
        assets,
        generated_sources,
    })
}

fn prepare_meshes(
    request: &ProjectImportRequest,
    relative_source: &Path,
    scene: &engine_asset::gltf::GltfScene,
    assets: &mut Vec<SourceAssetEntry>,
) -> Result<(), String> {
    let bake_node_transforms = request.bake_node_transforms.unwrap_or_else(|| {
        scene.primitives.iter().all(|primitive| {
            primitive.mesh.joints.is_empty()
                && primitive.mesh.weights.is_empty()
                && primitive.morph_targets.is_empty()
        })
    });
    if request.merge_primitives {
        let cook_rules = CookRules {
            gltf_merge_primitives: scene.primitives.len() > 1,
            gltf_bake_node_transforms: bake_node_transforms,
            ..CookRules::default()
        };
        assets.push(SourceAssetEntry {
            id: AssetId::new(&request.asset_id),
            asset_type: AssetType::Mesh,
            source_path: manifest_source_path(relative_source),
            cook_rules,
        });
        return Ok(());
    }

    for primitive_index in 0..scene.primitives.len() {
        let id = if primitive_index == 0 {
            request.asset_id.clone()
        } else {
            format!("{}.mesh.{primitive_index}", request.asset_id)
        };
        validate_import_asset_id(&id)?;
        let primitive_selection = (scene.primitives.len() > 1).then_some(primitive_index as u32);
        let cook_rules = CookRules {
            gltf_primitive_index: primitive_selection,
            gltf_bake_node_transforms: bake_node_transforms,
            ..CookRules::default()
        };
        assets.push(SourceAssetEntry {
            id: AssetId::new(&id),
            asset_type: AssetType::Mesh,
            source_path: manifest_source_path(relative_source),
            cook_rules,
        });
        if !scene.primitives[primitive_index].morph_targets.is_empty() {
            let morph_id = format!("{id}.morphs");
            validate_import_asset_id(&morph_id)?;
            let morph_rules = CookRules {
                gltf_primitive_index: primitive_selection,
                ..CookRules::default()
            };
            assets.push(SourceAssetEntry {
                id: AssetId::new(morph_id),
                asset_type: AssetType::MorphTargetSet,
                source_path: manifest_source_path(relative_source),
                cook_rules: morph_rules,
            });
        }
    }
    Ok(())
}

fn prepare_materials_and_textures(
    request: &ProjectImportRequest,
    project: &GameProject,
    import_folder: &Path,
    scene: &engine_asset::gltf::GltfScene,
    assets: &mut Vec<SourceAssetEntry>,
    generated_sources: &mut Vec<(PathBuf, Vec<u8>)>,
) -> Result<(), String> {
    let mut referenced_textures = BTreeSet::new();
    for material in &scene.materials {
        referenced_textures.extend(
            [
                material.base_color_texture,
                material.normal_texture,
                material.metallic_roughness_texture,
                material.occlusion_texture,
                material.emissive_texture,
            ]
            .into_iter()
            .flatten(),
        );
    }

    for texture_index in referenced_textures {
        let texture = scene
            .textures
            .get(texture_index)
            .ok_or_else(|| format!("glTF material references missing texture {texture_index}"))?;
        let texture_id = format!("{}.texture.{texture_index}", request.asset_id);
        validate_import_asset_id(&texture_id)?;
        let texture_name = format!("{texture_id}.png");
        let texture_relative = import_folder.join(texture_name);
        let texture_bytes =
            engine_asset::gltf::encode_texture_png(texture).map_err(|error| error.to_string())?;
        generated_sources.push((project.asset_source.join(&texture_relative), texture_bytes));
        assets.push(SourceAssetEntry {
            id: AssetId::new(texture_id),
            asset_type: AssetType::Texture,
            source_path: manifest_source_path(&texture_relative),
            cook_rules: CookRules::default(),
        });
    }

    for material in &scene.materials {
        let material_id = format!("{}.material.{}", request.asset_id, material.material_index);
        validate_import_asset_id(&material_id)?;
        let source = engine_asset::cook::material_source_from_gltf(material, |texture_index| {
            format!("{}.texture.{texture_index}", request.asset_id)
        });
        let mut source_bytes = serde_json::to_vec_pretty(&source)
            .map_err(|error| format!("could not serialize imported glTF material: {error}"))?;
        source_bytes.push(b'\n');
        let material_name = format!("{material_id}.material.json");
        let material_relative = import_folder.join(material_name);
        generated_sources.push((project.asset_source.join(&material_relative), source_bytes));
        assets.push(SourceAssetEntry {
            id: AssetId::new(material_id),
            asset_type: AssetType::Material,
            source_path: manifest_source_path(&material_relative),
            cook_rules: CookRules::default(),
        });
    }
    Ok(())
}

fn prepare_animation_assets(
    request: &ProjectImportRequest,
    project: &GameProject,
    import_folder: &Path,
    scene: &engine_asset::gltf::GltfScene,
    assets: &mut Vec<SourceAssetEntry>,
    generated_sources: &mut Vec<(PathBuf, Vec<u8>)>,
) -> Result<(), String> {
    let imported_skins = engine_animation::import_gltf_animation_assets(scene)?;
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
        assets.push(SourceAssetEntry {
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
            assets.push(SourceAssetEntry {
                id: AssetId::new(animation_id),
                asset_type: AssetType::Animation,
                source_path: manifest_source_path(&animation_relative),
                cook_rules: CookRules::default(),
            });
        }
    }
    Ok(())
}
