use super::*;

pub(super) fn reject_known_references(
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
pub(super) struct TrashMetadata {
    pub(super) schema: String,
    pub(super) deleted_unix_nanos: u128,
    pub(super) manifest_path: String,
    pub(super) source_path: String,
    pub(super) cooked_path: String,
    pub(super) entry: SourceAssetEntry,
}

pub(super) fn allocate_trash_directory(
    project: &GameProject,
    asset_id: &AssetId,
) -> Result<PathBuf, String> {
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
