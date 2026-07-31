//! Browser viewport input translation and scene-camera navigation.

use super::*;

impl EditorApp {
    pub(super) fn handle_web_viewport_input(
        &mut self,
        params: ViewportInputParams,
    ) -> Result<(), BridgeError> {
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
        if self.viewport_tab != next_viewport {
            self.cancel_web_viewport_input();
            self.viewport_tab = next_viewport;
        }
        match params.event {
            ViewportInput::PointerDown {
                pointer_id,
                x,
                y,
                button,
                buttons,
                modifiers,
            } => {
                let pointer = self.web_pointer_to_physical(x, y);
                self.web_viewport_input.pointer_id = Some(pointer_id);
                self.web_viewport_input.pointer = Some(pointer);
                self.web_viewport_input.buttons = buttons;
                self.web_viewport_input.modifiers = modifiers;
                if button == 0
                    && self.viewport_tab == ViewportTab::Scene
                    && self.play_session.is_editing()
                {
                    self.gizmo_pointer_events
                        .push(GizmoPointerEvent::Press(pointer));
                }
                #[cfg(feature = "runtime-subsystems")]
                if button == 0 && self.viewport_tab == ViewportTab::Game {
                    let ui_pointer = self.web_viewport_pointer_to_physical(x, y);
                    if let Some(game_loop) = self.game_loop.as_mut() {
                        game_loop.ui_pointer_move(ui_pointer.x, ui_pointer.y);
                        game_loop.ui_pointer_left_press();
                    }
                }
            }
            ViewportInput::PointerUp {
                pointer_id,
                x,
                y,
                button,
                buttons,
                modifiers,
            } => {
                if self
                    .web_viewport_input
                    .pointer_id
                    .is_some_and(|active| active != pointer_id)
                {
                    return Ok(());
                }
                let pointer = self.web_pointer_to_physical(x, y);
                self.web_viewport_input.pointer = Some(pointer);
                self.web_viewport_input.buttons = buttons;
                self.web_viewport_input.modifiers = modifiers;
                if buttons == 0 {
                    self.web_viewport_input.pointer_id = None;
                }
                if button == 0
                    && self.viewport_tab == ViewportTab::Scene
                    && self.play_session.is_editing()
                {
                    self.gizmo_pointer_events
                        .push(GizmoPointerEvent::Release(pointer));
                }
                #[cfg(feature = "runtime-subsystems")]
                if button == 0 && self.viewport_tab == ViewportTab::Game {
                    let ui_pointer = self.web_viewport_pointer_to_physical(x, y);
                    if let Some(game_loop) = self.game_loop.as_mut() {
                        game_loop.ui_pointer_move(ui_pointer.x, ui_pointer.y);
                        game_loop.ui_pointer_left_release();
                    }
                }
            }
            ViewportInput::PointerMove {
                pointer_id,
                x,
                y,
                button,
                buttons,
                modifiers,
            } => {
                if self
                    .web_viewport_input
                    .pointer_id
                    .is_some_and(|active| active != pointer_id)
                {
                    return Ok(());
                }
                let pointer = self.web_pointer_to_physical(x, y);
                if let Some(previous) = self.web_viewport_input.pointer {
                    let scale = self.window_scale_factor as f32;
                    let delta = (pointer - previous) / scale.max(f32::EPSILON);
                    let (pitch, yaw, distance) = self.scene_view.camera_orbit();
                    let orbit_sensitivity = if modifiers.shift { 0.05 } else { 0.2 };
                    if buttons & 2 != 0 && self.viewport_tab == ViewportTab::Scene {
                        self.scene_view.set_camera_orbit(
                            pitch + delta.y * orbit_sensitivity,
                            yaw + delta.x * orbit_sensitivity,
                            distance,
                        );
                    }
                    if buttons & 4 != 0 && self.viewport_tab == ViewportTab::Scene {
                        let yaw_radians = yaw.to_radians();
                        let right = Vec3::new(-yaw_radians.sin(), 0.0, yaw_radians.cos());
                        let pan_scale = distance.max(0.1) * 0.0025;
                        let target = Vec3::from_array(*self.scene_view.target())
                            - right * delta.x * pan_scale
                            + Vec3::Y * delta.y * pan_scale;
                        self.scene_view.set_target(target.to_array());
                    }
                }
                self.web_viewport_input.pointer = Some(pointer);
                self.web_viewport_input.buttons = buttons;
                self.web_viewport_input.modifiers = modifiers;
                if (buttons & 1 != 0 || button == 0)
                    && self.viewport_tab == ViewportTab::Scene
                    && self.play_session.is_editing()
                {
                    self.gizmo_pointer_events
                        .push(GizmoPointerEvent::Move(pointer));
                }
                #[cfg(feature = "runtime-subsystems")]
                if self.viewport_tab == ViewportTab::Game {
                    let ui_pointer = self.web_viewport_pointer_to_physical(x, y);
                    if let Some(game_loop) = self.game_loop.as_mut() {
                        game_loop.ui_pointer_move(ui_pointer.x, ui_pointer.y);
                    }
                }
            }
            ViewportInput::PointerCancel { pointer_id } => {
                if self
                    .web_viewport_input
                    .pointer_id
                    .is_none_or(|active| active == pointer_id)
                {
                    self.cancel_web_viewport_input();
                }
            }
            ViewportInput::Wheel {
                x,
                y,
                delta_x,
                delta_y,
                delta_mode,
                modifiers,
            } => {
                self.web_viewport_input.modifiers = modifiers;
                if self.viewport_tab == ViewportTab::Scene {
                    let (pitch, yaw, distance) = self.scene_view.camera_orbit();
                    let unit_scale = match delta_mode {
                        0 => 1.0,
                        1 => 16.0,
                        _ => self.web_viewport_rect.height.max(1.0),
                    };
                    self.scene_view.set_camera_orbit(
                        pitch,
                        yaw + delta_x * unit_scale * 0.01,
                        distance * (-delta_y * unit_scale * 0.0025).exp(),
                    );
                }
                #[cfg(feature = "runtime-subsystems")]
                if self.viewport_tab == ViewportTab::Game {
                    let ui_pointer = self.web_viewport_pointer_to_physical(x, y);
                    if let Some(game_loop) = self.game_loop.as_mut() {
                        game_loop.ui_pointer_move(ui_pointer.x, ui_pointer.y);
                    }
                }
            }
            ViewportInput::KeyDown {
                key,
                code,
                repeat,
                modifiers,
            } => {
                if !repeat {
                    self.web_viewport_input.keys.insert(code.clone());
                }
                self.web_viewport_input.modifiers = modifiers;
                if key == "Escape" {
                    self.gizmo_pointer_events.push(GizmoPointerEvent::Cancel);
                }
                self.route_web_key(&code, true, modifiers);
            }
            ViewportInput::KeyUp {
                key,
                code,
                repeat,
                modifiers,
            } => {
                self.web_viewport_input.keys.remove(&code);
                self.web_viewport_input.modifiers = modifiers;
                if !repeat || key == "Escape" {
                    self.route_web_key(&code, false, modifiers);
                }
            }
            ViewportInput::Focus => self.web_viewport_input.focused = true,
            ViewportInput::Blur => self.cancel_web_viewport_input(),
        }
        Ok(())
    }

