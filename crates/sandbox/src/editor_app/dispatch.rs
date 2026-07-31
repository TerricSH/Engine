//! The single typed command router for the React editor shell.

use std::collections::BTreeSet;
use std::path::PathBuf;

use engine_editor::asset_browser::AssetKindFilter;
use engine_editor::component_catalog::ComponentCatalog;
use engine_editor::material_editor::ShaderParamType;
use engine_editor::{
    AddComponent, Command, CommandBatch, EntityPasteParent, MoveEntitySibling, RemoveComponent,
    ReplaceComponent, SetComponentEnabled, SetComponentField, SetEntityEnabled, SetEntityName,
    SetEntityParent, SetSceneSettings, SiblingMove,
};
#[cfg(feature = "target-desktop")]
use platform::PlatformEvent;
use serde_json::{json, Value as JsonValue};

use super::protocol::*;
use super::*;

pub(super) struct DispatchMessages {
    pub json_messages: Vec<String>,
}

struct DispatchOutcome {
    result: JsonValue,
    state_changed: bool,
}

impl DispatchOutcome {
    fn accepted(state_changed: bool) -> Self {
        Self {
            result: json!({ "accepted": true }),
            state_changed,
        }
    }

    fn result(result: JsonValue, state_changed: bool) -> Self {
        Self {
            result,
            state_changed,
        }
    }

    fn job(job_id: u64) -> Self {
        Self::result(json!({ "accepted": true, "jobId": job_id }), true)
    }
}

impl EditorApp {
    pub(super) fn dispatch_ipc_json(&mut self, raw: &str) -> DispatchMessages {
        let parsed = serde_json::from_str::<BridgeRequest>(raw);
        let request = match parsed {
            Ok(request) => request,
            Err(error) => {
                let id = serde_json::from_str::<JsonValue>(raw)
                    .ok()
                    .and_then(|value| {
                        value
                            .get("id")
                            .and_then(JsonValue::as_str)
                            .map(str::to_string)
                    })
                    .unwrap_or_default();
                return DispatchMessages {
                    json_messages: vec![self.error_response(
                        id,
                        BridgeError::new(
                            EditorErrorCode::InvalidRequest,
                            format!("Malformed editor IPC JSON: {error}"),
                        ),
                    )],
                };
            }
        };

        let decoded = match EditorRequest::decode(&request) {
            Ok(decoded) => decoded,
            Err(error) => {
                return DispatchMessages {
                    json_messages: vec![self.error_response(request.id, error)],
                };
            }
        };

        if !matches!(decoded, EditorRequest::Ready(_)) {
            if request.protocol.as_deref() != Some(EDITOR_PROTOCOL) {
                return DispatchMessages {
                    json_messages: vec![self.error_response(
                        request.id,
                        BridgeError::new(
                            EditorErrorCode::ProtocolMismatch,
                            format!("Expected protocol '{EDITOR_PROTOCOL}'"),
                        ),
                    )],
                };
            }
            if request.session_id.as_deref() != Some(self.session_id.as_str()) {
                return DispatchMessages {
                    json_messages: vec![self.error_response(
                        request.id,
                        BridgeError::new(
                            EditorErrorCode::ProtocolMismatch,
                            "The editor session changed; bootstrap again",
                        ),
                    )],
                };
            }
            if request_requires_revision(&decoded)
                && request.base_revision != Some(self.editor_revision)
            {
                let mut error = BridgeError::new(
                    EditorErrorCode::StaleRevision,
                    "The editor changed since this command was created",
                );
                error.current_revision = Some(self.editor_revision);
                return DispatchMessages {
                    json_messages: vec![self.error_response(request.id, error)],
                };
            }
        }

        match self.dispatch_editor_request(decoded) {
            Ok(outcome) => {
                if outcome.state_changed {
                    self.editor_revision = self.editor_revision.wrapping_add(1);
                }
                let response = BridgeResponse {
                    protocol: EDITOR_PROTOCOL,
                    id: request.id,
                    session_id: self.session_id.clone(),
                    revision: self.editor_revision,
                    result: Some(outcome.result),
                    error: None,
                };
                let mut json_messages = vec![serialize_message(&response)];
                if outcome.state_changed {
                    self.editor_event_sequence = self.editor_event_sequence.wrapping_add(1);
                    let event = BridgeEvent {
                        protocol: EDITOR_PROTOCOL,
                        session_id: self.session_id.clone(),
                        sequence: self.editor_event_sequence,
                        revision: self.editor_revision,
                        event: PROJECT_CHANGED_EVENT,
                        params: self.editor_snapshot(),
                    };
                    json_messages.push(serialize_message(&event));
                }
                json_messages.extend(self.take_ui_open_panel_events_json());
                DispatchMessages { json_messages }
            }
            Err(error) => {
                let mut json_messages = vec![self.error_response(request.id, error)];
                json_messages.extend(self.take_ui_open_panel_events_json());
                DispatchMessages { json_messages }
            }
        }
    }

    fn error_response(&self, id: String, error: BridgeError) -> String {
        serialize_message(&BridgeResponse::<JsonValue> {
            protocol: EDITOR_PROTOCOL,
            id,
            session_id: self.session_id.clone(),
            revision: self.editor_revision,
            result: None,
            error: Some(error),
        })
    }
}

mod asset_tools;
mod assets;
mod commands;
mod document;
mod helpers;
mod project;
mod router;
mod scene;
mod viewport;
mod viewport_input;

#[cfg(test)]
mod tests;

use helpers::*;
