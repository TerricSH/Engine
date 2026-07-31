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
