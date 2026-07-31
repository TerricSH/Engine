use super::*;

pub(crate) fn manifest_source_path(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

pub(crate) fn gltf_external_source_files(
    source_file: &Path,
) -> Result<Vec<(PathBuf, PathBuf)>, String> {
    let document = gltf::Gltf::open(source_file)
        .map_err(|error| format!("could not inspect glTF dependencies: {error}"))?;
    let source_directory = source_file.parent().ok_or_else(|| {
        format!(
            "glTF source has no parent directory: {}",
            source_file.display()
        )
    })?;
    let source_directory = std::fs::canonicalize(source_directory).map_err(|error| {
        format!(
            "could not resolve glTF source directory {}: {error}",
            source_directory.display()
        )
    })?;

    let mut uris = Vec::new();
    for buffer in document.buffers() {
        if let gltf::buffer::Source::Uri(uri) = buffer.source() {
            uris.push(uri.to_string());
        }
    }
    for image in document.images() {
        if let gltf::image::Source::Uri { uri, .. } = image.source() {
            uris.push(uri.to_string());
        }
    }

    let mut portable_paths = BTreeSet::new();
    let mut dependencies = Vec::new();
    for uri in uris {
        if uri.starts_with("data:") {
            continue;
        }
        if uri.contains(':') {
            return Err(format!(
                "glTF dependency URI '{uri}' uses an unsupported external scheme"
            ));
        }
        let decoded = urlencoding::decode(&uri)
            .map_err(|error| format!("glTF dependency URI '{uri}' is invalid: {error}"))?;
        let decoded_path = PathBuf::from(decoded.as_ref());
        let mut relative = PathBuf::new();
        for component in decoded_path.components() {
            match component {
                Component::Normal(part) => relative.push(part),
                Component::CurDir => {}
                _ => {
                    return Err(format!(
                        "glTF dependency URI '{uri}' escapes its source directory"
                    ));
                }
            }
        }
        if relative.as_os_str().is_empty() {
            return Err(format!("glTF dependency URI '{uri}' has no file path"));
        }
        let portable = relative
            .to_string_lossy()
            .replace('\\', "/")
            .to_ascii_lowercase();
        if !portable_paths.insert(portable) {
            continue;
        }
        let resolved = std::fs::canonicalize(source_directory.join(&relative))
            .map_err(|error| format!("could not resolve glTF dependency '{uri}': {error}"))?;
        if !resolved.starts_with(&source_directory) || !resolved.is_file() {
            return Err(format!(
                "glTF dependency '{uri}' is not a regular file below {}",
                source_directory.display()
            ));
        }
        dependencies.push((resolved, relative));
    }
    dependencies.sort_by(|left, right| left.1.cmp(&right.1));
    Ok(dependencies)
}

#[cfg(feature = "tooling-editor")]
pub(crate) fn import_project_asset_from(
    project: PathBuf,
    source_file: PathBuf,
    asset_id: String,
    asset_type: Option<AssetType>,
    folder: PathBuf,
) -> Result<(), String> {
    import_project_asset(&ProjectImportRequest {
        project,
        source_file,
        asset_id,
        asset_type,
        folder,
    })
}

pub(crate) fn load_import_manifest(
    project: &GameProject,
    requested_asset_ids: &[String],
) -> Result<(PathBuf, SourceManifest), String> {
    let paths = source_manifest_paths(&project.asset_source)?;
    let mut portable_ids = BTreeSet::new();
    let mut game_manifest = None;
    for path in paths {
        let content = std::fs::read_to_string(&path)
            .map_err(|error| format!("could not read {}: {error}", path.display()))?;
        let manifest: SourceManifest = serde_json::from_str(&content)
            .map_err(|error| format!("invalid {}: {error}", path.display()))?;
        if manifest.schema_version != CURRENT_MANIFEST_VERSION {
            return Err(format!(
                "unsupported source manifest schema in {}",
                path.display()
            ));
        }
        for asset in &manifest.assets {
            validate_import_asset_id(&asset.id.id)
                .map_err(|error| format!("invalid asset id in {}: {error}", path.display()))?;
            let portable_id = asset.id.id.to_ascii_lowercase();
            if !portable_ids.insert(portable_id) {
                return Err(format!(
                    "asset id '{}' is duplicated or differs only by case in source manifests",
                    asset.id.id
                ));
            }
            validate_source_path(&project.asset_source, &asset.source_path)?;
        }
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("game.manifest"))
        {
            if game_manifest.is_some() {
                return Err(
                    "multiple source manifests differ only by the name 'game.manifest'".into(),
                );
            }
            game_manifest = Some((path, manifest));
        }
    }

    let mut requested_portable_ids = BTreeSet::new();
    for requested_asset_id in requested_asset_ids {
        let portable_id = requested_asset_id.to_ascii_lowercase();
        if !requested_portable_ids.insert(portable_id.clone()) {
            return Err(format!(
                "generated asset id '{requested_asset_id}' is duplicated or differs only by case"
            ));
        }
        if portable_ids.contains(&portable_id) {
            return Err(format!(
                "asset id '{requested_asset_id}' already exists or differs only by case"
            ));
        }
    }
    Ok(game_manifest.unwrap_or_else(|| {
        (
            project.asset_source.join("game.manifest"),
            SourceManifest {
                schema_version: CURRENT_MANIFEST_VERSION,
                assets: Vec::new(),
            },
        )
    }))
}

