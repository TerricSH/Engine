#[cfg(all(test, feature = "subsystem-ui"))]
mod runtime_ui_tests {
    use super::*;

    #[test]
    fn retained_ui_geometry_is_embedded_and_clipped_to_the_scene_viewport() {
        let viewport = RenderViewportContext::new(
            1000,
            800,
            engine_renderer::Rect {
                min: [0.2, 0.125],
                max: [0.7, 0.75],
            },
        )
        .unwrap();
        let mut batches = vec![engine_renderer::UiBatch {
            canvas_id: "hud".into(),
            z_order: 0,
            clip_rect: engine_renderer::Rect {
                min: [-10.0, -20.0],
                max: [600.0, 700.0],
            },
            texture: None,
            vertices: vec![engine_renderer::UiVertex {
                position: [25.0, 40.0],
                uv: [0.0, 0.0],
                color: [255; 4],
            }],
            indices: Vec::new(),
            material: engine_renderer::AssetId::new("ui/default"),
        }];

        embed_scene_ui_batches(&mut batches, viewport);

        assert_eq!(batches[0].vertices[0].position, [225.0, 140.0]);
        assert_eq!(batches[0].clip_rect.min, [200.0, 100.0]);
        assert_eq!(batches[0].clip_rect.max, [700.0, 600.0]);
    }

    fn game_loop_with_canvas(mut canvas: engine_ui::Canvas) -> GameLoop {
        canvas.layout_all();
        let mut game_loop = GameLoop::new(EngineConfig::default());
        game_loop.load_scene(engine_scene::sample_scene()).unwrap();
        game_loop
            .runtime
            .with_world_mut(|world| {
                let entity = world.entity_by_persistent_id("camera-main").unwrap();
                world.add_component(entity, canvas);
            })
            .unwrap();
        game_loop
    }

    #[test]
    fn scaled_toggle_click_persists_value_and_reports_it_to_the_host() {
        let mut canvas = engine_ui::Canvas::new(100.0, 20.0);
        canvas.scale_mode = engine_ui::ScaleMode::FitWidth;
        let toggle_id = canvas.add_element(engine_ui::UiElement::new(
            engine_ui::UiElementKind::Toggle {
                label: "Music".into(),
                is_on: false,
                color_on: engine_ui::Color::new(0, 200, 80, 255),
                color_off: engine_ui::Color::new(80, 80, 80, 255),
                callback_id: Some("music".into()),
            },
            engine_ui::Layout::FILL,
        ));
        let mut game_loop = game_loop_with_canvas(canvas);
        game_loop.set_ui_viewport_size(200, 100);

        // Screen coordinates are converted back to the 100x20 logical Canvas.
        game_loop.ui_pointer_move(100.0, 20.0);
        game_loop.ui_pointer_left_press();
        game_loop.ui_pointer_left_release();

        assert_eq!(
            game_loop.take_ui_events(),
            vec![RuntimeUiEvent {
                canvas_id: "camera-main".into(),
                element_id: toggle_id.0,
                callback_id: Some("music".into()),
                value: Some(RuntimeUiValue::Bool(true)),
            }]
        );
        assert_eq!(
            game_loop.runtime.with_world(|world| {
                let entity = world.entity_by_persistent_id("camera-main").unwrap();
                let canvas = world.get::<engine_ui::Canvas>(entity).unwrap();
                match &canvas.get_element(toggle_id).unwrap().kind {
                    engine_ui::UiElementKind::Toggle { is_on, .. } => *is_on,
                    _ => false,
                }
            }),
            Some(true)
        );
    }

    #[test]
    fn slider_drag_reports_continuous_float_values() {
        let mut canvas = engine_ui::Canvas::new(100.0, 20.0);
        let slider_id = canvas.add_element(engine_ui::UiElement::new(
            engine_ui::UiElementKind::Slider {
                label: "Volume".into(),
                value: 0.0,
                min: 0.0,
                max: 1.0,
                callback_id: Some("volume".into()),
            },
            engine_ui::Layout::FILL,
        ));
        let mut game_loop = game_loop_with_canvas(canvas);

        game_loop.ui_pointer_move(10.0, 10.0);
        game_loop.ui_pointer_left_press();
        game_loop.ui_pointer_move(75.0, 10.0);

        assert_eq!(
            game_loop.take_ui_events(),
            vec![RuntimeUiEvent {
                canvas_id: "camera-main".into(),
                element_id: slider_id.0,
                callback_id: Some("volume".into()),
                value: Some(RuntimeUiValue::Float(0.75)),
            }]
        );
        assert_eq!(
            game_loop.runtime.with_world(|world| {
                let entity = world.entity_by_persistent_id("camera-main").unwrap();
                let canvas = world.get::<engine_ui::Canvas>(entity).unwrap();
                match &canvas.get_element(slider_id).unwrap().kind {
                    engine_ui::UiElementKind::Slider { value, .. } => *value,
                    _ => -1.0,
                }
            }),
            Some(0.75)
        );
    }

    #[test]
    fn runtime_batches_scale_to_viewport_and_reference_the_font_atlas() {
        if engine_ui::font_atlas_texture_upload().is_none() {
            return;
        }
        let mut canvas = engine_ui::Canvas::new(320.0, 180.0);
        canvas.scale_mode = engine_ui::ScaleMode::FitWidth;
        canvas.add_element(engine_ui::UiElement::new(
            engine_ui::UiElementKind::Text {
                content: "HUD".into(),
                font_size: 20.0,
                color: engine_ui::Color::WHITE,
            },
            engine_ui::Layout::new(
                glam::Vec2::ZERO,
                glam::Vec2::ZERO,
                glam::Vec2::new(10.0, 10.0),
                glam::Vec2::new(100.0, 40.0),
            ),
        ));
        let mut game_loop = game_loop_with_canvas(canvas);
        game_loop.set_ui_viewport_size(640, 480);

        let batches = game_loop.runtime_ui_batches();
        assert_eq!(batches[0].clip_rect.max, [640.0, 360.0]);
        assert_eq!(
            batches[0].texture,
            Some(engine_serialize::AssetId::new(engine_ui::FONT_ATLAS_ASSET))
        );
    }
}
