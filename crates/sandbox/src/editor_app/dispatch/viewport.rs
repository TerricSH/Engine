//! Play-mode, viewport, camera, and gizmo request handling.

use super::*;

impl EditorApp {
    pub(super) fn dispatch_viewport_request(
        &mut self,
        request: EditorRequest,
    ) -> Result<DispatchOutcome, BridgeError> {
        match request {
            EditorRequest::SetRuntimeMode(params) => {
                self.set_runtime_mode(params.mode)?;
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::StepRuntime => {
                if self.play_session.mode() != EditorPlayMode::Paused {
                    return Err(BridgeError::new(
                        EditorErrorCode::Conflict,
                        "Runtime stepping requires Play mode to be paused",
                    ));
                }
                self.step_play();
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::SetViewportBounds(params) => {
                if !matches!(params.viewport.as_str(), "scene" | "game") {
                    return Err(BridgeError::new(
                        EditorErrorCode::InvalidRequest,
                        format!("Unknown viewport '{}'", params.viewport),
                    ));
                }
                let next_viewport = if params.viewport == "game" {
                    ViewportTab::Game
                } else {
                    ViewportTab::Scene
                };
                if !params.visible {
                    self.cancel_web_viewport_input();
                    if self.viewport_tab == next_viewport {
                        self.web_viewport_rect = ScreenRect::default();
                    }
                    return Ok(DispatchOutcome::accepted(false));
                }
                if self.viewport_tab != next_viewport {
                    self.cancel_web_viewport_input();
                }
                self.web_viewport_rect = params.rect;
                self.viewport_tab = next_viewport;
                Ok(DispatchOutcome::accepted(false))
            }
            EditorRequest::ViewportInput(params) => {
                self.handle_web_viewport_input(params)?;
                Ok(DispatchOutcome::accepted(false))
            }
            EditorRequest::SetGizmoMode(params) => {
                self.gizmo.mode = match params.mode {
                    GizmoModeDto::Move => GizmoMode::Translate,
                    GizmoModeDto::Rotate => GizmoMode::Rotate,
                    GizmoModeDto::Scale => GizmoMode::Scale,
                };
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::SetGizmoSpace(params) => {
                self.gizmo.space = match params.mode {
                    GizmoSpaceDto::Global => GizmoSpace::Global,
                    GizmoSpaceDto::Local => GizmoSpace::Local,
                };
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::SetSnapping(params) => {
                self.gizmo.snapping = params.enabled;
                self.workspace_preferences.snapping_enabled = params.enabled;
                self.persist_workspace_preferences_if_changed();
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::FrameSelected => {
                self.frame_selected()?;
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::SetCamera(params) => {
                if !camera_params_are_finite(&params) {
                    return Err(validation_error(
                        "Scene camera parameters must contain only finite numbers",
                    ));
                }
                self.scene_view
                    .set_camera_orbit(params.pitch, params.yaw, params.distance);
                self.scene_view.set_target(params.target);
                self.scene_view.set_orthographic(params.orthographic);
                self.scene_view.set_camera_speed(params.speed);
                self.persist_workspace_preferences_if_changed();
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::SetGizmos(params) => {
                self.workspace_preferences.gizmos_visible = params.visible;
                self.persist_workspace_preferences_if_changed();
                Ok(DispatchOutcome::accepted(true))
            }
            _ => unreachable!("request routed to the wrong editor IPC domain"),
        }
    }
}
