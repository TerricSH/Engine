//! Editor-facing automatic HLOD bake workflow.

use std::path::Path;

use engine_asset::cook::{
    apply_hlod_bake_to_scene, bake_hlod_scene, decode_cooked_mesh, read_cooked_artifact,
    write_hlod_proxy_artifacts, CookError, HlodBakeOutput, HlodBakeSettings,
};
use engine_scene::Scene;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum HlodAuthoringError {
    #[error(transparent)]
    Cook(#[from] CookError),
}

#[derive(Clone, Debug)]
pub struct HlodAuthoringReport {
    pub output: HlodBakeOutput,
    pub written_proxy_count: usize,
}

/// Bake the current scene using already cooked source meshes, write generated
/// proxy artifacts, and update the scene only after every artifact succeeds.
///
/// Generated proxy IDs are deterministic, so repeating the operation replaces
/// the same scene entities and `.cooked` files instead of accumulating copies.
pub fn bake_scene_hlod_assets(
    scene: &mut Scene,
    cooked_directory: &Path,
    settings: HlodBakeSettings,
) -> Result<HlodAuthoringReport, HlodAuthoringError> {
    let output = bake_hlod_scene(scene, settings, |mesh_asset| {
        let artifact =
            read_cooked_artifact(&cooked_directory.join(format!("{}.cooked", mesh_asset.id)))?;
        decode_cooked_mesh(&artifact)
    })?;
    let written = write_hlod_proxy_artifacts(&output, cooked_directory)?;
    apply_hlod_bake_to_scene(scene, &output, settings)?;
    Ok(HlodAuthoringReport {
        output,
        written_proxy_count: written.len(),
    })
}
