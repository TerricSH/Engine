#[test]
fn canvas_resize() {
    let mut canvas = test_canvas();
    canvas.resize(1024.0, 768.0);
    assert_eq!(canvas.width, 1024.0);
    assert_eq!(canvas.height, 768.0);
}

#[test]
fn add_and_remove_element() {
    let mut canvas = test_canvas();
    let id = canvas.add_element(panel_element(Layout::FILL, 0, Color::WHITE));
    assert!(canvas.get_element(id).is_some());
    assert!(canvas.remove_element(id));
    assert!(canvas.get_element(id).is_none());
}

#[test]
fn add_element_overwrites_id() {
    let mut canvas = test_canvas();
    let mut el = panel_element(Layout::FILL, 0, Color::WHITE);
    el.id = ElementId(999); // should be overwritten
    let id = canvas.add_element(el);
    let stored = canvas.get_element(id).unwrap();
    assert_eq!(stored.id, id);
    assert_ne!(stored.id, ElementId(999));
}

#[test]
fn insert_element_preserves_explicit_script_id_and_rejects_duplicates() {
    let mut canvas = Canvas::new(800.0, 600.0);
    let id = ElementId(42);
    assert_eq!(
        canvas
            .insert_element(id, panel_element(Layout::FILL, 0, Color::WHITE))
            .unwrap(),
        id
    );
    assert_eq!(canvas.get_element(id).unwrap().id, id);
    assert!(matches!(
        canvas.insert_element(id, panel_element(Layout::FILL, 0, Color::WHITE)),
        Err(crate::UiError::DuplicateElementId(duplicate)) if duplicate == id
    ));
    assert!(matches!(
        canvas.insert_element(
            ElementId::INVALID,
            panel_element(Layout::FILL, 0, Color::WHITE)
        ),
        Err(crate::UiError::InvalidElementId(ElementId::INVALID))
    ));
}

#[test]
fn get_element_mut_allows_mutation() {
    let mut canvas = test_canvas();
    let id = canvas.add_element(panel_element(Layout::FILL, 0, Color::WHITE));
    {
        let el = canvas.get_element_mut(id).unwrap();
        el.enabled = false;
    }
    assert!(!canvas.get_element(id).unwrap().enabled);
}

#[test]
fn clear_removes_all_elements() {
    let mut canvas = test_canvas();
    canvas.add_element(panel_element(Layout::FILL, 0, Color::WHITE));
    canvas.add_element(panel_element(Layout::FILL, 1, Color::WHITE));
    canvas.clear();
    assert!(canvas.build_batches().is_empty());
}
