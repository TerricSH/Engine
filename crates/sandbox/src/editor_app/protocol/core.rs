#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeRequest {
    pub id: String,
    #[serde(default)]
    pub protocol: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub base_revision: Option<u64>,
    pub method: String,
    #[serde(default)]
    pub params: JsonValue,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeResponse<T: Serialize> {
    pub protocol: &'static str,
    pub id: String,
    pub session_id: String,
    pub revision: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<BridgeError>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeEvent<T: Serialize> {
    pub protocol: &'static str,
    pub session_id: String,
    pub sequence: u64,
    pub revision: u64,
    pub event: &'static str,
    pub params: T,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum UiPanel {
    Hierarchy,
    Scene,
    Game,
    Inspector,
    Project,
    Console,
    Material,
    Animation,
    Profiler,
    Terrain,
    Build,
    Settings,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum UiDockZone {
    Left,
    Center,
    Right,
    Bottom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiOpenPanelParams {
    pub panel: UiPanel,
    pub preferred_zone: UiDockZone,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeError {
    pub code: EditorErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_revision: Option<u64>,
}

impl BridgeError {
    pub fn new(code: EditorErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            field: None,
            current_revision: None,
        }
    }

    pub(super) fn invalid_params(method: &str, error: serde_json::Error) -> Self {
        Self {
            code: EditorErrorCode::InvalidRequest,
            message: format!("Invalid parameters for '{method}': {error}"),
            field: Some("params".into()),
            current_revision: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum EditorErrorCode {
    InvalidRequest,
    ProtocolMismatch,
    StaleRevision,
    EditingRequired,
    SelectionRequired,
    NotFound,
    Conflict,
    ValidationFailed,
    IoFailed,
    RuntimeUnavailable,
    ScriptFailed,
    Internal,
}

#[derive(Clone, Debug)]
pub enum EditorRequest {
    Ready(ReadyParams),
    GetSnapshot,
    RequestExit,
    SaveDocument,
    OpenDocument(SceneIdParams),
    CreateDocument(CreateSceneParams),
    SaveDocumentAs(SceneIdParams),
    DuplicateDocument(DuplicateSceneParams),
    RenameDocument(RenameSceneParams),
    DeleteDocument(DeleteSceneParams),
    SetStartupDocument(SceneIdParams),
    ResolvePendingSwitch(DecisionParams),
    ResolveClose(DecisionParams),
    ResolveRecovery(RecoveryDecisionParams),
    Undo,
    Redo,
    SelectEntity(SelectEntityParams),
    CreateEntity(CreateEntityParams),
    SetEntityEnabled(SetEntityEnabledParams),
    SetEntityName(SetEntityNameParams),
    SetEntityParent(SetEntityParentParams),
    MoveEntity(MoveEntityParams),
    CopyEntities(EntityIdsParams),
    CutEntities(EntityIdsParams),
    PasteEntities(PasteEntitiesParams),
    DuplicateEntity(EntityIdParams),
    DeleteEntity(EntityIdParams),
    SetComponentEnabled(SetComponentEnabledParams),
    SetComponentField(SetComponentFieldParams),
    AddComponent(ComponentParams),
    ResetComponent(ComponentParams),
    RemoveComponent(ComponentParams),
    CopyComponent(ComponentParams),
    PasteComponent(ComponentParams),
    ApplySceneSettings(Box<SceneSettingsParams>),
    SetRuntimeMode(RuntimeModeParams),
    StepRuntime,
    SetViewportBounds(ViewportBoundsParams),
    ViewportInput(ViewportInputParams),
    SetGizmoMode(GizmoModeParams),
    SetGizmoSpace(GizmoSpaceParams),
    SetSnapping(SetSnappingParams),
    FrameSelected,
    SetCamera(CameraParams),
    SetGizmos(SetGizmosParams),
    SelectAsset(SelectAssetParams),
    SetAssetBrowser(AssetBrowserParams),
    RefreshAssets,
    RevealProject,
    RevealAssetFolder(AssetFolderParams),
    RevealAsset(AssetIdParams),
    OpenAsset(AssetIdParams),
    ImportAsset(ImportAssetParams),
    CreateAssetFolder(CreateFolderParams),
    RenameAssetFolder(RenameFolderParams),
    DeleteAssetFolder(AssetFolderParams),
    CreateMaterial(CreateMaterialParams),
    CreatePrefab(CreatePrefabParams),
    InstantiatePrefab(InstantiatePrefabParams),
    UnpackPrefab(UnpackPrefabParams),
    DuplicateAsset(AssetIdParams),
    MoveAsset(MoveAssetParams),
    DeleteAsset(AssetIdParams),
    AssignAsset(AssignAssetParams),
    OpenMaterial(AssetIdParams),
    SetMaterialParameter(MaterialParameterParams),
    SaveMaterial,
    AssignOpenMaterial,
    SetAnimation(AnimationParams),
    ReplayTerrainSeed(TerrainSeedParams),
    RegenerateTerrain,
    RetryTerrain,
    ClearDiagnostics,
    ExportDiagnostics,
    StartBuild(BuildParams),
    CancelBuild,
    RunProject,
    CreateProject(CreateProjectParams),
    OpenProject(OpenProjectParams),
    SaveProjectSettings(ProjectSettingsParams),
    ReplaceInputMap(InputMapParams),
    SaveInputMap,
    CreateScript(CreateScriptParams),
    RebuildScripts,
    AttachScript(AttachScriptParams),
    PersistLayout(LayoutParams),
}
use super::*;
