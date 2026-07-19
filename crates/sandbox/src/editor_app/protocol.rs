//! Versioned protocol between the React editor shell and the Rust editor host.
//!
//! Every production mutation is represented by one explicit request variant.
//! There is deliberately no generic stringly-typed `execute` escape hatch.

use engine_asset::cook::AssetType;
use engine_gameplay::input::InputActionMap;
use engine_scene::SceneSettings;
use engine_serialize::{AssetId, Value};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value as JsonValue;

pub const EDITOR_PROTOCOL: &str = "EngineEditorIpc-v2";
pub const EDITOR_PROTOCOL_VERSION: u32 = 2;
pub const PROJECT_CHANGED_EVENT: &str = "project.changed";
pub const TELEMETRY_EVENT: &str = "editor.telemetry";
pub const UI_OPEN_PANEL_EVENT: &str = "ui.openPanel";

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

    fn invalid_params(method: &str, error: serde_json::Error) -> Self {
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
    ApplySceneSettings(SceneSettingsParams),
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

impl EditorRequest {
    pub fn decode(request: &BridgeRequest) -> Result<Self, BridgeError> {
        let method = request.method.as_str();
        let decoded = match method {
            "editor.ready" => Self::Ready(params(request)?),
            "editor.getSnapshot" => Self::GetSnapshot,
            "editor.quit" => Self::RequestExit,
            "document.save" => Self::SaveDocument,
            "document.open" => Self::OpenDocument(params(request)?),
            "document.create" => Self::CreateDocument(params(request)?),
            "document.saveAs" => Self::SaveDocumentAs(params(request)?),
            "document.duplicate" => Self::DuplicateDocument(params(request)?),
            "document.rename" => Self::RenameDocument(params(request)?),
            "document.delete" => Self::DeleteDocument(params(request)?),
            "document.setStartup" => Self::SetStartupDocument(params(request)?),
            "document.resolvePendingSwitch" => Self::ResolvePendingSwitch(params(request)?),
            "document.resolveClose" => Self::ResolveClose(params(request)?),
            "document.resolveRecovery" => Self::ResolveRecovery(params(request)?),
            "scene.undo" => Self::Undo,
            "scene.redo" => Self::Redo,
            "scene.select" => Self::SelectEntity(params(request)?),
            "scene.createEntity" => Self::CreateEntity(params(request)?),
            "scene.setEntityEnabled" => Self::SetEntityEnabled(params(request)?),
            "scene.renameEntity" => Self::SetEntityName(params(request)?),
            "scene.setEntityParent" => Self::SetEntityParent(params(request)?),
            "scene.moveEntity" => Self::MoveEntity(params(request)?),
            "scene.copyEntities" => Self::CopyEntities(params(request)?),
            "scene.cutEntities" => Self::CutEntities(params(request)?),
            "scene.pasteEntities" => Self::PasteEntities(params(request)?),
            "scene.duplicateEntity" => Self::DuplicateEntity(params(request)?),
            "scene.deleteEntity" => Self::DeleteEntity(params(request)?),
            "scene.setComponentEnabled" => Self::SetComponentEnabled(params(request)?),
            "scene.setComponentField" => Self::SetComponentField(params(request)?),
            "scene.addComponent" => Self::AddComponent(params(request)?),
            "scene.resetComponent" => Self::ResetComponent(params(request)?),
            "scene.removeComponent" => Self::RemoveComponent(params(request)?),
            "scene.copyComponent" => Self::CopyComponent(params(request)?),
            "scene.pasteComponent" => Self::PasteComponent(params(request)?),
            "scene.applySettings" => Self::ApplySceneSettings(params(request)?),
            "runtime.setMode" => Self::SetRuntimeMode(params(request)?),
            "runtime.step" => Self::StepRuntime,
            "viewport.bounds" => Self::SetViewportBounds(params(request)?),
            "viewport.input" => Self::ViewportInput(params(request)?),
            "viewport.setTool" => Self::SetGizmoMode(params(request)?),
            "viewport.setOrientationMode" => Self::SetGizmoSpace(params(request)?),
            "viewport.setSnapping" => Self::SetSnapping(params(request)?),
            "viewport.focusSelection" => Self::FrameSelected,
            "viewport.setCamera" => Self::SetCamera(params(request)?),
            "viewport.setGizmos" => Self::SetGizmos(params(request)?),
            "assets.select" => Self::SelectAsset(params(request)?),
            "assets.setBrowser" => Self::SetAssetBrowser(params(request)?),
            "assets.refresh" => Self::RefreshAssets,
            "project.reveal" => Self::RevealProject,
            "assets.revealFolder" => Self::RevealAssetFolder(params(request)?),
            "assets.reveal" => Self::RevealAsset(params(request)?),
            "assets.open" => Self::OpenAsset(params(request)?),
            "assets.import" => Self::ImportAsset(params(request)?),
            "assets.createFolder" => Self::CreateAssetFolder(params(request)?),
            "assets.renameFolder" => Self::RenameAssetFolder(params(request)?),
            "assets.deleteFolder" => Self::DeleteAssetFolder(params(request)?),
            "assets.createMaterial" => Self::CreateMaterial(params(request)?),
            "assets.createPrefab" => Self::CreatePrefab(params(request)?),
            "assets.instantiatePrefab" => Self::InstantiatePrefab(params(request)?),
            "assets.unpackPrefab" => Self::UnpackPrefab(params(request)?),
            "assets.duplicate" => Self::DuplicateAsset(params(request)?),
            "assets.move" => Self::MoveAsset(params(request)?),
            "assets.delete" => Self::DeleteAsset(params(request)?),
            "assets.assign" => Self::AssignAsset(params(request)?),
            "material.open" => Self::OpenMaterial(params(request)?),
            "material.setParameter" => Self::SetMaterialParameter(params(request)?),
            "material.save" => Self::SaveMaterial,
            "material.assign" => Self::AssignOpenMaterial,
            "animation.setState" => Self::SetAnimation(params(request)?),
            "terrain.replaySeed" => Self::ReplayTerrainSeed(params(request)?),
            "terrain.regenerate" => Self::RegenerateTerrain,
            "terrain.retryFailed" => Self::RetryTerrain,
            "console.clear" => Self::ClearDiagnostics,
            "console.export" => Self::ExportDiagnostics,
            "build.start" => Self::StartBuild(params(request)?),
            "build.cancel" => Self::CancelBuild,
            "build.run" => Self::RunProject,
            "project.create" => Self::CreateProject(params(request)?),
            "project.open" => Self::OpenProject(params(request)?),
            "project.saveSettings" => Self::SaveProjectSettings(params(request)?),
            "settings.replaceInputMap" => Self::ReplaceInputMap(params(request)?),
            "settings.saveInputMap" => Self::SaveInputMap,
            "script.create" => Self::CreateScript(params(request)?),
            "script.rebuild" => Self::RebuildScripts,
            "script.attach" => Self::AttachScript(params(request)?),
            "layout.persist" => Self::PersistLayout(params(request)?),
            _ => {
                return Err(BridgeError::new(
                    EditorErrorCode::InvalidRequest,
                    format!("Unknown editor method '{method}'"),
                ));
            }
        };
        Ok(decoded)
    }
}

fn params<T: DeserializeOwned>(request: &BridgeRequest) -> Result<T, BridgeError> {
    serde_json::from_value(request.params.clone())
        .map_err(|error| BridgeError::invalid_params(&request.method, error))
}

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

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeModeDto {
    Edit,
    Play,
    Paused,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeModeParams {
    pub mode: RuntimeModeDto,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewportBoundsParams {
    pub viewport: String,
    pub rect: ScreenRect,
    pub visible: bool,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InputModifiers {
    pub alt: bool,
    pub control: bool,
    pub meta: bool,
    pub shift: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ViewportInput {
    PointerDown {
        pointer_id: i64,
        x: f32,
        y: f32,
        button: i16,
        buttons: u16,
        modifiers: InputModifiers,
    },
    PointerUp {
        pointer_id: i64,
        x: f32,
        y: f32,
        button: i16,
        buttons: u16,
        modifiers: InputModifiers,
    },
    PointerMove {
        pointer_id: i64,
        x: f32,
        y: f32,
        button: i16,
        buttons: u16,
        modifiers: InputModifiers,
    },
    PointerCancel {
        pointer_id: i64,
    },
    Wheel {
        x: f32,
        y: f32,
        delta_x: f32,
        delta_y: f32,
        delta_mode: u8,
        modifiers: InputModifiers,
    },
    KeyDown {
        key: String,
        code: String,
        repeat: bool,
        modifiers: InputModifiers,
    },
    KeyUp {
        key: String,
        code: String,
        repeat: bool,
        modifiers: InputModifiers,
    },
    Focus,
    Blur,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewportInputParams {
    pub viewport: String,
    pub event: ViewportInput,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GizmoModeDto {
    Move,
    Rotate,
    Scale,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GizmoModeParams {
    pub mode: GizmoModeDto,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GizmoSpaceDto {
    Global,
    Local,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GizmoSpaceParams {
    pub mode: GizmoSpaceDto,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetSnappingParams {
    pub enabled: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraParams {
    pub pitch: f32,
    pub yaw: f32,
    pub distance: f32,
    pub target: [f32; 3],
    pub orthographic: bool,
    pub speed: f32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetGizmosParams {
    pub visible: bool,
}

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

#[cfg(test)]
mod tests {
    use super::*;

    fn request(method: &str, params: JsonValue) -> BridgeRequest {
        BridgeRequest {
            id: "request-1".into(),
            protocol: Some(EDITOR_PROTOCOL.into()),
            session_id: Some("session-1".into()),
            base_revision: Some(4),
            method: method.into(),
            params,
        }
    }

    #[test]
    fn unknown_methods_are_rejected_instead_of_using_a_generic_escape_hatch() {
        let error = EditorRequest::decode(&request(
            "editor.execute",
            serde_json::json!({"command": "anything"}),
        ))
        .unwrap_err();
        assert!(matches!(error.code, EditorErrorCode::InvalidRequest));
    }

    #[test]
    fn retired_command_aliases_cannot_reopen_a_second_protocol_path() {
        for method in [
            "editor.saveScene",
            "editor.undo",
            "editor.redo",
            "scene.reparent",
            "scene.duplicateSelection",
            "scene.deleteSelection",
        ] {
            let error = EditorRequest::decode(&request(method, serde_json::json!({})))
                .expect_err("retired command aliases must stay unavailable");
            assert!(matches!(error.code, EditorErrorCode::InvalidRequest));
        }
    }

    #[test]
    fn retired_parameter_aliases_and_fake_gizmo_modes_are_rejected() {
        for (method, params) in [
            (
                "scene.select",
                serde_json::json!({"activeEntityId": "cube"}),
            ),
            (
                "scene.createEntity",
                serde_json::json!({"archetype": "cube"}),
            ),
            (
                "scene.setEntityParent",
                serde_json::json!({"entityId": "cube", "parentId": "root"}),
            ),
            (
                "scene.setComponentField",
                serde_json::json!({
                    "entityId": "cube",
                    "componentType": "engine.transform",
                    "fieldPath": "translation",
                    "value": {"Vec3": [1.0, 2.0, 3.0]}
                }),
            ),
            ("viewport.setTool", serde_json::json!({"tool": "move"})),
            ("viewport.setTool", serde_json::json!({"mode": "hand"})),
            ("viewport.setTool", serde_json::json!({"mode": "rect"})),
            ("viewport.setTool", serde_json::json!({"mode": "combined"})),
        ] {
            let error = EditorRequest::decode(&request(method, params))
                .expect_err("retired protocol shapes must stay unavailable");
            assert!(matches!(error.code, EditorErrorCode::InvalidRequest));
        }
    }

    #[test]
    fn component_field_request_decodes_a_typed_engine_value() {
        let decoded = EditorRequest::decode(&request(
            "scene.setComponentField",
            serde_json::json!({
                "entityId": "cube",
                "componentType": "engine.transform",
                "fieldName": "translation",
                "value": {"Vec3": [1.0, 2.0, 3.0]}
            }),
        ))
        .unwrap();
        let EditorRequest::SetComponentField(params) = decoded else {
            panic!("wrong request variant");
        };
        assert_eq!(params.entity_id, "cube");
        assert_eq!(params.value, Value::Vec3([1.0, 2.0, 3.0]));
    }

    #[test]
    fn viewport_input_is_explicit_and_uses_css_local_coordinates() {
        let decoded = EditorRequest::decode(&request(
            "viewport.input",
            serde_json::json!({
                "viewport": "scene",
                "event": {
                    "type": "pointerDown",
                    "pointerId": 7,
                    "x": 14.5,
                    "y": 20.0,
                    "button": 0,
                    "buttons": 1,
                    "modifiers": {"alt": false, "control": false, "meta": false, "shift": true}
                }
            }),
        ))
        .unwrap();
        assert!(matches!(decoded, EditorRequest::ViewportInput(_)));
    }

    #[test]
    fn viewport_camera_and_gizmo_visibility_have_typed_commands() {
        let decoded = EditorRequest::decode(&request(
            "viewport.setCamera",
            serde_json::json!({
                "pitch": 12.0,
                "yaw": 36.0,
                "distance": 8.0,
                "target": [1.0, 2.0, 3.0],
                "orthographic": true,
                "speed": 6.0
            }),
        ))
        .unwrap();
        let EditorRequest::SetCamera(camera) = decoded else {
            panic!("wrong camera request variant");
        };
        assert_eq!(camera.target, [1.0, 2.0, 3.0]);
        assert!(camera.orthographic);

        let decoded = EditorRequest::decode(&request(
            "viewport.setGizmos",
            serde_json::json!({"visible": false}),
        ))
        .unwrap();
        let EditorRequest::SetGizmos(gizmos) = decoded else {
            panic!("wrong gizmo request variant");
        };
        assert!(!gizmos.visible);
    }

    #[test]
    fn ui_open_panel_event_has_a_versioned_typed_wire_shape() {
        let event = BridgeEvent {
            protocol: EDITOR_PROTOCOL,
            session_id: "session-1".to_string(),
            sequence: 4,
            revision: 9,
            event: UI_OPEN_PANEL_EVENT,
            params: UiOpenPanelParams {
                panel: UiPanel::Material,
                preferred_zone: UiDockZone::Bottom,
            },
        };
        assert_eq!(
            serde_json::to_value(event).unwrap(),
            serde_json::json!({
                "protocol": EDITOR_PROTOCOL,
                "sessionId": "session-1",
                "sequence": 4,
                "revision": 9,
                "event": "ui.openPanel",
                "params": {"panel": "material", "preferredZone": "bottom"}
            })
        );
    }

    #[test]
    fn terrain_debug_commands_preserve_full_u64_seed_text() {
        let decoded = EditorRequest::decode(&request(
            "terrain.replaySeed",
            serde_json::json!({"seed": "18446744073709551615"}),
        ))
        .unwrap();
        let EditorRequest::ReplayTerrainSeed(params) = decoded else {
            panic!("wrong terrain request variant");
        };
        assert_eq!(params.seed, "18446744073709551615");
        assert!(matches!(
            EditorRequest::decode(&request("terrain.regenerate", serde_json::json!({}))).unwrap(),
            EditorRequest::RegenerateTerrain
        ));
    }
}
