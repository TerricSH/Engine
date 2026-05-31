//! # Input Action Maps (G18-F02) & Rebinding Persistence (G18-F03)
//!
//! Platform-agnostic input system with:
//!
//! * **InputActionMap** — a named collection of `InputAction`s, each with
//!   one or more `InputBinding`s.
//! * **RawInputEvent** — a platform-agnostic event that flows from the
//!   engine's input layer into the binding resolver.
//! * **InputValue / InputModifier** — typed result values and transforms
//!   (deadzone, scale, invert).
//! * **Serialization** — JSON roundtrip via `serde` for persistence.
//! * **Rebinding** — replace bindings at runtime with error checking.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Core enums
// ---------------------------------------------------------------------------

/// The resolved value of an input action after binding-resolution and
/// modifier application.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum InputValue {
    /// Digital on/off state (e.g. keyboard key, gamepad button).
    Bool(bool),
    /// Single-axis analog value (e.g. trigger, mouse wheel).
    Float(f32),
    /// Two-axis analog value (e.g. thumbstick, mouse delta).
    Vec2(glam::Vec2),
}

/// The physical input device category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InputDevice {
    KeyboardMouse,
    Gamepad,
    Touch,
}

/// Transform applied to a raw input value before it reaches the action.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum InputModifier {
    /// Pass-through — no modification.
    None,
    /// Negate the value (`-x` for scalars, `-v` for vectors,
    /// `!b` for bools).
    Invert,
    /// Values below `threshold` snap to zero; remaining values are
    /// re-scaled linearly into `[0, 1]`.
    Deadzone(f32),
    /// Multiply the raw value by `factor`.
    Scale(f32),
}

// ---------------------------------------------------------------------------
// KeyCode (unified keyboard, mouse, and gamepad button codes)
// ---------------------------------------------------------------------------

/// A unified key identifier covering keyboard keys, mouse buttons, and
/// gamepad buttons.  This allows `RawInputEvent` to carry any button-like
/// input through a single `Option<KeyCode>` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum KeyCode {
    // -- Keyboard letters (QWERTY row) ---------------------------------------
    Q,
    W,
    E,
    R,
    T,
    Y,
    U,
    I,
    O,
    P,
    A,
    S,
    D,
    F,
    G,
    H,
    J,
    K,
    L,
    Z,
    X,
    C,
    V,
    B,
    N,
    M,
    // -- Keyboard digits -----------------------------------------------------
    Digit0,
    Digit1,
    Digit2,
    Digit3,
    Digit4,
    Digit5,
    Digit6,
    Digit7,
    Digit8,
    Digit9,
    // -- Keyboard specials ---------------------------------------------------
    Space,
    Enter,
    Escape,
    Tab,
    Backspace,
    Delete,
    Up,
    Down,
    Left,
    Right,
    ShiftLeft,
    ShiftRight,
    ControlLeft,
    ControlRight,
    AltLeft,
    AltRight,
    // -- Mouse buttons -------------------------------------------------------
    MouseLeft,
    MouseRight,
    MouseMiddle,
    // -- Gamepad buttons (aliases) -------------------------------------------
    GamepadA,
    GamepadB,
    GamepadX,
    GamepadY,
    GamepadLB,
    GamepadRB,
    GamepadLT,
    GamepadRT,
    GamepadStart,
    GamepadBack,
    GamepadDPadUp,
    GamepadDPadDown,
    GamepadDPadLeft,
    GamepadDPadRight,
}

// ---------------------------------------------------------------------------
// Gamepad-specific types
// ---------------------------------------------------------------------------

/// Logical gamepad face / shoulder buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GamepadButton {
    A,
    B,
    X,
    Y,
    LB,
    RB,
    LT,
    RT,
    Start,
    Back,
    DPadUp,
    DPadDown,
    DPadLeft,
    DPadRight,
}

/// Analog stick and trigger axes on a gamepad.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum GamepadAxis {
    LeftX,
    LeftY,
    RightX,
    RightY,
    LT,
    RT,
}

// ---------------------------------------------------------------------------
// InputBinding
// ---------------------------------------------------------------------------

/// One binding slot inside an `InputAction`.
///
/// The binding describes *what* hardware input triggers the action and
/// *how* the raw value is transformed before reaching gameplay code.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InputBinding {
    /// Which device category this binding listens to.
    pub device: InputDevice,
    /// The action this binding belongs to (used for serialisation context).
    pub action: String,
    /// Keyboard / mouse / gamepad-button key codes that trigger this binding.
    pub keys: Vec<KeyCode>,
    /// If present, this binding only fires for the given gamepad button.
    pub gamepad_button: Option<GamepadButton>,
    /// If present, this binding listens to an analog gamepad axis.
    pub gamepad_axis: Option<GamepadAxis>,
    /// Transform to apply to the raw input value.
    pub modifier: InputModifier,
}

