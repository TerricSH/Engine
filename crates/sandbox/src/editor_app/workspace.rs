use super::*;

impl EditorApp {
    pub(super) fn persist_workspace_preferences_if_changed(&mut self) {
        let (pitch, yaw, distance) = self.scene_view.camera_orbit();
        self.workspace_preferences.scene_pitch = pitch;
        self.workspace_preferences.scene_yaw = yaw;
        self.workspace_preferences.scene_distance = distance;
        self.workspace_preferences.scene_target = *self.scene_view.target();
        self.workspace_preferences.scene_orthographic = self.scene_view.orthographic();
        self.workspace_preferences.scene_camera_speed = self.scene_view.camera_speed();
        self.workspace_preferences.snapping_enabled = self.gizmo.snapping;
        if self.workspace_preferences == self.saved_workspace_preferences {
            return;
        }
        match save_workspace_preferences(&self.project, &self.workspace_preferences) {
            Ok(()) => self.saved_workspace_preferences = self.workspace_preferences.clone(),
            Err(error) => tracing::warn!(%error, "editor workspace preferences were not saved"),
        }
    }

    pub(super) fn request_ui_open_panel(
        &mut self,
        panel: protocol::UiPanel,
        preferred_zone: protocol::UiDockZone,
    ) {
        let request = protocol::UiOpenPanelParams {
            panel,
            preferred_zone,
        };
        if self.pending_ui_open_panels.last() != Some(&request) {
            self.pending_ui_open_panels.push(request);
        }
    }

    pub(super) fn take_ui_open_panel_events_json(&mut self) -> Vec<String> {
        std::mem::take(&mut self.pending_ui_open_panels)
            .into_iter()
            .map(|params| {
                self.editor_event_sequence = self.editor_event_sequence.wrapping_add(1);
                serde_json::to_string(&protocol::BridgeEvent {
                    protocol: protocol::EDITOR_PROTOCOL,
                    session_id: self.session_id.clone(),
                    sequence: self.editor_event_sequence,
                    revision: self.editor_revision,
                    event: protocol::UI_OPEN_PANEL_EVENT,
                    params,
                })
                .expect("editor UI navigation events must serialize")
            })
            .collect()
    }
}