pub(crate) fn resolve_import_asset_type(
    source: &Path,
    requested: Option<&AssetType>,
) -> Result<AssetType, String> {
    let file_name = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("source file name is not UTF-8: {}", source.display()))?
        .to_ascii_lowercase();
    let extension = source
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let inferred = if file_name.ends_with(".prefab.ron") {
        Some(AssetType::Prefab)
    } else if file_name.ends_with(".material.json") {
        Some(AssetType::Material)
    } else if matches!(extension.as_str(), "gltf" | "glb") {
        Some(AssetType::Mesh)
    } else if matches!(extension.as_str(), "hdr" | "exr") {
        Some(AssetType::EnvironmentMap)
    } else if is_supported_texture_extension(&extension) {
        Some(AssetType::Texture)
    } else if matches!(extension.as_str(), "wav" | "mp3" | "ogg" | "flac") {
        Some(AssetType::Audio)
    } else if extension == "anim" {
        Some(AssetType::Animation)
    } else if extension == "skel" {
        Some(AssetType::Skeleton)
    } else if matches!(extension.as_str(), "navmesh" | "nav") {
        Some(AssetType::NavMesh)
    } else {
        None
    };

    let asset_type = requested.cloned().or(inferred.clone()).ok_or_else(|| {
        format!(
            "could not safely infer an import type for {}; use a supported extension or --type",
            source.display()
        )
    })?;
    let extension_supported = match asset_type {
        AssetType::Mesh => matches!(extension.as_str(), "gltf" | "glb"),
        AssetType::Texture => is_supported_texture_extension(&extension),
        AssetType::Material => extension == "json",
        AssetType::EnvironmentMap => matches!(extension.as_str(), "hdr" | "exr"),
        AssetType::Audio => matches!(extension.as_str(), "wav" | "mp3" | "ogg" | "flac"),
        AssetType::Animation => extension == "anim",
        AssetType::Skeleton => extension == "skel",
        AssetType::NavMesh => matches!(extension.as_str(), "navmesh" | "nav"),
        AssetType::Prefab => file_name.ends_with(".prefab.ron"),
        _ => false,
    };
    if !extension_supported {
        return Err(format!(
            "source extension '.{extension}' is not supported for {} imports",
            import_asset_type_label(&asset_type)
        ));
    }
    if let (Some(requested), Some(inferred)) = (requested, inferred) {
        if requested != &inferred {
            return Err(format!(
                "requested import type {} conflicts with the source format inferred as {}",
                import_asset_type_label(requested),
                import_asset_type_label(&inferred)
            ));
        }
    }
    Ok(asset_type)
}
