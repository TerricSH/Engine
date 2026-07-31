#[test]
fn layout_all_computes_panel_rect() {
    let mut canvas = test_canvas();
    let layout = Layout::new(
        Vec2::new(0.25, 0.25),
        Vec2::new(0.75, 0.75),
        Vec2::ZERO,
        Vec2::ZERO,
    );
    let id = canvas.add_element(panel_element(layout, 0, Color::WHITE));
    canvas.layout_all();
    let el = canvas.get_element(id).unwrap();
    // 25% of 800 = 200, 75% of 800 = 600 → width = 400
    // 25% of 600 = 150, 75% of 600 = 450 → height = 300
    assert_eq!(el.rect, UiRect::new(200.0, 150.0, 400.0, 300.0));
}

#[test]
fn layout_all_child_relative_to_parent() {
    let mut canvas = Canvas::new(800.0, 600.0);

    // Parent: left half of canvas
    let parent_layout = Layout::new(Vec2::ZERO, Vec2::new(0.5, 1.0), Vec2::ZERO, Vec2::ZERO);
    let parent_id = canvas.add_element(panel_element(parent_layout, 0, Color::WHITE));

    // Child: fills its parent (the left half)
    let child_layout = Layout::FILL;
    let child_id = canvas.add_element(
        UiElement::new(
            UiElementKind::Panel {
                color: Color::WHITE,
            },
            child_layout,
        )
        .with_z_order(0)
        .with_children(vec![]),
    );

    // Register parent-child relationship
    canvas
        .get_element_mut(parent_id)
        .unwrap()
        .children
        .push(child_id);

    canvas.layout_all();

    let parent = canvas.get_element(parent_id).unwrap();
    assert_eq!(parent.rect, UiRect::new(0.0, 0.0, 400.0, 600.0));

    let child = canvas.get_element(child_id).unwrap();
    // Child should compute relative to parent: fills parent = (0,0,400,600)
    assert_eq!(child.rect, UiRect::new(0.0, 0.0, 400.0, 600.0));
}

#[test]
fn scale_mode_default() {
    let canvas = test_canvas();
    assert_eq!(canvas.scale_mode, ScaleMode::Fixed);
}
