#[test]
fn build_batches_empty_canvas() {
    let canvas = test_canvas();
    assert!(canvas.build_batches().is_empty());
}

#[test]
fn build_batches_skips_disabled() {
    let mut canvas = test_canvas();
    canvas.add_element(panel_element(Layout::FILL, 0, Color::WHITE).with_enabled(false));
    assert!(canvas.build_batches().is_empty());
}

#[test]
fn build_batches_single_panel() {
    let mut canvas = test_canvas();
    let layout = Layout::new(
        Vec2::ZERO,
        Vec2::ZERO,
        Vec2::new(0.0, 0.0),
        Vec2::new(100.0, 50.0),
    );
    canvas.add_element(panel_element(layout, 0, Color::WHITE));
    canvas.layout_all();
    let batches = canvas.build_batches();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].z_order, 0);
    assert_eq!(batches[0].vertices.len(), 4);
    assert_eq!(batches[0].indices.len(), 6);
    assert!(batches[0].texture.is_none());
}

#[test]
fn build_batches_z_order_splits() {
    let mut canvas = test_canvas();
    let l1 = Layout::new(Vec2::ZERO, Vec2::ZERO, Vec2::ZERO, Vec2::new(10.0, 10.0));
    let l2 = Layout::new(
        Vec2::ZERO,
        Vec2::ZERO,
        Vec2::new(0.0, 0.0),
        Vec2::new(10.0, 10.0),
    );
    canvas.add_element(panel_element(l1, 0, Color::WHITE));
    canvas.add_element(panel_element(l2, 1, Color::WHITE));
    canvas.layout_all();
    let batches = canvas.build_batches();
    assert_eq!(batches.len(), 2);
    assert_eq!(batches[0].z_order, 0);
    assert_eq!(batches[1].z_order, 1);
}

#[test]
fn build_batches_merges_same_z_and_texture() {
    let mut canvas = test_canvas();
    let l1 = Layout::new(Vec2::ZERO, Vec2::ZERO, Vec2::ZERO, Vec2::new(10.0, 10.0));
    let l2 = Layout::new(
        Vec2::ZERO,
        Vec2::ZERO,
        Vec2::new(10.0, 0.0),
        Vec2::new(20.0, 10.0),
    );
    canvas.add_element(panel_element(l1, 0, Color::WHITE));
    canvas.add_element(panel_element(l2, 0, Color::WHITE));
    canvas.layout_all();
    let batches = canvas.build_batches();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].vertices.len(), 8);
    assert_eq!(batches[0].indices.len(), 12);
}

#[test]
fn build_batches_vertex_positions() {
    let mut canvas = test_canvas();
    let layout = Layout::new(
        Vec2::ZERO,
        Vec2::ZERO,
        Vec2::new(10.0, 20.0),
        Vec2::new(40.0, 60.0),
    );
    canvas.add_element(panel_element(layout, 0, Color::WHITE));
    canvas.layout_all();
    let batches = canvas.build_batches();
    let v = &batches[0].vertices;
    assert_eq!(v[0].position, [10.0, 20.0]); // top-left
    assert_eq!(v[1].position, [40.0, 20.0]); // top-right
    assert_eq!(v[2].position, [40.0, 60.0]); // bottom-right
    assert_eq!(v[3].position, [10.0, 60.0]); // bottom-left
}

#[test]
fn build_batches_quad_uvs() {
    let mut canvas = test_canvas();
    let layout = Layout::new(Vec2::ZERO, Vec2::ZERO, Vec2::ZERO, Vec2::new(10.0, 10.0));
    canvas.add_element(panel_element(layout, 0, Color::WHITE));
    canvas.layout_all();
    let batches = canvas.build_batches();
    let v = &batches[0].vertices;
    assert_eq!(v[0].uv, [0.0, 0.0]);
    assert_eq!(v[1].uv, [1.0, 0.0]);
    assert_eq!(v[2].uv, [1.0, 1.0]);
    assert_eq!(v[3].uv, [0.0, 1.0]);
}

#[test]
fn build_batches_panel_color() {
    let color = Color::new(64, 128, 192, 255);
    let mut canvas = test_canvas();
    let layout = Layout::new(Vec2::ZERO, Vec2::ZERO, Vec2::ZERO, Vec2::new(10.0, 10.0));
    canvas.add_element(panel_element(layout, 0, color));
    canvas.layout_all();
    let batches = canvas.build_batches();
    for v in &batches[0].vertices {
        assert_eq!(v.color, [64, 128, 192, 255]);
    }
}

