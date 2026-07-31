//! Exhaustive request-domain routing before domain-specific dispatch.

use super::*;

#[derive(Clone, Copy)]
enum RequestDomain {
    Document,
    Scene,
    Viewport,
    Asset,
    Project,
}

impl EditorApp {
    pub(super) fn dispatch_editor_request(
        &mut self,
        request: EditorRequest,
    ) -> Result<DispatchOutcome, BridgeError> {
        if self.pending_document_action.is_some()
            && matches!(
                &request,
                EditorRequest::OpenDocument(_)
                    | EditorRequest::CreateDocument(_)
                    | EditorRequest::SaveDocumentAs(_)
                    | EditorRequest::DuplicateDocument(_)
                    | EditorRequest::RenameDocument(_)
                    | EditorRequest::DeleteDocument(_)
                    | EditorRequest::SetStartupDocument(_)
            )
        {
            return Err(BridgeError::new(
                EditorErrorCode::Conflict,
                "Resolve or cancel the pending scene document operation before starting another",
            ));
        }

        let domain = match &request {
            EditorRequest::Ready(_)
            | EditorRequest::GetSnapshot
            | EditorRequest::RequestExit
            | EditorRequest::SaveDocument
            | EditorRequest::OpenDocument(_)
            | EditorRequest::CreateDocument(_)
            | EditorRequest::SaveDocumentAs(_)
            | EditorRequest::DuplicateDocument(_)
            | EditorRequest::RenameDocument(_)
            | EditorRequest::DeleteDocument(_)
            | EditorRequest::SetStartupDocument(_)
            | EditorRequest::ResolvePendingSwitch(_)
            | EditorRequest::ResolveClose(_)
            | EditorRequest::ResolveRecovery(_) => RequestDomain::Document,
            EditorRequest::Undo
            | EditorRequest::Redo
            | EditorRequest::SelectEntity(_)
            | EditorRequest::CreateEntity(_)
            | EditorRequest::SetEntityEnabled(_)
            | EditorRequest::SetEntityName(_)
            | EditorRequest::SetEntityParent(_)
            | EditorRequest::MoveEntity(_)
            | EditorRequest::CopyEntities(_)
            | EditorRequest::CutEntities(_)
            | EditorRequest::PasteEntities(_)
            | EditorRequest::DuplicateEntity(_)
            | EditorRequest::DeleteEntity(_)
            | EditorRequest::SetComponentEnabled(_)
            | EditorRequest::SetComponentField(_)
            | EditorRequest::AddComponent(_)
            | EditorRequest::ResetComponent(_)
            | EditorRequest::RemoveComponent(_)
            | EditorRequest::CopyComponent(_)
            | EditorRequest::PasteComponent(_)
            | EditorRequest::ApplySceneSettings(_) => RequestDomain::Scene,
            EditorRequest::SetRuntimeMode(_)
            | EditorRequest::StepRuntime
            | EditorRequest::SetViewportBounds(_)
            | EditorRequest::ViewportInput(_)
            | EditorRequest::SetGizmoMode(_)
            | EditorRequest::SetGizmoSpace(_)
            | EditorRequest::SetSnapping(_)
            | EditorRequest::FrameSelected
            | EditorRequest::SetCamera(_)
            | EditorRequest::SetGizmos(_) => RequestDomain::Viewport,
            EditorRequest::SelectAsset(_)
            | EditorRequest::SetAssetBrowser(_)
            | EditorRequest::RefreshAssets
            | EditorRequest::RevealProject
            | EditorRequest::RevealAssetFolder(_)
            | EditorRequest::RevealAsset(_)
            | EditorRequest::OpenAsset(_)
            | EditorRequest::ImportAsset(_)
            | EditorRequest::CreateAssetFolder(_)
            | EditorRequest::RenameAssetFolder(_)
            | EditorRequest::DeleteAssetFolder(_)
            | EditorRequest::CreateMaterial(_)
            | EditorRequest::CreatePrefab(_)
            | EditorRequest::InstantiatePrefab(_)
            | EditorRequest::UnpackPrefab(_)
            | EditorRequest::DuplicateAsset(_)
            | EditorRequest::MoveAsset(_)
            | EditorRequest::DeleteAsset(_)
            | EditorRequest::AssignAsset(_)
            | EditorRequest::OpenMaterial(_)
            | EditorRequest::SetMaterialParameter(_)
            | EditorRequest::SaveMaterial
            | EditorRequest::AssignOpenMaterial => RequestDomain::Asset,
            EditorRequest::SetAnimation(_)
            | EditorRequest::ReplayTerrainSeed(_)
            | EditorRequest::RegenerateTerrain
            | EditorRequest::RetryTerrain
            | EditorRequest::ClearDiagnostics
            | EditorRequest::ExportDiagnostics
            | EditorRequest::StartBuild(_)
            | EditorRequest::CancelBuild
            | EditorRequest::RunProject
            | EditorRequest::CreateProject(_)
            | EditorRequest::OpenProject(_)
            | EditorRequest::SaveProjectSettings(_)
            | EditorRequest::ReplaceInputMap(_)
            | EditorRequest::SaveInputMap
            | EditorRequest::CreateScript(_)
            | EditorRequest::RebuildScripts
            | EditorRequest::AttachScript(_)
            | EditorRequest::PersistLayout(_) => RequestDomain::Project,
        };

        match domain {
            RequestDomain::Document => self.dispatch_document_request(request),
            RequestDomain::Scene => self.dispatch_scene_request(request),
            RequestDomain::Viewport => self.dispatch_viewport_request(request),
            RequestDomain::Asset => self.dispatch_asset_request(request),
            RequestDomain::Project => self.dispatch_project_request(request),
        }
    }
}
