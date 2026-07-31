#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetIdParams {
    pub asset_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssetFolderParams {
    #[serde(default)]
    pub folder: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectAssetParams {
    #[serde(default)]
    pub asset_id: Option<AssetId>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetBrowserParams {
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub folder: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub page: Option<usize>,
    #[serde(default)]
    pub view: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportAssetParams {
    pub source: String,
    pub asset_id: String,
    #[serde(default)]
    pub asset_type: Option<AssetType>,
    #[serde(default)]
    pub folder: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateFolderParams {
    pub folder: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameFolderParams {
    pub folder: String,
    pub new_folder: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateMaterialParams {
    pub folder: String,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatePrefabParams {
    pub asset_id: String,
    pub relative_source_path: String,
    pub manifest_name: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstantiatePrefabParams {
    pub asset_id: AssetId,
    #[serde(default)]
    pub parent_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UnpackModeDto {
    Instance,
    Completely,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnpackPrefabParams {
    pub entity_id: String,
    pub mode: UnpackModeDto,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveAssetParams {
    pub asset_id: String,
    pub new_source_path: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssignAssetParams {
    pub asset_id: String,
    pub entity_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaterialParameterParams {
    pub name: String,
    pub value: JsonValue,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnimationParams {
    #[serde(default)]
    pub skeleton: Option<Option<String>>,
    #[serde(default)]
    pub clip: Option<Option<String>>,
    #[serde(default)]
    pub playing: Option<bool>,
    #[serde(default)]
    pub looping: Option<bool>,
    #[serde(default)]
    pub speed: Option<f32>,
    #[serde(default)]
    pub time: Option<f32>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerrainSeedParams {
    /// Decimal u64 string avoids JavaScript's 53-bit integer limit.
    pub seed: String,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BuildOperation {
    Validate,
    CookAndCompile,
    PackageWindows,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildParams {
    #[serde(default)]
    pub target_id: Option<String>,
    #[serde(default)]
    pub operation: Option<BuildOperation>,
    #[serde(default)]
    pub run_after_build: bool,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub output_root: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateProjectParams {
    pub root: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default = "default_true")]
    pub with_csharp: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenProjectParams {
    pub path: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSettingsParams {
    pub title: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InputMapParams {
    pub map: InputActionMap,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateScriptParams {
    pub class_name: String,
    pub folder: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachScriptParams {
    pub entity_id: String,
    pub assembly_id: String,
    pub class_name: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayoutParams {
    pub serialized_layout: String,
}
use super::*;