impl InputBinding {
    /// Create a simple keyboard-binding with no modifier.
    pub fn keyboard(action: impl Into<String>, keys: Vec<KeyCode>) -> Self {
        Self {
            device: InputDevice::KeyboardMouse,
            action: action.into(),
            keys,
            gamepad_button: None,
            gamepad_axis: None,
            modifier: InputModifier::None,
        }
    }

    /// Create a gamepad-button binding with no modifier.
    pub fn gamepad_button(action: impl Into<String>, btn: GamepadButton) -> Self {
        Self {
            device: InputDevice::Gamepad,
            action: action.into(),
            keys: Vec::new(),
            gamepad_button: Some(btn),
            gamepad_axis: None,
            modifier: InputModifier::None,
        }
    }

    /// Create a gamepad-axis binding with no modifier.
    pub fn gamepad_axis(action: impl Into<String>, axis: GamepadAxis) -> Self {
        Self {
            device: InputDevice::Gamepad,
            action: action.into(),
            keys: Vec::new(),
            gamepad_button: None,
            gamepad_axis: Some(axis),
            modifier: InputModifier::None,
        }
    }
}

// ---------------------------------------------------------------------------
// InputAction & InputActionMap
// ---------------------------------------------------------------------------

/// Describes the expected dimensionality of an action's value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InputValueType {
    /// Boolean on/off (keys, buttons).
    Digital,
    /// Single-float analog (triggers, scroll wheels).
    Analog1D,
    /// Two-float analog (thumbsticks, mouse delta).
    Analog2D,
}

/// A named action with one or more bindings.
///
/// Each action carries a `current_value` that can be updated by the
/// resolution functions or manually by gameplay code.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InputAction {
    /// Human-readable name (e.g. `"jump"`, `"move_horizontal"`).
    pub name: String,
    /// Ordered list of bindings; the first matching binding wins.
    pub bindings: Vec<InputBinding>,
    /// Semantic type of the value this action produces.
    pub value_type: InputValueType,
    /// The most recently resolved value (initially zero / false).
    pub current_value: InputValue,
}

impl InputAction {
    /// Create a new action with the given name and value type.
    ///
    /// `current_value` is set to the logical zero / false for the type.
    pub fn new(name: impl Into<String>, value_type: InputValueType) -> Self {
        let current_value = match value_type {
            InputValueType::Digital => InputValue::Bool(false),
            InputValueType::Analog1D => InputValue::Float(0.0),
            InputValueType::Analog2D => InputValue::Vec2(glam::Vec2::ZERO),
        };
        Self {
            name: name.into(),
            bindings: Vec::new(),
            value_type,
            current_value,
        }
    }

    /// Add a binding to this action (builder-style).
    pub fn add_binding(&mut self, binding: InputBinding) -> &mut Self {
        self.bindings.push(binding);
        self
    }
}

/// A named collection of input actions, scoped to a gameplay context.
///
/// # Example
///
/// ```
/// use engine_gameplay::input::*;
///
/// let mut map = InputActionMap::new("player", "gameplay");
/// map.add_action(
///     InputAction::new("jump", InputValueType::Digital)
///         .add_binding(InputBinding::keyboard("jump", vec![KeyCode::Space]))
///         .add_binding(InputBinding::gamepad_button("jump", GamepadButton::A))
///         .clone(), // add_action takes ownership
/// );
///
/// let events = [RawInputEvent::keyboard(KeyCode::Space, 1.0)];
/// assert_eq!(
///     resolve_action(&map, &events, "jump"),
///     Some(InputValue::Bool(true))
/// );
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InputActionMap {
    /// Map name (e.g. `"player"`, `"vehicle"`).
    pub name: String,
    /// All actions in this map.
    pub actions: Vec<InputAction>,
    /// Context tag (e.g. `"gameplay"`, `"menu"`) for grouping.
    pub context: String,
}

impl InputActionMap {
    /// Create a new, empty action map.
    pub fn new(name: impl Into<String>, context: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            actions: Vec::new(),
            context: context.into(),
        }
    }

    /// Add an action to this map.
    ///
    /// Returns `&mut self` for chaining.
    pub fn add_action(&mut self, action: InputAction) -> &mut Self {
        self.actions.push(action);
        self
    }

    /// Look up an action by name.
    pub fn action(&self, name: &str) -> Option<&InputAction> {
        self.actions.iter().find(|a| a.name == name)
    }

    /// Mutable lookup of an action by name.
    pub fn action_mut(&mut self, name: &str) -> Option<&mut InputAction> {
        self.actions.iter_mut().find(|a| a.name == name)
    }

    /// Resolve the named action against a batch of raw input events.
    ///
    /// Iterates bindings in order; the first binding that matches any event
    /// wins.  The raw value is then converted to the action's `InputValueType`
    /// and the binding's `InputModifier` is applied.
    ///
    /// For `Analog2D` actions, X and Y components are collected from all
    /// bindings and composed into a single `Vec2`.
    pub fn resolve_binding(
        &self,
        action_name: &str,
        raw_events: &[RawInputEvent],
    ) -> Option<InputValue> {
        resolve_action(self, raw_events, action_name)
    }
}

