/// A declarative change to Scene View state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SceneViewAction {
    SetPitch(f32),
    SetYaw(f32),
    SetDistance(f32),
}

impl SceneViewAction {
    pub fn affects_camera(self) -> bool {
        matches!(
            self,
            Self::SetPitch(_) | Self::SetYaw(_) | Self::SetDistance(_)
        )
    }
}

/// Persistent state for the editor's Scene View.
///
/// Rendering belongs to the cross-platform React shell. This type deliberately
/// contains no widget or platform-event implementation.
pub struct SceneViewPanel {
    pitch: f32,
    yaw: f32,
    distance: f32,
    target: [f32; 3],
    render_target_label: Option<String>,
    orthographic: bool,
    camera_speed: f32,
}

impl SceneViewPanel {
    pub fn new(_name: impl Into<String>) -> Self {
        Self {
            pitch: 20.0,
            yaw: 45.0,
            distance: 10.0,
            target: [0.0, 0.0, 0.0],
            render_target_label: None,
            orthographic: false,
            camera_speed: 5.0,
        }
    }

    pub fn set_camera_orbit(&mut self, pitch: f32, yaw: f32, distance: f32) {
        self.pitch = pitch.clamp(-89.0, 89.0);
        self.yaw = yaw.clamp(-180.0, 180.0);
        self.distance = distance.clamp(0.1, 100.0);
    }

    pub fn camera_orbit(&self) -> (f32, f32, f32) {
        (self.pitch, self.yaw, self.distance)
    }

    pub fn set_target(&mut self, target: [f32; 3]) {
        self.target = target;
    }

    pub fn target(&self) -> &[f32; 3] {
        &self.target
    }

    pub fn set_orthographic(&mut self, orthographic: bool) {
        self.orthographic = orthographic;
    }

    pub fn orthographic(&self) -> bool {
        self.orthographic
    }

    pub fn set_camera_speed(&mut self, speed: f32) {
        self.camera_speed = speed.clamp(0.1, 100.0);
    }

    pub fn camera_speed(&self) -> f32 {
        self.camera_speed
    }

    pub fn set_render_target(&mut self, label: Option<String>) {
        self.render_target_label = label;
    }

    pub fn render_target(&self) -> Option<&str> {
        self.render_target_label.as_deref()
    }

    pub fn apply_action(&mut self, action: SceneViewAction) {
        match action {
            SceneViewAction::SetPitch(value) => self.pitch = value.clamp(-89.0, 89.0),
            SceneViewAction::SetYaw(value) => self.yaw = value.clamp(-180.0, 180.0),
            SceneViewAction::SetDistance(value) => self.distance = value.clamp(0.1, 100.0),
        }
    }
}

impl Default for SceneViewPanel {
    fn default() -> Self {
        Self::new("Scene View")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene_view_state_clamps_camera_inputs() {
        let mut view = SceneViewPanel::default();
        view.set_camera_orbit(120.0, 240.0, 0.0);
        assert_eq!(view.camera_orbit(), (89.0, 180.0, 0.1));
    }

    #[test]
    fn scene_view_actions_are_declarative() {
        let mut view = SceneViewPanel::default();
        view.apply_action(SceneViewAction::SetDistance(25.0));
        assert_eq!(view.camera_orbit().2, 25.0);
    }

    #[test]
    fn scene_view_projection_and_fly_speed_are_persistent_clamped_state() {
        let mut view = SceneViewPanel::default();
        view.set_orthographic(true);
        view.set_camera_speed(500.0);
        assert!(view.orthographic());
        assert_eq!(view.camera_speed(), 100.0);
        view.set_camera_speed(0.0);
        assert_eq!(view.camera_speed(), 0.1);
    }
}
