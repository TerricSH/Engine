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