#[test]
fn build_batches_text_uses_font_atlas_uvs_and_full_vertex_color() {
    let mut canvas = test_canvas();
    let layout = Layout::new(Vec2::ZERO, Vec2::ZERO, Vec2::ZERO, Vec2::new(50.0, 20.0));
    canvas.add_element(
        UiElement::new(
            UiElementKind::Text {
                content: "Hello".into(),
                font_size: 16.0,
                color: Color::new(255, 0, 0, 255),
            },
            layout,
        )
        .with_z_order(0),
    );
    canvas.layout_all();
    let batches = canvas.build_batches();
    if crate::font_atlas_texture_upload().is_some() {
        assert_eq!(batches[0].vertices.len(), 20);
        assert_eq!(
            batches[0].texture,
            Some(AssetId::new(crate::FONT_ATLAS_ASSET))
        );
        for vertex in &batches[0].vertices {
            assert_eq!(vertex.color, [255, 0, 0, 255]);
            assert!(vertex.uv[0] >= 0.0 && vertex.uv[0] <= 1.0);
            assert!(vertex.uv[1] >= 0.0 && vertex.uv[1] <= 1.0);
        }
    } else {
        assert!(batches.is_empty());
    }
}

#[test]
fn build_batches_image_has_texture() {
    let mut canvas = test_canvas();
    let layout = Layout::new(Vec2::ZERO, Vec2::ZERO, Vec2::ZERO, Vec2::new(64.0, 64.0));
    canvas.add_element(image_element(layout, 0, "ui/button", Color::WHITE));
    canvas.layout_all();
    let batches = canvas.build_batches();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].texture, Some(AssetId::new("ui/button")));
}

#[test]
fn build_batches_applies_button_hover_color_and_emits_label_text() {
    let mut canvas = test_canvas();
    let id = canvas.add_element(UiElement::new(
        UiElementKind::Button {
            label: "Play".into(),
            normal_color: Color::new(10, 20, 30, 255),
            hover_color: Color::new(40, 50, 60, 255),
            pressed_color: Color::new(70, 80, 90, 255),
            callback_id: Some("play".into()),
        },
        Layout::new(Vec2::ZERO, Vec2::ZERO, Vec2::ZERO, Vec2::new(120.0, 40.0)),
    ));
    canvas.layout_all();
    let mut input = UiInputState::new();
    input.hovered = Some(id);

    let batches = canvas.build_batches_for_viewport(800.0, 600.0, Some(&input));
    assert_eq!(batches[0].vertices[0].color, [40, 50, 60, 255]);
    if crate::font_atlas_texture_upload().is_some() {
        assert!(batches.iter().any(|batch| {
            batch.texture == Some(AssetId::new(crate::FONT_ATLAS_ASSET))
                && !batch.vertices.is_empty()
        }));
    }
}

#[test]
fn fit_width_scales_vertices_and_clip_rect_to_viewport() {
    let mut canvas = Canvas::new(320.0, 180.0);
    canvas.scale_mode = ScaleMode::FitWidth;
    canvas.add_element(panel_element(
        Layout::new(
            Vec2::ZERO,
            Vec2::ZERO,
            Vec2::new(10.0, 20.0),
            Vec2::new(30.0, 40.0),
        ),
        0,
        Color::WHITE,
    ));
    canvas.layout_all();

    let batches = canvas.build_batches_for_viewport(640.0, 480.0, None);
    assert_eq!(batches[0].vertices[0].position, [20.0, 40.0]);
    assert_eq!(batches[0].vertices[2].position, [60.0, 80.0]);
    assert_eq!(batches[0].clip_rect.max, [640.0, 360.0]);
}

#[test]
fn batch_clip_rect_matches_canvas() {
    let mut canvas = Canvas::new(1920.0, 1080.0);
    let layout = Layout::new(Vec2::ZERO, Vec2::ZERO, Vec2::ZERO, Vec2::new(10.0, 10.0));
    canvas.add_element(panel_element(layout, 0, Color::WHITE));
    canvas.layout_all();
    let batches = canvas.build_batches();
    assert_eq!(batches[0].clip_rect.min, [0.0, 0.0]);
    assert_eq!(batches[0].clip_rect.max, [1920.0, 1080.0]);
}

#[test]
fn batch_material_default() {
    let mut canvas = test_canvas();
    let layout = Layout::new(Vec2::ZERO, Vec2::ZERO, Vec2::ZERO, Vec2::new(10.0, 10.0));
    canvas.add_element(panel_element(layout, 0, Color::WHITE));
    canvas.layout_all();
    let batches = canvas.build_batches();
    assert_eq!(batches[0].material, AssetId::new(DEFAULT_UI_MATERIAL));
}
