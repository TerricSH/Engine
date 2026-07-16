use std::collections::HashSet;

#[cfg(any(test, feature = "backend-vulkan"))]
use std::collections::HashMap;

use engine_asset::project::GameProject;
#[cfg(any(test, feature = "backend-vulkan"))]
use engine_gameplay::input::{resolve_action, set_current_value, RawInputEvent};
use engine_gameplay::input::{
    InputAction, InputActionMap, InputBinding, InputDevice, InputModifier, InputValue,
    InputValueType, KeyCode,
};
#[cfg(any(test, feature = "backend-vulkan"))]
use platform::{MouseButton, PlatformEvent};
use serde::{Deserialize, Serialize};

const INPUT_ACTIONS_SCHEMA: &str = "InputActions-v0";

#[derive(Debug, Serialize, Deserialize)]
struct InputActionsDocument {
    schema: String,
    map: InputActionMap,
}

/// Starter input map written by `project new`.
pub(crate) fn starter_input_map() -> InputActionMap {
    let mut map = InputActionMap::new("player", "gameplay");
    for (name, keys) in [
        ("move_forward", vec![KeyCode::W, KeyCode::Up]),
        ("move_backward", vec![KeyCode::S, KeyCode::Down]),
        ("move_left", vec![KeyCode::A, KeyCode::Left]),
        ("move_right", vec![KeyCode::D, KeyCode::Right]),
        ("jump", vec![KeyCode::Space]),
        ("fire", vec![KeyCode::MouseLeft]),
    ] {
        let mut action = InputAction::new(name, InputValueType::Digital);
        action.add_binding(InputBinding::keyboard(name, keys));
        map.add_action(action);
    }
    map
}

pub(crate) fn starter_input_json() -> String {
    let document = InputActionsDocument {
        schema: INPUT_ACTIONS_SCHEMA.to_string(),
        map: starter_input_map(),
    };
    let mut json = serde_json::to_string_pretty(&document)
        .expect("starter input action document must serialize");
    json.push('\n');
    json
}

pub(crate) fn load_project_input_map(project: &GameProject) -> Result<InputActionMap, String> {
    let Some(path) = project.input_actions.as_ref() else {
        return Ok(InputActionMap::new("player", "gameplay"));
    };
    let json = std::fs::read_to_string(path).map_err(|error| {
        format!(
            "could not read input action map {}: {error}",
            path.display()
        )
    })?;
    let document: InputActionsDocument = serde_json::from_str(&json)
        .map_err(|error| format!("invalid input action map {}: {error}", path.display()))?;
    if document.schema != INPUT_ACTIONS_SCHEMA {
        return Err(format!(
            "unsupported input action schema '{}' in {}",
            document.schema,
            path.display()
        ));
    }
    let mut map = document.map;
    validate_input_map(&map)
        .map_err(|error| format!("invalid input action map {}: {error}", path.display()))?;
    reset_current_values(&mut map);
    Ok(map)
}

fn reset_current_values(map: &mut InputActionMap) {
    for action in &mut map.actions {
        action.current_value = match action.value_type {
            InputValueType::Digital => InputValue::Bool(false),
            InputValueType::Analog1D => InputValue::Float(0.0),
            InputValueType::Analog2D => InputValue::Vec2(glam::Vec2::ZERO),
        };
    }
}

pub(crate) fn validate_input_map(map: &InputActionMap) -> Result<(), String> {
    if map.name.trim().is_empty() {
        return Err("map name cannot be empty".into());
    }
    if map.context.trim().is_empty() {
        return Err("map context cannot be empty".into());
    }
    if map.actions.is_empty() {
        return Err("at least one input action is required".into());
    }

    let mut names = HashSet::new();
    for action in &map.actions {
        if action.name.trim().is_empty() {
            return Err("action name cannot be empty".into());
        }
        if !names.insert(action.name.as_str()) {
            return Err(format!("duplicate action name '{}'", action.name));
        }
        if action.bindings.is_empty() {
            return Err(format!("action '{}' has no bindings", action.name));
        }
        let current_type_matches = matches!(
            (action.value_type, &action.current_value),
            (InputValueType::Digital, InputValue::Bool(_))
                | (InputValueType::Analog1D, InputValue::Float(_))
                | (InputValueType::Analog2D, InputValue::Vec2(_))
        );
        if !current_type_matches {
            return Err(format!(
                "action '{}' current value does not match its value type",
                action.name
            ));
        }
        if action.value_type == InputValueType::Analog2D
            && action
                .bindings
                .iter()
                .any(|binding| !binding.keys.is_empty())
        {
            return Err(format!(
                "action '{}' uses keyboard bindings for Analog2D, which InputActions-v0 cannot express safely",
                action.name
            ));
        }
        for binding in &action.bindings {
            if binding.action != action.name {
                return Err(format!(
                    "binding action '{}' does not match owner '{}'",
                    binding.action, action.name
                ));
            }
            let source_count = usize::from(!binding.keys.is_empty())
                + usize::from(binding.gamepad_button.is_some())
                + usize::from(binding.gamepad_axis.is_some());
            if source_count == 0 {
                return Err(format!("action '{}' has an empty binding", action.name));
            }
            if source_count > 1 {
                return Err(format!(
                    "action '{}' binding mixes multiple input sources",
                    action.name
                ));
            }
            if !binding.keys.is_empty() && binding.device != InputDevice::KeyboardMouse {
                return Err(format!(
                    "action '{}' key binding must use KeyboardMouse",
                    action.name
                ));
            }
            if (binding.gamepad_button.is_some() || binding.gamepad_axis.is_some())
                && binding.device != InputDevice::Gamepad
            {
                return Err(format!(
                    "action '{}' gamepad binding must use Gamepad",
                    action.name
                ));
            }
            let mut keys = HashSet::new();
            if binding.keys.iter().any(|key| !keys.insert(*key)) {
                return Err(format!(
                    "action '{}' binding contains a duplicate key",
                    action.name
                ));
            }
            match binding.modifier {
                InputModifier::Deadzone(threshold)
                    if !threshold.is_finite() || !(0.0..=1.0).contains(&threshold) =>
                {
                    return Err(format!("action '{}' has an invalid deadzone", action.name));
                }
                InputModifier::Scale(factor) if !factor.is_finite() => {
                    return Err(format!("action '{}' has a non-finite scale", action.name));
                }
                _ => {}
            }
        }
    }
    Ok(())
}

