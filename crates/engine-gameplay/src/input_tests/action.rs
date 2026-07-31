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
