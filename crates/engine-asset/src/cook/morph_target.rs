//! Cook glTF morph target deltas independently from the base mesh.

use std::path::Path;

use engine_serialize::SchemaVersion;
use serde::{Deserialize, Serialize};

use super::{error::CookError, write_cooked_artifact, AssetType, CookResult, CookedArtifact};

pub const COOKED_MORPH_TARGET_SCHEMA_VERSION: SchemaVersion = SchemaVersion::new(0, 1, 0);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CookedMorphTarget {
    pub name: String,
    pub position_deltas: Vec<[f32; 3]>,
    pub normal_deltas: Vec<[f32; 3]>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CookedMorphTargetSet {
    pub vertex_count: u32,
    pub targets: Vec<CookedMorphTarget>,
}

pub fn cook_morph_target_set(
    source: &Path,
    output: &Path,
    primitive_index: Option<u32>,
) -> Result<CookResult, CookError> {
    let scene = crate::gltf::load_gltf_scene(source)
        .map_err(|error| CookError::Parse(error.to_string()))?;
    if primitive_index.is_none() && scene.primitives.len() != 1 {
        return Err(CookError::InvalidAsset(format!(
            "single morph-target asset requires exactly one glTF primitive, found {}",
            scene.primitives.len()
        )));
    }
    let selected_index = primitive_index.unwrap_or(0) as usize;
    let primitive = scene.primitives.get(selected_index).ok_or_else(|| {
        CookError::InvalidAsset(format!(
            "glTF primitive selection {selected_index} is out of range for {} primitives",
            scene.primitives.len()
        ))
    })?;
    if primitive.morph_targets.is_empty() {
        return Err(CookError::InvalidAsset(format!(
            "glTF primitive {selected_index} contains no morph targets"
        )));
    }
    let cooked = CookedMorphTargetSet {
        vertex_count: u32::try_from(primitive.mesh.positions.len())
            .map_err(|_| CookError::InvalidAsset("morph vertex count exceeds u32".into()))?,
        targets: primitive
            .morph_targets
            .iter()
            .map(|target| CookedMorphTarget {
                name: target.name.clone(),
                position_deltas: target
                    .position_deltas
                    .iter()
                    .map(|delta| delta.to_array())
                    .collect(),
                normal_deltas: target
                    .normal_deltas
                    .iter()
                    .map(|delta| delta.to_array())
                    .collect(),
            })
            .collect(),
    };
    let payload =
        bincode::serialize(&cooked).map_err(|error| CookError::InvalidAsset(error.to_string()))?;
    write_cooked_artifact(
        output,
        AssetType::MorphTargetSet.kind_code(),
        &payload,
        COOKED_MORPH_TARGET_SCHEMA_VERSION,
    )
}

pub fn decode_cooked_morph_target_set(
    artifact: &CookedArtifact,
) -> Result<CookedMorphTargetSet, CookError> {
    if artifact.header.asset_kind != AssetType::MorphTargetSet.kind_code() {
        return Err(CookError::InvalidAsset(
            "artifact is not a morph-target set".into(),
        ));
    }
    if artifact.header.schema_version != COOKED_MORPH_TARGET_SCHEMA_VERSION {
        return Err(CookError::UnsupportedFormat(format!(
            "unsupported cooked morph-target schema {:?}",
            artifact.header.schema_version
        )));
    }
    bincode::deserialize(&artifact.payload)
        .map_err(|error| CookError::InvalidAsset(format!("invalid morph-target payload: {error}")))
}
