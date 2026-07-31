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