// ---------------------------------------------------------------------------
// RawInputEvent
// ---------------------------------------------------------------------------

/// A platform-agnostic raw input event.
///
/// The engine's platform layer emits these; the binding resolver consumes them.
///
/// * **Keyboard / mouse click** — set `key = Some(...)`, `value = 1.0`.
/// * **Mouse / scroll / analog button** — set `key = Some(...)`,
///   `value = the_raw_float`.
/// * **Gamepad axis** — set `axis = Some((kind, value))`.
/// * **Gamepad button** — set `key = Some(KeyCode::GamepadA …)`,
///   `value = 1.0` (or 0.0 on release).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawInputEvent {
    /// The device that produced this event.
    pub device: InputDevice,
    /// A button-like key code (keyboard, mouse, or gamepad button).
    pub key: Option<KeyCode>,
    /// An analog axis sample (gamepad sticks and triggers).
    pub axis: Option<(GamepadAxis, f32)>,
    /// Raw float value associated with the key or axis.
    pub value: f32,
}

impl RawInputEvent {
    /// Convenience constructor for a keyboard key press.
    pub fn keyboard(key: KeyCode, value: f32) -> Self {
        Self {
            device: InputDevice::KeyboardMouse,
            key: Some(key),
            axis: None,
            value,
        }
    }

    /// Convenience constructor for a mouse button.
    pub fn mouse(key: KeyCode, value: f32) -> Self {
        Self {
            device: InputDevice::KeyboardMouse,
            key: Some(key),
            axis: None,
            value,
        }
    }

    /// Convenience constructor for a gamepad button press.
    pub fn gamepad_button(key: KeyCode, value: f32) -> Self {
        Self {
            device: InputDevice::Gamepad,
            key: Some(key),
            axis: None,
            value,
        }
    }

    /// Convenience constructor for a gamepad axis sample.
    pub fn gamepad_axis(axis: GamepadAxis, value: f32) -> Self {
        Self {
            device: InputDevice::Gamepad,
            key: None,
            axis: Some((axis, value)),
            value,
        }
    }
}

// ---------------------------------------------------------------------------
// Helper functions (internal)
// ---------------------------------------------------------------------------

/// Map a `GamepadButton` to the equivalent `KeyCode` variant.
fn gamepad_button_to_keycode(btn: GamepadButton) -> KeyCode {
    match btn {
        GamepadButton::A => KeyCode::GamepadA,
        GamepadButton::B => KeyCode::GamepadB,
        GamepadButton::X => KeyCode::GamepadX,
        GamepadButton::Y => KeyCode::GamepadY,
        GamepadButton::LB => KeyCode::GamepadLB,
        GamepadButton::RB => KeyCode::GamepadRB,
        GamepadButton::LT => KeyCode::GamepadLT,
        GamepadButton::RT => KeyCode::GamepadRT,
        GamepadButton::Start => KeyCode::GamepadStart,
        GamepadButton::Back => KeyCode::GamepadBack,
        GamepadButton::DPadUp => KeyCode::GamepadDPadUp,
        GamepadButton::DPadDown => KeyCode::GamepadDPadDown,
        GamepadButton::DPadLeft => KeyCode::GamepadDPadLeft,
        GamepadButton::DPadRight => KeyCode::GamepadDPadRight,
    }
}

/// Check whether a `RawInputEvent` matches a binding.
fn event_matches_binding(event: &RawInputEvent, binding: &InputBinding) -> bool {
    if event.device != binding.device {
        return false;
    }

    // Gamepad-button binding: compare via KeyCode alias.
    if let Some(btn) = binding.gamepad_button {
        let expected_key = gamepad_button_to_keycode(btn);
        return event.key == Some(expected_key);
    }

    // Gamepad-axis binding.
    if let Some(expected_axis) = binding.gamepad_axis {
        return event.axis.map(|(a, _)| a == expected_axis).unwrap_or(false);
    }

    // Keyboard / mouse / gamepad-button (via keys list).
    if !binding.keys.is_empty() {
        return event
            .key
            .map(|k| binding.keys.contains(&k))
            .unwrap_or(false);
    }

    false
}

/// Extract the raw float value from an event, preferring the axis payload
/// when present.
fn raw_value(event: &RawInputEvent) -> f32 {
    if let Some((_, v)) = event.axis {
        v
    } else {
        event.value
    }
}