    pub(super) fn web_pointer_to_physical(&self, x: f32, y: f32) -> Vec2 {
        css_pointer_to_physical(
            self.web_viewport_rect.x + x,
            self.web_viewport_rect.y + y,
            self.window_scale_factor,
        )
    }

    pub(super) fn web_viewport_pointer_to_physical(&self, x: f32, y: f32) -> Vec2 {
        css_pointer_to_physical(x, y, self.window_scale_factor)
    }

    pub(in super::super) fn cancel_web_viewport_input(&mut self) {
        self.web_viewport_input.pointer_id = None;
        self.web_viewport_input.pointer = None;
        self.web_viewport_input.buttons = 0;
        self.web_viewport_input.modifiers = InputModifiers::default();
        self.web_viewport_input.keys.clear();
        self.web_viewport_input.focused = false;
        self.gizmo_pointer_events.push(GizmoPointerEvent::Cancel);
        #[cfg(feature = "target-desktop")]
        if let Some(game_loop) = self.game_loop.as_mut() {
            self.input_state.reset(&mut game_loop.input_map);
        }
        #[cfg(feature = "runtime-subsystems")]
        if let Some(game_loop) = self.game_loop.as_mut() {
            game_loop.cancel_ui_pointer();
        }
    }

    pub(super) fn route_web_key(&mut self, code: &str, pressed: bool, modifiers: InputModifiers) {
        if self.play_session.is_editing() {
            return;
        }
        let Some(key) = web_key_code(code) else {
            return;
        };
        #[cfg(not(feature = "target-desktop"))]
        let _ = (key, pressed, modifiers);
        #[cfg(feature = "target-desktop")]
        if let Some(game_loop) = self.game_loop.as_mut() {
            self.input_state.apply_platform_event(
                &mut game_loop.input_map,
                &if pressed {
                    PlatformEvent::KeyPressed {
                        key,
                        modifiers: web_modifiers(modifiers),
                    }
                } else {
                    PlatformEvent::KeyReleased {
                        key,
                        modifiers: web_modifiers(modifiers),
                    }
                },
            );
        }
    }

    pub(in super::super) fn tick_web_viewport_camera(&mut self, delta_seconds: f32) {
        if self.viewport_tab != ViewportTab::Scene || self.web_viewport_input.buttons & 2 == 0 {
            return;
        }
        let mut movement = Vec3::ZERO;
        let keys = &self.web_viewport_input.keys;
        movement.z += if keys.contains("KeyW") { 1.0 } else { 0.0 };
        movement.z -= if keys.contains("KeyS") { 1.0 } else { 0.0 };
        movement.x += if keys.contains("KeyD") { 1.0 } else { 0.0 };
        movement.x -= if keys.contains("KeyA") { 1.0 } else { 0.0 };
        movement.y += if keys.contains("KeyE") { 1.0 } else { 0.0 };
        movement.y -= if keys.contains("KeyQ") { 1.0 } else { 0.0 };
        if movement.length_squared() <= f32::EPSILON {
            return;
        }
        let (_, yaw, _) = self.scene_view.camera_orbit();
        let yaw_radians = yaw.to_radians();
        let forward = Vec3::new(-yaw_radians.cos(), 0.0, -yaw_radians.sin());
        let right = Vec3::new(-yaw_radians.sin(), 0.0, yaw_radians.cos());
        let speed_multiplier = if self.web_viewport_input.modifiers.shift {
            3.0
        } else if self.web_viewport_input.modifiers.control {
            0.25
        } else {
            1.0
        };
        let world_delta = (right * movement.x + Vec3::Y * movement.y + forward * movement.z)
            .normalize_or_zero()
            * self.scene_view.camera_speed()
            * speed_multiplier
            * delta_seconds.min(0.1);
        let target = Vec3::from_array(*self.scene_view.target()) + world_delta;
        self.scene_view.set_target(target.to_array());
    }
}