/// Stateful bridge from platform press/release events to an action map.
#[derive(Default)]
#[cfg(any(test, feature = "backend-vulkan"))]
pub(crate) struct ProjectInputState {
    held: HashMap<KeyCode, f32>,
}

#[cfg(any(test, feature = "backend-vulkan"))]
impl ProjectInputState {
    #[cfg(all(feature = "tooling-editor", feature = "backend-vulkan"))]
    pub(crate) fn reset(&mut self, map: &mut InputActionMap) {
        self.held.clear();
        self.recompute(map);
    }

    /// Apply one platform event and recompute every current action value.
    /// Returns whether the event affected the input state.
    pub(crate) fn apply_platform_event(
        &mut self,
        map: &mut InputActionMap,
        event: &PlatformEvent,
    ) -> bool {
        let changed = match event {
            PlatformEvent::KeyPressed { key, .. } => map_platform_key(*key)
                .map(|key| self.held.insert(key, 1.0) != Some(1.0))
                .unwrap_or(false),
            PlatformEvent::KeyReleased { key, .. } => map_platform_key(*key)
                .map(|key| self.held.remove(&key).is_some())
                .unwrap_or(false),
            PlatformEvent::MousePressed { button, .. } => map_mouse_button(*button)
                .map(|key| self.held.insert(key, 1.0) != Some(1.0))
                .unwrap_or(false),
            PlatformEvent::MouseReleased { button, .. } => map_mouse_button(*button)
                .map(|key| self.held.remove(&key).is_some())
                .unwrap_or(false),
            PlatformEvent::Suspended | PlatformEvent::Focused(false) => {
                let had_input = !self.held.is_empty();
                self.held.clear();
                had_input
            }
            _ => false,
        };
        if changed {
            self.recompute(map);
        }
        changed
    }

    fn recompute(&self, map: &mut InputActionMap) {
        let events = self
            .held
            .iter()
            .map(|(key, value)| RawInputEvent::keyboard(*key, *value))
            .collect::<Vec<_>>();
        let actions = map
            .actions
            .iter()
            .map(|action| (action.name.clone(), action.value_type))
            .collect::<Vec<_>>();
        for (name, value_type) in actions {
            let value = resolve_action(map, &events, &name).unwrap_or(match value_type {
                InputValueType::Digital => InputValue::Bool(false),
                InputValueType::Analog1D => InputValue::Float(0.0),
                InputValueType::Analog2D => InputValue::Vec2(glam::Vec2::ZERO),
            });
            set_current_value(map, &name, value);
        }
    }
}

#[cfg(any(test, feature = "backend-vulkan"))]
fn map_mouse_button(button: MouseButton) -> Option<KeyCode> {
    match button {
        MouseButton::Left => Some(KeyCode::MouseLeft),
        MouseButton::Right => Some(KeyCode::MouseRight),
        MouseButton::Middle => Some(KeyCode::MouseMiddle),
        MouseButton::Other(_) => None,
    }
}

