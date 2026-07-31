pub(crate) fn prepare_project_runtime(
    runtime: &mut EngineRuntime,
    project: &GameProject,
    scene: &Scene,
) -> Result<CookedAssetLoadReport, String> {
    let cooked_report = load_project_assets(runtime, project)?;
    let missing_assets = missing_runtime_asset_dependencies(runtime, scene);
    if !missing_assets.is_empty() {
        return Err(format!(
            "scene references runtime assets that are neither built-in nor present in {}: {}",
            project.cooked_assets.display(),
            missing_assets
                .into_iter()
                .map(|asset| asset.id)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    Ok(cooked_report)
}

/// Load every runtime-supported cooked asset without rejecting unresolved
/// scene references.
///
/// The player calls the strict wrapper above; the editor uses this entry point
/// so a broken authoring reference can still be opened and repaired.
pub(crate) fn load_project_assets(
    runtime: &mut EngineRuntime,
    project: &GameProject,
) -> Result<CookedAssetLoadReport, String> {
    let cooked_report = runtime
        .load_cooked_assets(&project.cooked_assets)
        .map_err(format_diagnostics)?;
    tracing::info!(
        discovered = cooked_report.discovered_assets,
        loaded_meshes = cooked_report.loaded_meshes,
        loaded_textures = cooked_report.loaded_textures,
        loaded_materials = cooked_report.loaded_materials,
        loaded_extensions = cooked_report.loaded_extension_assets(),
        skipped = cooked_report.skipped_assets.len(),
        "project cooked assets loaded"
    );
    Ok(cooked_report)
}

#[cfg(all(feature = "tooling-editor", feature = "backend-vulkan"))]
pub(crate) fn missing_render_asset_dependencies(
    runtime: &EngineRuntime,
    scene: &Scene,
) -> Vec<engine_serialize::AssetId> {
    render_asset_dependencies(scene)
        .into_iter()
        .filter(|asset| !runtime.asset_registry().contains(asset))
        .collect()
}

pub(super) fn missing_runtime_asset_dependencies(
    runtime: &EngineRuntime,
    scene: &Scene,
) -> Vec<engine_serialize::AssetId> {
    scene
        .collect_asset_dependencies()
        .into_iter()
        .filter(|asset| !runtime.asset_registry().contains(asset))
        .collect()
}

#[cfg(all(feature = "tooling-editor", feature = "backend-vulkan"))]
fn render_asset_dependencies(scene: &Scene) -> Vec<engine_serialize::AssetId> {
    let mut dependencies = scene
        .entities
        .iter()
        .filter_map(|entity| entity.components.get("engine.renderable"))
        .flat_map(|component| {
            ["mesh", "material"].into_iter().filter_map(move |field| {
                match component.fields.get(field) {
                    Some(engine_serialize::Value::Asset(asset)) => Some(asset.clone()),
                    _ => None,
                }
            })
        })
        .collect::<Vec<_>>();
    dependencies.sort();
    dependencies.dedup();
    dependencies
}
use super::*;
