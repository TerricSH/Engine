use std::collections::BTreeMap;

use super::{deserialize_canvas, encode_element, serialize_canvas};
use crate::*;
use engine_scene::{sample_scene, ComponentRegistry, World};
use engine_serialize::{AssetId, Value};
use glam::Vec2;

fn test_canvas() -> Canvas {
    Canvas::new(800.0, 600.0)
}

fn panel_element(layout: Layout, z: i32, color: Color) -> UiElement {
    UiElement::new(UiElementKind::Panel { color }, layout).with_z_order(z)
}

fn image_element(layout: Layout, z: i32, texture_id: &str, color: Color) -> UiElement {
    UiElement::new(
        UiElementKind::Image {
            texture_id: texture_id.to_string(),
            color,
        },
        layout,
    )
    .with_z_order(z)
}

fn every_element_kind() -> Vec<UiElementKind> {
    vec![
        UiElementKind::Panel {
            color: Color::new(1, 2, 3, 4),
        },
        UiElementKind::Image {
            texture_id: "ui/portrait".into(),
            color: Color::new(5, 6, 7, 8),
        },
        UiElementKind::Text {
            content: "Status".into(),
            font_size: 18.5,
            color: Color::new(9, 10, 11, 12),
        },
        UiElementKind::Button {
            label: "Start".into(),
            normal_color: Color::new(13, 14, 15, 16),
            hover_color: Color::new(17, 18, 19, 20),
            pressed_color: Color::new(21, 22, 23, 24),
            callback_id: Some("start_game".into()),
        },
        UiElementKind::Toggle {
            label: "Music".into(),
            is_on: true,
            color_on: Color::new(25, 26, 27, 28),
            color_off: Color::new(29, 30, 31, 32),
            callback_id: None,
        },
        UiElementKind::Checkbox {
            label: "Hints".into(),
            checked: false,
            color: Color::new(33, 34, 35, 36),
            callback_id: Some("show_hints".into()),
        },
        UiElementKind::Slider {
            label: "Volume".into(),
            value: 0.75,
            min: 0.0,
            max: 1.0,
            callback_id: None,
        },
        UiElementKind::ScrollView {
            scroll_x: 4.0,
            scroll_y: 8.0,
            content_width: 1024.0,
            content_height: 2048.0,
            color: Color::new(37, 38, 39, 40),
        },
    ]
}

fn restored_canvas(fields: &BTreeMap<String, Value>) -> Canvas {
    let component = deserialize_canvas(fields);
    component
        .downcast_ref::<Canvas>()
        .expect("Canvas deserializer must return Canvas")
        .clone()
}

include!("tests/serialization.rs");
include!("tests/mutations.rs");
include!("tests/batching.rs");
include!("tests/layout.rs");