/// Apply an `InputModifier` to a resolved `InputValue`.
fn apply_modifier(value: InputValue, modifier: &InputModifier) -> InputValue {
    match modifier {
        InputModifier::None => value,
        InputModifier::Invert => match value {
            InputValue::Bool(b) => InputValue::Bool(!b),
            InputValue::Float(f) => InputValue::Float(-f),
            InputValue::Vec2(v) => InputValue::Vec2(-v),
        },
        InputModifier::Deadzone(threshold) => {
            let threshold = threshold.max(0.0);
            match value {
                InputValue::Bool(b) => InputValue::Bool(b),
                InputValue::Float(f) => {
                    if f.abs() <= threshold {
                        InputValue::Float(0.0)
                    } else {
                        // Re-scale into [0, 1] preserving sign.
                        let sign = f.signum();
                        let scaled = (f.abs() - threshold) / (1.0 - threshold);
                        InputValue::Float(sign * scaled.clamp(0.0, 1.0))
                    }
                }
                InputValue::Vec2(v) => {
                    let len = v.length();
                    if len <= threshold {
                        InputValue::Vec2(glam::Vec2::ZERO)
                    } else {
                        let scaled = ((len - threshold) / (1.0 - threshold)).clamp(0.0, 1.0);
                        InputValue::Vec2(v.normalize() * scaled)
                    }
                }
            }
        }
        InputModifier::Scale(factor) => match value {
            InputValue::Bool(b) => InputValue::Bool(b),
            InputValue::Float(f) => InputValue::Float(f * *factor),
            InputValue::Vec2(v) => InputValue::Vec2(v * *factor),
        },
    }
}

// ---------------------------------------------------------------------------
// Public API — resolution
// ---------------------------------------------------------------------------

