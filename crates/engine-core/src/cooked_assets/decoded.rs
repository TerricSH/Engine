use std::any::Any;
use std::path::PathBuf;

use engine_asset::cook::AssetType;
use engine_renderer::{
    AssetId, EnvironmentMapUpload, MaterialUpload, MeshUpload, MorphTargetSetUpload, TextureUpload,
};
use engine_serialize::{Diagnostic, DiagnosticSeverity};

pub(crate) enum DecodedCookedAsset {
    Mesh(MeshUpload),
    Texture(TextureUpload),
    Material(PathBuf, Box<MaterialUpload>),
    EnvironmentMap(EnvironmentMapUpload),
    MorphTargetSet(MorphTargetSetUpload),
    Extension(DecodedExtensionAsset),
    Skipped(AssetType),
}

impl DecodedCookedAsset {
    pub(crate) fn asset_id(&self) -> &AssetId {
        match self {
            DecodedCookedAsset::Mesh(upload) => &upload.mesh_id,
            DecodedCookedAsset::Texture(upload) => &upload.texture_id,
            DecodedCookedAsset::Material(_, upload) => &upload.material_id,
            DecodedCookedAsset::EnvironmentMap(upload) => &upload.environment_id,
            DecodedCookedAsset::MorphTargetSet(upload) => &upload.target_set_id,
            DecodedCookedAsset::Extension(asset) => &asset.id,
            DecodedCookedAsset::Skipped(_) => {
                unreachable!("skipped artifacts are never queued for commit")
            }
        }
    }
}

pub(crate) struct DecodedExtensionAsset {
    pub(crate) type_id: String,
    pub(crate) id: AssetId,
    pub(crate) payload: Vec<u8>,
    pub(crate) value: Box<dyn Any + Send + Sync>,
}

pub(crate) fn additive_conflict_error(id: &AssetId, kind: &str) -> Diagnostic {
    Diagnostic::new(
        "AS0003",
        DiagnosticSeverity::Error,
        "engine-core.cooked-assets",
        format!(
            "additive install of {kind} asset '{}' conflicts with a different asset already \
             installed under the same ID; unload it explicitly or use a replace-mode load",
            id.id
        ),
    )
}
