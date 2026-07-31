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

mod assets_project;
mod core;
mod runtime_viewport;
mod scene_entity;

pub use assets_project::*;
pub use core::*;
pub use runtime_viewport::*;
pub use scene_entity::*;

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
            "scene.applySettings" => Self::ApplySceneSettings(Box::new(params(request)?)),
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

#[cfg(test)]
mod tests {
    include!("protocol/tests.rs");
}
