#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadyParams {
    pub protocol_version: u32,
    #[serde(default)]
    pub client_version: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneIdParams {
    pub scene_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSceneParams {
    pub scene_id: String,
    pub folder: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateSceneParams {
    pub source_id: String,
    pub new_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameSceneParams {
    pub old_id: String,
    pub new_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteSceneParams {
    pub scene_id: String,
    #[serde(default)]
    pub replacement_startup: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SaveDiscardCancel {
    Save,
    Discard,
    Cancel,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionParams {
    pub decision: SaveDiscardCancel,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RecoveryDecision {
    Restore,
    Discard,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryDecisionParams {
    pub decision: RecoveryDecision,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SelectEntityParams {
    #[serde(default)]
    pub entity_id: Option<String>,
    #[serde(default)]
    pub entity_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntityIdParams {
    #[serde(default)]
    pub entity_id: String,
    #[serde(default)]
    pub entity_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntityIdsParams {
    pub entity_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateEntityParams {
    #[serde(default)]
    pub template_id: Option<String>,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub entity_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetEntityEnabledParams {
    #[serde(default)]
    pub entity_id: String,
    #[serde(default)]
    pub entity_ids: Vec<String>,
    pub enabled: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetEntityNameParams {
    pub entity_id: String,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetEntityParentParams {
    #[serde(default)]
    pub entity_id: String,
    #[serde(default)]
    pub entity_ids: Vec<String>,
    #[serde(default)]
    pub parent: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SiblingMoveDto {
    Up,
    Down,
    First,
    Last,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveEntityParams {
    pub entity_id: String,
    pub movement: SiblingMoveDto,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PasteEntitiesParams {
    #[serde(default)]
    pub parent_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentParams {
    #[serde(default)]
    pub entity_id: String,
    #[serde(default)]
    pub entity_ids: Vec<String>,
    pub component_type: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetComponentEnabledParams {
    #[serde(default)]
    pub entity_id: String,
    #[serde(default)]
    pub entity_ids: Vec<String>,
    pub component_type: String,
    pub enabled: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetComponentFieldParams {
    #[serde(default)]
    pub entity_id: String,
    #[serde(default)]
    pub entity_ids: Vec<String>,
    pub component_type: String,
    pub field_name: String,
    pub value: Value,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneSettingsParams {
    pub settings: SceneSettings,
}
use super::*;
