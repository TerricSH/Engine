//! Editor session and scene-document lifecycle request handling.

use super::*;

impl EditorApp {
    pub(super) fn dispatch_document_request(
        &mut self,
        request: EditorRequest,
    ) -> Result<DispatchOutcome, BridgeError> {
        match request {
            EditorRequest::Ready(params) => {
                if params.protocol_version != EDITOR_PROTOCOL_VERSION {
                    return Err(BridgeError::new(
                        EditorErrorCode::ProtocolMismatch,
                        format!(
                            "React editor protocol {} is incompatible with host protocol {}",
                            params.protocol_version, EDITOR_PROTOCOL_VERSION
                        ),
                    ));
                }
                tracing::info!(
                    client_version = params.client_version.as_deref().unwrap_or("unspecified"),
                    protocol_version = params.protocol_version,
                    "React editor IPC session established"
                );
                Ok(DispatchOutcome::result(
                    serde_json::to_value(self.editor_snapshot()).map_err(internal_error)?,
                    false,
                ))
            }
            EditorRequest::GetSnapshot => Ok(DispatchOutcome::result(
                serde_json::to_value(self.editor_snapshot()).map_err(internal_error)?,
                false,
            )),
            EditorRequest::RequestExit => {
                self.request_editor_exit();
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::SaveDocument => {
                self.require_editing()?;
                self.save_current_scene_document().map_err(io_error)?;
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::OpenDocument(params) => {
                self.require_editing()?;
                self.apply_scene_document_action(SceneDocumentAction::Open(params.scene_id))
                    .map_err(validation_error)?;
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::CreateDocument(params) => {
                self.require_editing()?;
                self.apply_scene_document_action(SceneDocumentAction::Create {
                    scene_id: params.scene_id,
                    folder: PathBuf::from(params.folder),
                })
                .map_err(validation_error)?;
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::SaveDocumentAs(params) => {
                self.require_editing()?;
                self.apply_scene_document_action(SceneDocumentAction::SaveAs(params.scene_id))
                    .map_err(validation_error)?;
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::DuplicateDocument(params) => {
                self.require_editing()?;
                self.apply_scene_document_action(SceneDocumentAction::Duplicate {
                    source_id: params.source_id,
                    new_id: params.new_id,
                })
                .map_err(validation_error)?;
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::RenameDocument(params) => {
                self.require_editing()?;
                self.apply_scene_document_action(SceneDocumentAction::Rename {
                    old_id: params.old_id,
                    new_id: params.new_id,
                })
                .map_err(validation_error)?;
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::DeleteDocument(params) => {
                self.require_editing()?;
                self.apply_scene_document_action(SceneDocumentAction::Delete {
                    scene_id: params.scene_id,
                    replacement_startup: params.replacement_startup,
                })
                .map_err(validation_error)?;
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::SetStartupDocument(params) => {
                self.require_editing()?;
                self.apply_scene_document_action(SceneDocumentAction::SetStartup(params.scene_id))
                    .map_err(validation_error)?;
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::ResolvePendingSwitch(params) => {
                self.require_editing()?;
                self.pending_scene_switch.as_ref().ok_or_else(|| {
                    BridgeError::new(EditorErrorCode::Conflict, "No scene switch is pending")
                })?;
                let pending = self.pending_document_action.clone().ok_or_else(|| {
                    BridgeError::new(
                        EditorErrorCode::Conflict,
                        "The pending scene action was lost; cancel and retry the operation",
                    )
                })?;
                match params.decision {
                    SaveDiscardCancel::Save => {
                        self.save_current_scene_document().map_err(io_error)?;
                        self.apply_scene_document_action_after_confirmation(pending, true)
                            .map_err(validation_error)?;
                    }
                    SaveDiscardCancel::Discard => {
                        self.apply_scene_document_action_after_confirmation(pending, true)
                            .map_err(validation_error)?;
                    }
                    SaveDiscardCancel::Cancel => {
                        self.apply_scene_document_action(SceneDocumentAction::CancelSwitch)
                            .map_err(validation_error)?;
                    }
                }
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::ResolveClose(params) => {
                if !self.close_confirmation_pending {
                    return Err(BridgeError::new(
                        EditorErrorCode::Conflict,
                        "No editor close confirmation is pending",
                    ));
                }
                let action = match params.decision {
                    SaveDiscardCancel::Save => CloseDocumentAction::SaveAndClose,
                    SaveDiscardCancel::Discard => CloseDocumentAction::DiscardAndClose,
                    SaveDiscardCancel::Cancel => CloseDocumentAction::Cancel,
                };
                self.apply_close_document_action(action).map_err(io_error)?;
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::ResolveRecovery(params) => {
                match params.decision {
                    RecoveryDecision::Restore => self.restore_recovery_snapshot(),
                    RecoveryDecision::Discard => self.discard_recovery_snapshot(),
                }
                .map_err(io_error)?;
                Ok(DispatchOutcome::accepted(true))
            }
            _ => unreachable!("request routed to the wrong editor IPC domain"),
        }
    }
}