#[cfg(any(test, feature = "backend-vulkan"))]
fn map_platform_key(key: platform::KeyCode) -> Option<KeyCode> {
    use platform::KeyCode as P;
    Some(match key {
        P::Q => KeyCode::Q,
        P::W => KeyCode::W,
        P::E => KeyCode::E,
        P::R => KeyCode::R,
        P::T => KeyCode::T,
        P::Y => KeyCode::Y,
        P::U => KeyCode::U,
        P::I => KeyCode::I,
        P::O => KeyCode::O,
        P::P => KeyCode::P,
        P::A => KeyCode::A,
        P::S => KeyCode::S,
        P::D => KeyCode::D,
        P::F => KeyCode::F,
        P::G => KeyCode::G,
        P::H => KeyCode::H,
        P::J => KeyCode::J,
        P::K => KeyCode::K,
        P::L => KeyCode::L,
        P::Z => KeyCode::Z,
        P::X => KeyCode::X,
        P::C => KeyCode::C,
        P::V => KeyCode::V,
        P::B => KeyCode::B,
        P::N => KeyCode::N,
        P::M => KeyCode::M,
        P::Key0 => KeyCode::Digit0,
        P::Key1 => KeyCode::Digit1,
        P::Key2 => KeyCode::Digit2,
        P::Key3 => KeyCode::Digit3,
        P::Key4 => KeyCode::Digit4,
        P::Key5 => KeyCode::Digit5,
        P::Key6 => KeyCode::Digit6,
        P::Key7 => KeyCode::Digit7,
        P::Key8 => KeyCode::Digit8,
        P::Key9 => KeyCode::Digit9,
        P::Space => KeyCode::Space,
        P::Enter => KeyCode::Enter,
        P::Escape => KeyCode::Escape,
        P::Tab => KeyCode::Tab,
        P::Backspace => KeyCode::Backspace,
        P::Delete => KeyCode::Delete,
        P::Up => KeyCode::Up,
        P::Down => KeyCode::Down,
        P::Left => KeyCode::Left,
        P::Right => KeyCode::Right,
        P::LShift => KeyCode::ShiftLeft,
        P::RShift => KeyCode::ShiftRight,
        P::LControl => KeyCode::ControlLeft,
        P::RControl => KeyCode::ControlRight,
        P::LAlt => KeyCode::AltLeft,
        P::RAlt => KeyCode::AltRight,
        P::F1
        | P::F2
        | P::F3
        | P::F4
        | P::F5
        | P::F6
        | P::F7
        | P::F8
        | P::F9
        | P::F10
        | P::F11
        | P::F12
        | P::Other(_) => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_gameplay::input::query_current_value;
    use platform::Modifiers;

    #[test]
    fn starter_map_roundtrips_and_validates() {
        let document: InputActionsDocument = serde_json::from_str(&starter_input_json()).unwrap();
        assert_eq!(document.schema, INPUT_ACTIONS_SCHEMA);
        validate_input_map(&document.map).unwrap();
        assert_eq!(document.map.actions.len(), 6);
    }

    #[test]
    fn persisted_current_values_are_reset_before_runtime_use() {
        let mut map = starter_input_map();
        map.action_mut("jump").unwrap().current_value = InputValue::Bool(true);
        reset_current_values(&mut map);
        assert_eq!(
            query_current_value(&map, "jump"),
            Some(&InputValue::Bool(false))
        );
    }

    #[test]
    fn key_and_mouse_press_release_update_actions() {
        let mut map = starter_input_map();
        let mut state = ProjectInputState::default();
        assert!(state.apply_platform_event(
            &mut map,
            &PlatformEvent::KeyPressed {
                key: platform::KeyCode::W,
                modifiers: Modifiers::default(),
            },
        ));
        assert_eq!(
            query_current_value(&map, "move_forward"),
            Some(&InputValue::Bool(true))
        );
        state.apply_platform_event(
            &mut map,
            &PlatformEvent::KeyReleased {
                key: platform::KeyCode::W,
                modifiers: Modifiers::default(),
            },
        );
        assert_eq!(
            query_current_value(&map, "move_forward"),
            Some(&InputValue::Bool(false))
        );

        state.apply_platform_event(
            &mut map,
            &PlatformEvent::MousePressed {
                button: MouseButton::Left,
                x: 10.0,
                y: 20.0,
            },
        );
        assert_eq!(
            query_current_value(&map, "fire"),
            Some(&InputValue::Bool(true))
        );
        state.apply_platform_event(&mut map, &PlatformEvent::Focused(false));
        assert_eq!(
            query_current_value(&map, "fire"),
            Some(&InputValue::Bool(false))
        );
    }

    #[test]
    fn alternate_bound_key_keeps_action_pressed() {
        let mut map = starter_input_map();
        let mut state = ProjectInputState::default();
        for key in [platform::KeyCode::W, platform::KeyCode::Up] {
            state.apply_platform_event(
                &mut map,
                &PlatformEvent::KeyPressed {
                    key,
                    modifiers: Modifiers::default(),
                },
            );
        }
        state.apply_platform_event(
            &mut map,
            &PlatformEvent::KeyReleased {
                key: platform::KeyCode::W,
                modifiers: Modifiers::default(),
            },
        );
        assert_eq!(
            query_current_value(&map, "move_forward"),
            Some(&InputValue::Bool(true))
        );
    }
}