/// Resolve a single action from the map against a batch of raw events.
///
/// * For `Digital` and `Analog1D` actions the first matching binding wins.
/// * For `Analog2D` actions, X and Y axis components are accumulated from
///   all matching bindings and composed into a `Vec2`.  A modifier is
///   applied only if a single binding type can be identified.
pub fn resolve_action(
    map: &InputActionMap,
    events: &[RawInputEvent],
    action_name: &str,
) -> Option<InputValue> {
    let action = map.actions.iter().find(|a| a.name == action_name)?;

    match action.value_type {
        InputValueType::Analog2D => {
            // Collect X and Y components from all bindings.
            let mut x_val: Option<f32> = None;
            let mut y_val: Option<f32> = None;

            for binding in &action.bindings {
                for event in events {
                    if event_matches_binding(event, binding) {
                        let rv = raw_value(event);
                        if let Some((axis, _)) = event.axis {
                            match axis {
                                GamepadAxis::LeftX | GamepadAxis::RightX => x_val = Some(rv),
                                GamepadAxis::LeftY | GamepadAxis::RightY => y_val = Some(rv),
                    GamepadAxis::LT => x_val = Some(rv),
                    GamepadAxis::RT => y_val = Some(rv),
                            }
                        } else {
                            // Non-axis match (keyboard/gamepad-button): treated as X-axis.
                            // Keyboard-based Analog2D (e.g. WASD for both axes) requires
                            // separate bindings for X and Y directions.
                            if rv.abs() > 0.5 {
                                x_val = Some(rv);
                            }
                        }
                    }
                }
            }

            if x_val.is_some() || y_val.is_some() {
                let vec = glam::Vec2::new(x_val.unwrap_or(0.0), y_val.unwrap_or(0.0));
                let value = InputValue::Vec2(vec);

                // Apply modifier from the first matching binding (if any).
                let modifier = action
                    .bindings
                    .iter()
                    .find_map(|b| {
                        events
                            .iter()
                            .any(|e| event_matches_binding(e, b))
                            .then_some(b.modifier)
                    })
                    .unwrap_or(InputModifier::None);

                let result = apply_modifier(value, &modifier);
                return Some(result);
            }
            None
        }
        InputValueType::Digital | InputValueType::Analog1D => {
            for binding in &action.bindings {
                for event in events {
                    if event_matches_binding(event, binding) {
                        let rv = raw_value(event);
                        let value = match action.value_type {
                            InputValueType::Digital => InputValue::Bool(rv.abs() > 0.5),
                            _ => InputValue::Float(rv),
                        };
                        return Some(apply_modifier(value, &binding.modifier));
                    }
                }
            }
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Public API — persistence and rebinding
// ---------------------------------------------------------------------------

/// Serialise an `InputActionMap` to a JSON string.
///
/// Uses `serde_json` with pretty-printing for readability.
pub fn serialize_bindings(map: &InputActionMap) -> String {
    serde_json::to_string_pretty(map).expect("InputActionMap serialisation should not fail")
}

/// Deserialise an `InputActionMap` from a JSON string.
///
/// Returns `Err(String)` with a human-readable message on failure.
pub fn deserialize_bindings(json: &str) -> Result<InputActionMap, String> {
    serde_json::from_str::<InputActionMap>(json)
        .map_err(|e| format!("Failed to deserialize input bindings: {e}"))
}

/// Replace a binding in an action.
///
/// # Errors
///
/// Returns `Err` if the action is not found or `binding_index` is out of
/// bounds.
pub fn rebind_action(
    map: &mut InputActionMap,
    action_name: &str,
    binding_index: usize,
    new_binding: InputBinding,
) -> Result<(), String> {
    let action = map
        .action_mut(action_name)
        .ok_or_else(|| format!("Action '{action_name}' not found"))?;

    if binding_index >= action.bindings.len() {
        return Err(format!(
            "Binding index {binding_index} out of range for action '{action_name}' \
             ({} bindings)",
            action.bindings.len()
        ));
    }

    action.bindings[binding_index] = new_binding;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- Action creation ----------------------------------------------------

    #[test]
    fn create_action_map_and_add_actions() {
        let mut map = InputActionMap::new("player", "gameplay");
        let jump = InputAction::new("jump", InputValueType::Digital)
            .add_binding(InputBinding::keyboard("jump", vec![KeyCode::Space]))
            .clone();
        map.add_action(jump);

        let walk = InputAction::new("walk", InputValueType::Analog2D)
            .add_binding(InputBinding::gamepad_axis("walk", GamepadAxis::LeftX))
            .add_binding(InputBinding::gamepad_axis("walk", GamepadAxis::LeftY))
            .clone();
        map.add_action(walk);

        assert_eq!(map.actions.len(), 2);
        assert_eq!(map.action("jump").unwrap().name, "jump");
        assert_eq!(map.action("walk").unwrap().name, "walk");
        assert_eq!(map.action("nonexistent"), None);
    }

    #[test]
    fn action_mut_modifies_in_place() {
        let mut map = InputActionMap::new("player", "gameplay");
        map.add_action(
            InputAction::new("jump", InputValueType::Digital)
                .add_binding(InputBinding::keyboard("jump", vec![KeyCode::Space]))
                .clone(),
        );

        let action = map.action_mut("jump").unwrap();
        action
            .bindings
            .push(InputBinding::gamepad_button("jump", GamepadButton::A));
        assert_eq!(map.action("jump").unwrap().bindings.len(), 2);
    }

    #[test]
    fn chained_add_action_returns_self() {
        let mut map = InputActionMap::new("player", "gameplay");
        map.add_action(InputAction::new("a", InputValueType::Digital))
            .add_action(InputAction::new("b", InputValueType::Digital));
        assert_eq!(map.actions.len(), 2);
    }

    // -- Digital binding resolution -----------------------------------------

    #[test]
    fn digital_keyboard_binding_pressed() {
        let mut map = InputActionMap::new("player", "gameplay");
        map.add_action(
            InputAction::new("jump", InputValueType::Digital)
                .add_binding(InputBinding::keyboard("jump", vec![KeyCode::Space]))
                .clone(),
        );

        let events = [RawInputEvent::keyboard(KeyCode::Space, 1.0)];
        assert_eq!(
            resolve_action(&map, &events, "jump"),
            Some(InputValue::Bool(true))
        );
    }

    #[test]
    fn digital_keyboard_binding_released() {
        let mut map = InputActionMap::new("player", "gameplay");
        map.add_action(
            InputAction::new("jump", InputValueType::Digital)
                .add_binding(InputBinding::keyboard("jump", vec![KeyCode::Space]))
                .clone(),
        );

        let events = [RawInputEvent::keyboard(KeyCode::Space, 0.0)];
        assert_eq!(
            resolve_action(&map, &events, "jump"),
            Some(InputValue::Bool(false))
        );
    }

    #[test]
    fn digital_gamepad_button_binding() {
        let mut map = InputActionMap::new("player", "gameplay");
        map.add_action(
            InputAction::new("jump", InputValueType::Digital)
                .add_binding(InputBinding::gamepad_button("jump", GamepadButton::A))
                .clone(),
        );

        let events = [RawInputEvent::gamepad_button(KeyCode::GamepadA, 1.0)];
        assert_eq!(
            resolve_action(&map, &events, "jump"),
            Some(InputValue::Bool(true))
        );
    }

    #[test]
    fn digital_multiple_keys_any_matches() {
        let mut map = InputActionMap::new("player", "gameplay");
        map.add_action(
            InputAction::new("fire", InputValueType::Digital)
                .add_binding(InputBinding::keyboard(
                    "fire",
                    vec![KeyCode::MouseLeft, KeyCode::Space],
                ))
                .clone(),
        );

        // Mouse click matches.
        let events = [RawInputEvent::mouse(KeyCode::MouseLeft, 1.0)];
        assert_eq!(
            resolve_action(&map, &events, "fire"),
            Some(InputValue::Bool(true))
        );

        // Space also matches.
        let events = [RawInputEvent::keyboard(KeyCode::Space, 1.0)];
        assert_eq!(
            resolve_action(&map, &events, "fire"),
            Some(InputValue::Bool(true))
        );
    }

    #[test]
    fn digital_wrong_key_does_not_match() {
        let mut map = InputActionMap::new("player", "gameplay");
        map.add_action(
            InputAction::new("jump", InputValueType::Digital)
                .add_binding(InputBinding::keyboard("jump", vec![KeyCode::Space]))
                .clone(),
        );

        let events = [RawInputEvent::keyboard(KeyCode::Enter, 1.0)];
        assert_eq!(resolve_action(&map, &events, "jump"), None);
    }

    // -- Analog1D binding resolution ----------------------------------------

    #[test]
    fn analog1d_axis_binding() {
        let mut map = InputActionMap::new("player", "gameplay");
        map.add_action(
            InputAction::new("throttle", InputValueType::Analog1D)
                .add_binding(InputBinding::gamepad_axis("throttle", GamepadAxis::RT))
                .clone(),
        );

        let events = [RawInputEvent::gamepad_axis(GamepadAxis::RT, 0.75)];
        assert_eq!(
            resolve_action(&map, &events, "throttle"),
            Some(InputValue::Float(0.75))
        );
    }

    // -- Analog2D resolution ------------------------------------------------

    #[test]
    fn analog2d_stick_both_axes() {
        let mut map = InputActionMap::new("player", "gameplay");
        map.add_action(
            InputAction::new("move", InputValueType::Analog2D)
                .add_binding(InputBinding::gamepad_axis("move", GamepadAxis::LeftX))
                .add_binding(InputBinding::gamepad_axis("move", GamepadAxis::LeftY))
                .clone(),
        );

        let events = [
            RawInputEvent::gamepad_axis(GamepadAxis::LeftX, 0.5),
            RawInputEvent::gamepad_axis(GamepadAxis::LeftY, -1.0),
        ];
        let result = resolve_action(&map, &events, "move");
        assert_eq!(result, Some(InputValue::Vec2(glam::Vec2::new(0.5, -1.0))));
    }

    #[test]
    fn analog2d_stick_single_axis() {
        let mut map = InputActionMap::new("player", "gameplay");
        map.add_action(
            InputAction::new("move", InputValueType::Analog2D)
                .add_binding(InputBinding::gamepad_axis("move", GamepadAxis::LeftX))
                .add_binding(InputBinding::gamepad_axis("move", GamepadAxis::LeftY))
                .clone(),
        );

        // Only X axis event.
        let events = [RawInputEvent::gamepad_axis(GamepadAxis::LeftX, 0.8)];
        let result = resolve_action(&map, &events, "move");
        assert_eq!(result, Some(InputValue::Vec2(glam::Vec2::new(0.8, 0.0))));
    }

    // -- Modifier application -----------------------------------------------

    #[test]
    fn modifier_none_passthrough() {
        let mut map = InputActionMap::new("player", "gameplay");
        map.add_action(
            InputAction::new("throttle", InputValueType::Analog1D)
                .add_binding(
                    InputBinding::gamepad_axis("throttle", GamepadAxis::RT)
                        .with_modifier(InputModifier::None),
                )
                .clone(),
        );

        let events = [RawInputEvent::gamepad_axis(GamepadAxis::RT, 0.5)];
        assert_eq!(
            resolve_action(&map, &events, "throttle"),
            Some(InputValue::Float(0.5))
        );
    }

    #[test]
    fn modifier_invert_float() {
        let mut map = InputActionMap::new("player", "gameplay");
        map.add_action(
            InputAction::new("look", InputValueType::Analog1D)
                .add_binding(
                    InputBinding::gamepad_axis("look", GamepadAxis::RightY)
                        .with_modifier(InputModifier::Invert),
                )
                .clone(),
        );

        let events = [RawInputEvent::gamepad_axis(GamepadAxis::RightY, 0.3)];
        assert_eq!(
            resolve_action(&map, &events, "look"),
            Some(InputValue::Float(-0.3))
        );
    }

    #[test]
    fn modifier_invert_bool() {
        let inverted = apply_modifier(InputValue::Bool(true), &InputModifier::Invert);
        assert_eq!(inverted, InputValue::Bool(false));

        let not_inverted = apply_modifier(InputValue::Bool(false), &InputModifier::Invert);
        assert_eq!(not_inverted, InputValue::Bool(true));
    }

    #[test]
    fn modifier_scale_float() {
        let mut map = InputActionMap::new("player", "gameplay");
        map.add_action(
            InputAction::new("sensitivity", InputValueType::Analog1D)
                .add_binding(
                    InputBinding::gamepad_axis("sensitivity", GamepadAxis::RightX)
                        .with_modifier(InputModifier::Scale(2.0)),
                )
                .clone(),
        );

        let events = [RawInputEvent::gamepad_axis(GamepadAxis::RightX, 0.4)];
        assert_eq!(
            resolve_action(&map, &events, "sensitivity"),
            Some(InputValue::Float(0.8))
        );
    }

    #[test]
    fn modifier_deadzone_float_below_threshold() {
        let mut map = InputActionMap::new("player", "gameplay");
        map.add_action(
            InputAction::new("move_x", InputValueType::Analog1D)
                .add_binding(
                    InputBinding::gamepad_axis("move_x", GamepadAxis::LeftX)
                        .with_modifier(InputModifier::Deadzone(0.2)),
                )
                .clone(),
        );

        // Value below deadzone → zero.
        let events = [RawInputEvent::gamepad_axis(GamepadAxis::LeftX, 0.1)];
        assert_eq!(
            resolve_action(&map, &events, "move_x"),
            Some(InputValue::Float(0.0))
        );
    }

    #[test]
    fn modifier_deadzone_float_above_threshold() {
        let mut map = InputActionMap::new("player", "gameplay");
        map.add_action(
            InputAction::new("move_x", InputValueType::Analog1D)
                .add_binding(
                    InputBinding::gamepad_axis("move_x", GamepadAxis::LeftX)
                        .with_modifier(InputModifier::Deadzone(0.2)),
                )
                .clone(),
        );

        // Value above deadzone → re-scaled.
        // raw = 0.5, threshold = 0.2 → (0.5 - 0.2) / (1.0 - 0.2) = 0.3 / 0.8 = 0.375
        let events = [RawInputEvent::gamepad_axis(GamepadAxis::LeftX, 0.5)];
        let result = resolve_action(&map, &events, "move_x");
        assert_eq!(result, Some(InputValue::Float(0.375)));
    }

    #[test]
    fn modifier_deadzone_vec2_below_threshold() {
        let value = InputValue::Vec2(glam::Vec2::new(0.05, 0.05));
        let result = apply_modifier(value, &InputModifier::Deadzone(0.1));
        assert_eq!(result, InputValue::Vec2(glam::Vec2::ZERO));
    }

    #[test]
    fn modifier_deadzone_vec2_above_threshold() {
        let value = InputValue::Vec2(glam::Vec2::new(0.5, 0.0));
        let result = apply_modifier(value, &InputModifier::Deadzone(0.2));
        // len = 0.5, threshold = 0.2 → (0.5 - 0.2) / 0.8 = 0.375
        // normalize * 0.375 = (1.0, 0.0) * 0.375 = (0.375, 0.0)
        assert_eq!(result, InputValue::Vec2(glam::Vec2::new(0.375, 0.0)));
    }

    // -- Serialization roundtrip --------------------------------------------

    #[test]
    fn serialize_deserialize_roundtrip() {
        let mut map = InputActionMap::new("player", "gameplay");
        map.add_action(
            InputAction::new("jump", InputValueType::Digital)
                .add_binding(InputBinding::keyboard("jump", vec![KeyCode::Space]))
                .add_binding(InputBinding::gamepad_button("jump", GamepadButton::A))
                .clone(),
        );
        map.add_action(
            InputAction::new("move", InputValueType::Analog2D)
                .add_binding(
                    InputBinding::gamepad_axis("move", GamepadAxis::LeftX)
                        .with_modifier(InputModifier::Deadzone(0.15)),
                )
                .add_binding(InputBinding::gamepad_axis("move", GamepadAxis::LeftY))
                .clone(),
        );

        let json = serialize_bindings(&map);
        let deserialized = deserialize_bindings(&json).unwrap();

        assert_eq!(deserialized.name, map.name);
        assert_eq!(deserialized.context, map.context);
        assert_eq!(deserialized.actions.len(), map.actions.len());

        let jump_orig = map.action("jump").unwrap();
        let jump_new = deserialized.action("jump").unwrap();
        assert_eq!(jump_orig.name, jump_new.name);
        assert_eq!(jump_orig.value_type, jump_new.value_type);
        assert_eq!(jump_orig.bindings.len(), jump_new.bindings.len());
        assert_eq!(jump_orig.bindings[0].keys, jump_new.bindings[0].keys);
        assert_eq!(
            jump_orig.bindings[0].gamepad_button,
            jump_new.bindings[0].gamepad_button
        );

        let move_new = deserialized.action("move").unwrap();
        assert_eq!(move_new.bindings[0].modifier, InputModifier::Deadzone(0.15));
    }

    #[test]
    fn deserialize_invalid_json_returns_error() {
        let result = deserialize_bindings("not valid json");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Failed to deserialize"));
    }

    // -- Rebinding ----------------------------------------------------------

    #[test]
    fn rebind_action_replaces_binding() {
        let mut map = InputActionMap::new("player", "gameplay");
        map.add_action(
            InputAction::new("jump", InputValueType::Digital)
                .add_binding(InputBinding::keyboard("jump", vec![KeyCode::Space]))
                .clone(),
        );

        assert!(map.action("jump").unwrap().bindings[0]
            .keys
            .contains(&KeyCode::Space));

        // Rebind to Enter.
        rebind_action(
            &mut map,
            "jump",
            0,
            InputBinding::keyboard("jump", vec![KeyCode::Enter]),
        )
        .unwrap();

        let action = map.action("jump").unwrap();
        assert!(!action.bindings[0].keys.contains(&KeyCode::Space));
        assert!(action.bindings[0].keys.contains(&KeyCode::Enter));

        // Resolution uses the new binding.
        let events = [RawInputEvent::keyboard(KeyCode::Space, 1.0)];
        assert_eq!(resolve_action(&map, &events, "jump"), None);

        let events = [RawInputEvent::keyboard(KeyCode::Enter, 1.0)];
        assert_eq!(
            resolve_action(&map, &events, "jump"),
            Some(InputValue::Bool(true))
        );
    }

    #[test]
    fn rebind_action_invalid_action_returns_error() {
        let mut map = InputActionMap::new("player", "gameplay");
        map.add_action(
            InputAction::new("jump", InputValueType::Digital)
                .add_binding(InputBinding::keyboard("jump", vec![KeyCode::Space]))
                .clone(),
        );

        let result = rebind_action(
            &mut map,
            "nonexistent",
            0,
            InputBinding::keyboard("nonexistent", vec![KeyCode::Enter]),
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[test]
    fn rebind_action_invalid_index_returns_error() {
        let mut map = InputActionMap::new("player", "gameplay");
        map.add_action(
            InputAction::new("jump", InputValueType::Digital)
                .add_binding(InputBinding::keyboard("jump", vec![KeyCode::Space]))
                .clone(),
        );

        // Index 5 is out of range (only 1 binding).
        let result = rebind_action(
            &mut map,
            "jump",
            5,
            InputBinding::keyboard("jump", vec![KeyCode::Enter]),
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("out of range"));
    }

    // -- Edge cases ---------------------------------------------------------

    #[test]
    fn device_mismatch_no_match() {
        let mut map = InputActionMap::new("player", "gameplay");
        map.add_action(
            InputAction::new("jump", InputValueType::Digital)
                .add_binding(InputBinding::gamepad_button("jump", GamepadButton::A))
                .clone(),
        );

        // Keyboard event does not match gamepad binding.
        let events = [RawInputEvent::keyboard(KeyCode::Space, 1.0)];
        assert_eq!(resolve_action(&map, &events, "jump"), None);
    }

    #[test]
    fn empty_events_no_match() {
        let mut map = InputActionMap::new("player", "gameplay");
        map.add_action(
            InputAction::new("jump", InputValueType::Digital)
                .add_binding(InputBinding::keyboard("jump", vec![KeyCode::Space]))
                .clone(),
        );

        assert_eq!(resolve_action(&map, &[], "jump"), None);
    }

    #[test]
    fn unknown_action_returns_none() {
        let map = InputActionMap::new("player", "gameplay");
        assert_eq!(
            resolve_action(
                &map,
                &[RawInputEvent::keyboard(KeyCode::Space, 1.0)],
                "unknown"
            ),
            None
        );
    }

    // -- InputAction defaults -----------------------------------------------

    #[test]
    fn default_current_value() {
        let digital = InputAction::new("d", InputValueType::Digital);
        assert_eq!(digital.current_value, InputValue::Bool(false));

        let analog1d = InputAction::new("a", InputValueType::Analog1D);
        assert_eq!(analog1d.current_value, InputValue::Float(0.0));

        let analog2d = InputAction::new("a2", InputValueType::Analog2D);
        assert_eq!(analog2d.current_value, InputValue::Vec2(glam::Vec2::ZERO));
    }

    // -- resolve_binding (method alias) -------------------------------------

    #[test]
    fn resolve_binding_method() {
        let mut map = InputActionMap::new("player", "gameplay");
        map.add_action(
            InputAction::new("jump", InputValueType::Digital)
                .add_binding(InputBinding::keyboard("jump", vec![KeyCode::Space]))
                .clone(),
        );

        let events = [RawInputEvent::keyboard(KeyCode::Space, 1.0)];
        assert_eq!(
            map.resolve_binding("jump", &events),
            Some(InputValue::Bool(true))
        );
    }
}

// ---------------------------------------------------------------------------
// InputBinding convenience: with_modifier (used in tests)
// ---------------------------------------------------------------------------

impl InputBinding {
    /// Set the modifier on this binding (builder pattern).
    pub fn with_modifier(mut self, modifier: InputModifier) -> Self {
        self.modifier = modifier;
        self
    }
}
