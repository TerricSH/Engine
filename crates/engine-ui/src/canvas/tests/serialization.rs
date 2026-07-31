#[test]
fn canvas_new_and_accessors() {
    let canvas = Canvas::new(800.0, 600.0);
    assert_eq!(canvas.width, 800.0);
    assert_eq!(canvas.height, 600.0);
    assert_eq!(canvas.scale_mode, ScaleMode::Fixed);
}

#[test]
fn canvas_scene_roundtrip_preserves_every_element_kind_and_tree() {
    let mut canvas = Canvas::new(1280.0, 720.0);
    canvas.scale_mode = ScaleMode::FitHeight;

    let ids: Vec<_> = every_element_kind()
        .into_iter()
        .enumerate()
        .map(|(index, kind)| {
            let offset = index as f32 * 10.0;
            canvas.add_element(
                UiElement::new(
                    kind,
                    Layout::new(
                        Vec2::ZERO,
                        Vec2::ZERO,
                        Vec2::new(offset, offset + 1.0),
                        Vec2::new(offset + 100.0, offset + 51.0),
                    ),
                )
                .with_z_order(index as i32 - 3)
                .with_enabled(index % 2 == 0),
            )
        })
        .collect();
    canvas.get_element_mut(ids[0]).unwrap().children = vec![ids[1], ids[2]];
    canvas.get_element_mut(ids[2]).unwrap().children = vec![ids[3]];
    canvas.set_next_id(50);

    let fields = serialize_canvas(&canvas);
    assert_eq!(
        fields.get("scale_mode"),
        Some(&Value::Enum("FitHeight".into()))
    );
    let Some(Value::List(elements)) = fields.get("elements") else {
        panic!("elements must use the list contract");
    };
    let Value::Map(image) = &elements[1] else {
        panic!("element must use the map contract");
    };
    let Some(Value::Map(image_kind)) = image.get("kind") else {
        panic!("element kind must use the map contract");
    };
    assert_eq!(
        image_kind.get("texture_id"),
        Some(&Value::Asset(AssetId::new("ui/portrait")))
    );

    let restored = restored_canvas(&fields);
    assert_eq!(restored.width, canvas.width);
    assert_eq!(restored.height, canvas.height);
    assert_eq!(restored.scale_mode, canvas.scale_mode);
    assert_eq!(restored.elements, canvas.elements);
    assert_eq!(restored.next_id, 50);
}

#[test]
fn canvas_empty_and_legacy_metadata_only_formats_load_safely() {
    let empty = Canvas::new(320.0, 240.0);
    let restored = restored_canvas(&serialize_canvas(&empty));
    assert!(restored.elements.is_empty());
    assert_eq!(restored.next_id, 1);

    let legacy = BTreeMap::from([
        ("width".into(), Value::Float32(1024.0)),
        ("height".into(), Value::Float32(768.0)),
        ("scale_mode".into(), Value::Str("FitWidth".into())),
        ("next_id".into(), Value::UInt(7)),
        ("element_count".into(), Value::UInt(99)),
    ]);
    let mut restored = restored_canvas(&legacy);
    assert_eq!(restored.width, 1024.0);
    assert_eq!(restored.height, 768.0);
    assert_eq!(restored.scale_mode, ScaleMode::FitWidth);
    assert!(restored.elements.is_empty());
    assert_eq!(
        restored.add_element(panel_element(Layout::FILL, 0, Color::WHITE)),
        ElementId(7)
    );
}

#[test]
fn canvas_deserializer_repairs_invalid_ids_links_cycles_and_next_id() {
    let make_element = |id| UiElement {
        id: ElementId(id),
        kind: UiElementKind::Panel {
            color: Color::new(id as u8, 0, 0, 255),
        },
        layout: Layout::FILL,
        z_order: id as i32,
        enabled: true,
        children: Vec::new(),
        rect: UiRect::ZERO,
    };
    let mut first = encode_element(&make_element(1));
    let mut second = encode_element(&make_element(2));
    let mut third = encode_element(&make_element(3));
    let mut duplicate = first.clone();
    let mut invalid = first.clone();

    let set_field = |record: &mut Value, key: &str, value: Value| {
        let Value::Map(fields) = record else {
            panic!("encoded element must be a map");
        };
        fields.insert(key.into(), value);
    };
    set_field(
        &mut first,
        "children",
        Value::List(vec![
            Value::UInt(2),
            Value::UInt(2),
            Value::UInt(99),
            Value::UInt(1),
            Value::UInt(u64::MAX),
        ]),
    );
    set_field(&mut second, "children", Value::List(vec![Value::UInt(3)]));
    set_field(&mut third, "children", Value::List(vec![Value::UInt(1)]));
    set_field(&mut duplicate, "z_order", Value::Int(999));
    set_field(&mut invalid, "id", Value::UInt(u64::from(u32::MAX)));

    let fields = BTreeMap::from([
        ("next_id".into(), Value::UInt(1)),
        (
            "elements".into(),
            Value::List(vec![first, second, third, duplicate, invalid]),
        ),
    ]);
    let mut restored = restored_canvas(&fields);

    assert_eq!(restored.elements.len(), 3);
    assert_eq!(restored.get_element(ElementId(1)).unwrap().z_order, 1);
    assert_eq!(
        restored.get_element(ElementId(1)).unwrap().children,
        vec![ElementId(2)]
    );
    assert_eq!(
        restored.get_element(ElementId(2)).unwrap().children,
        vec![ElementId(3)]
    );
    assert!(restored
        .get_element(ElementId(3))
        .unwrap()
        .children
        .is_empty());
    assert_eq!(
        restored.add_element(panel_element(Layout::FILL, 0, Color::WHITE)),
        ElementId(4)
    );
}

#[test]
fn canvas_roundtrip_recomputes_nested_layout_without_persisting_rects() {
    let mut canvas = Canvas::new(800.0, 600.0);
    let parent = canvas.add_element(panel_element(
        Layout::new(Vec2::ZERO, Vec2::new(0.5, 1.0), Vec2::ZERO, Vec2::ZERO),
        0,
        Color::WHITE,
    ));
    let child = canvas.add_element(panel_element(Layout::FILL, 1, Color::BLACK));
    canvas.get_element_mut(parent).unwrap().children.push(child);
    canvas.layout_all();
    let expected_parent = canvas.get_element(parent).unwrap().rect;
    let expected_child = canvas.get_element(child).unwrap().rect;

    let mut restored = restored_canvas(&serialize_canvas(&canvas));
    assert_eq!(restored.get_element(parent).unwrap().rect, UiRect::ZERO);
    assert_eq!(restored.get_element(child).unwrap().rect, UiRect::ZERO);
    restored.layout_all();

    assert_eq!(restored.get_element(parent).unwrap().rect, expected_parent);
    assert_eq!(restored.get_element(child).unwrap().rect, expected_child);
}

#[test]
fn canvas_image_texture_is_collected_as_a_scene_dependency() {
    let mut registry = ComponentRegistry::new();
    register_ui_extensions(&mut registry);
    let mut world = World::from_scene(&sample_scene());
    world.set_component_registry(registry);
    let entity = world
        .entity_by_persistent_id("cube-01")
        .expect("sample entity must exist");
    let mut canvas = Canvas::new(800.0, 600.0);
    canvas.add_element(image_element(Layout::FILL, 0, "ui/hud-atlas", Color::WHITE));
    world.add_component(entity, canvas);

    let dependencies = world.to_scene().collect_asset_dependencies();

    assert!(dependencies.contains(&AssetId::new("ui/hud-atlas")));
}
