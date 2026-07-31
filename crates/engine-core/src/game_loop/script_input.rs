use super::*;

#[cfg(all(feature = "subsystem-scripting-csharp", feature = "subsystem-gameplay"))]
pub(super) fn script_input_value_is_active(value: &engine_script::GameplayInputValue) -> bool {
    match value {
        engine_script::GameplayInputValue::Bool(value) => *value,
        engine_script::GameplayInputValue::Float(value) => value.abs() > 0.5,
        engine_script::GameplayInputValue::Vec2(value) => {
            value[0] * value[0] + value[1] * value[1] > 0.25
        }
    }
}

impl GameLoop {
    /// Set the pixel extent used by script pointer and camera projection data.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn set_script_viewport_size(&mut self, width: u32, height: u32) {
        self.script_pointer.viewport = [width.max(1) as f32, height.max(1) as f32];
        self.update_script_pointer_inside();
    }

    /// Configure the trusted directory used by script save slots.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn set_script_save_directory(&mut self, directory: impl Into<std::path::PathBuf>) {
        self.script_save_directory = Some(directory.into());
    }

    /// Update the gameplay pointer without coupling game input to retained UI.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn script_pointer_move(&mut self, x: f32, y: f32) {
        if !x.is_finite() || !y.is_finite() {
            self.script_pointer_focus(false);
            return;
        }
        self.script_pointer.delta[0] += x - self.script_pointer.position[0];
        self.script_pointer.delta[1] += y - self.script_pointer.position[1];
        self.script_pointer.position = [x, y];
        self.update_script_pointer_inside();
    }

    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn script_pointer_primary(&mut self, down: bool) {
        if down && !self.script_pointer.primary_down {
            self.script_pointer.primary_pressed = true;
        } else if !down && self.script_pointer.primary_down {
            self.script_pointer.primary_released = true;
        }
        self.script_pointer.primary_down = down;
    }

    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn script_pointer_secondary(&mut self, down: bool) {
        self.script_pointer.secondary_down = down;
    }

    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn script_pointer_middle(&mut self, down: bool) {
        self.script_pointer.middle_down = down;
    }

    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn script_pointer_scroll(&mut self, x: f32, y: f32) {
        if x.is_finite() && y.is_finite() {
            self.script_pointer.scroll[0] += x;
            self.script_pointer.scroll[1] += y;
        }
    }

    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn script_pointer_focus(&mut self, focused: bool) {
        self.script_pointer.focused = focused;
        if !focused {
            self.script_pointer.primary_released |= self.script_pointer.primary_down;
            self.script_pointer.primary_down = false;
            self.script_pointer.secondary_down = false;
            self.script_pointer.middle_down = false;
            self.script_pointer.inside_viewport = false;
        } else {
            self.update_script_pointer_inside();
        }
    }

    #[cfg(feature = "subsystem-scripting-csharp")]
    pub(super) fn update_script_pointer_inside(&mut self) {
        let [width, height] = self.script_pointer.viewport;
        let [x, y] = self.script_pointer.position;
        self.script_pointer.inside_viewport = self.script_pointer.focused
            && width > 0.0
            && height > 0.0
            && x >= 0.0
            && y >= 0.0
            && x <= width
            && y <= height;
    }

    #[cfg(feature = "subsystem-scripting-csharp")]
    pub(super) fn refresh_script_view_context(&mut self) {
        let [width, height] = self.script_pointer.viewport;
        let viewport = if width > 0.0 && height > 0.0 {
            engine_scene::RenderViewportContext::new(
                width.round() as u32,
                height.round() as u32,
                engine_renderer::Rect::FULL,
            )
            .unwrap_or_default()
        } else {
            engine_scene::RenderViewportContext::default()
        };
        let camera = self
            .runtime
            .with_world(|world| engine_scene::active_camera_view(world, viewport))
            .flatten();
        self.script_pointer.ray_origin = None;
        self.script_pointer.ray_direction = None;
        if self.script_pointer.inside_viewport {
            if let Some((origin, direction)) = camera
                .as_ref()
                .and_then(|camera| camera.screen_ray(self.script_pointer.position))
            {
                self.script_pointer.ray_origin = Some(origin.to_array());
                self.script_pointer.ray_direction = Some(direction.to_array());
            }
        }
        let camera = camera.map(|camera| engine_script::GameplayCameraSnapshot {
            entity_id: camera.entity_id,
            perspective: camera.perspective,
            position: camera.position.to_array(),
            forward: camera.forward.to_array(),
            right: camera.right.to_array(),
            up: camera.up.to_array(),
            viewport: camera.viewport_pixels,
            view_projection: camera.view_projection.to_cols_array(),
            inverse_view_projection: camera.inverse_view_projection.to_cols_array(),
        });
        self.runtime
            .set_script_view_context(self.script_pointer.clone(), camera);
    }

    #[cfg(feature = "subsystem-scripting-csharp")]
    pub(super) fn finish_script_pointer_frame(&mut self) {
        self.script_pointer.delta = [0.0; 2];
        self.script_pointer.scroll = [0.0; 2];
        self.script_pointer.primary_pressed = false;
        self.script_pointer.primary_released = false;
    }

    #[cfg(all(feature = "subsystem-scripting-csharp", feature = "subsystem-gameplay"))]
    pub(super) fn resolved_script_input_actions(
        &self,
    ) -> std::collections::BTreeMap<String, engine_script::GameplayInputValue> {
        self.input_map
            .actions
            .iter()
            .map(|action| {
                let value = match &action.current_value {
                    engine_gameplay::InputValue::Bool(value) => {
                        engine_script::GameplayInputValue::Bool(*value)
                    }
                    engine_gameplay::InputValue::Float(value) => {
                        engine_script::GameplayInputValue::Float(*value)
                    }
                    engine_gameplay::InputValue::Vec2(value) => {
                        engine_script::GameplayInputValue::Vec2(value.to_array())
                    }
                };
                (action.name.clone(), value)
            })
            .collect()
    }

    #[cfg(all(feature = "subsystem-scripting-csharp", feature = "subsystem-gameplay"))]
    pub(super) fn resolved_script_input_transitions(
        &self,
        current: &std::collections::BTreeMap<String, engine_script::GameplayInputValue>,
    ) -> engine_script::GameplayInputTransitions {
        let action_names = self
            .previous_script_input_actions
            .keys()
            .chain(current.keys())
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        let mut transitions = engine_script::GameplayInputTransitions::default();
        for action_name in action_names {
            let was_active = self
                .previous_script_input_actions
                .get(&action_name)
                .is_some_and(script_input_value_is_active);
            let is_active = current
                .get(&action_name)
                .is_some_and(script_input_value_is_active);
            if is_active && !was_active {
                transitions.pressed.insert(action_name);
            } else if was_active && !is_active {
                transitions.released.insert(action_name);
            }
        }
        transitions
    }
}
