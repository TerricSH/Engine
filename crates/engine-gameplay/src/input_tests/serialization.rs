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
