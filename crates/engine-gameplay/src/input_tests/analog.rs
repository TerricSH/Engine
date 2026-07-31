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
