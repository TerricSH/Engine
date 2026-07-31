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
