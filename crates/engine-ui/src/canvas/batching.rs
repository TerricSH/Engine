use super::*;

pub(super) fn build_canvas_batches(
    canvas: &Canvas,
    viewport_width: f32,
    viewport_height: f32,
    input: Option<&UiInputState>,
) -> Vec<UiBatch> {
    let mut visible = canvas
        .elements
        .iter()
        .filter(|element| element.enabled)
        .collect::<Vec<_>>();
    visible.sort_by_key(|element| element.z_order);

    let scale = crate::canvas_scale(canvas, viewport_width, viewport_height);
    let clip = Rect {
        min: [0.0, 0.0],
        max: [canvas.width * scale, canvas.height * scale],
    };
    let mut batches = Vec::new();

    for element in visible {
        match &element.kind {
            UiElementKind::Panel { color } => {
                push_quad(
                    &mut batches,
                    element.z_order,
                    clip,
                    None,
                    &element.rect,
                    *color,
                );
            }
            UiElementKind::Image { color, .. } => push_quad(
                &mut batches,
                element.z_order,
                clip,
                batch::element_kind_texture(&element.kind),
                &element.rect,
                *color,
            ),
            UiElementKind::Text {
                content,
                font_size,
                color,
            } => push_text(
                &mut batches,
                element.z_order,
                clip,
                content,
                *font_size,
                *color,
                &element.rect,
            ),
            UiElementKind::Button {
                label,
                normal_color,
                hover_color,
                pressed_color,
                ..
            } => {
                let color = if input.is_some_and(|state| {
                    state.pressed == Some(element.id) || state.capture == Some(element.id)
                }) {
                    *pressed_color
                } else if input.is_some_and(|state| state.hovered == Some(element.id)) {
                    *hover_color
                } else {
                    *normal_color
                };
                push_quad(
                    &mut batches,
                    element.z_order,
                    clip,
                    None,
                    &element.rect,
                    color,
                );
                push_control_label(&mut batches, element, clip, label);
            }
            UiElementKind::Toggle {
                label,
                is_on,
                color_on,
                color_off,
                ..
            } => {
                let color = interaction_tint(
                    if *is_on { *color_on } else { *color_off },
                    input,
                    element.id,
                );
                push_quad(
                    &mut batches,
                    element.z_order,
                    clip,
                    None,
                    &element.rect,
                    color,
                );
                push_control_label(&mut batches, element, clip, label);
            }
            UiElementKind::Checkbox {
                label,
                checked,
                color,
                ..
            } => {
                let base = if *checked {
                    *color
                } else {
                    Color::new(80, 80, 80, 255)
                };
                push_quad(
                    &mut batches,
                    element.z_order,
                    clip,
                    None,
                    &element.rect,
                    interaction_tint(base, input, element.id),
                );
                push_control_label(&mut batches, element, clip, label);
            }
            UiElementKind::Slider {
                label,
                value,
                min,
                max,
                ..
            } => {
                push_quad(
                    &mut batches,
                    element.z_order,
                    clip,
                    None,
                    &element.rect,
                    interaction_tint(Color::new(60, 60, 60, 255), input, element.id),
                );
                let t = if (*max - *min).abs() > 1e-6 {
                    ((*value - *min) / (*max - *min)).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let value_rect = UiRect::new(
                    element.rect.x,
                    element.rect.y,
                    (element.rect.width * t).max(4.0),
                    element.rect.height,
                );
                push_quad(
                    &mut batches,
                    element.z_order,
                    clip,
                    None,
                    &value_rect,
                    Color::new(200, 200, 200, 255),
                );
                push_control_label(&mut batches, element, clip, label);
            }
            UiElementKind::ScrollView { color, .. } => push_quad(
                &mut batches,
                element.z_order,
                clip,
                None,
                &element.rect,
                *color,
            ),
        }
    }

    if (scale - 1.0).abs() > f32::EPSILON {
        for vertex in batches
            .iter_mut()
            .flat_map(|batch: &mut UiBatch| &mut batch.vertices)
        {
            vertex.position[0] *= scale;
            vertex.position[1] *= scale;
        }
    }
    batches
}

fn ensure_batch(
    batches: &mut Vec<UiBatch>,
    z_order: i32,
    clip_rect: Rect,
    texture: Option<AssetId>,
) -> &mut UiBatch {
    let starts_new = batches
        .last()
        .is_none_or(|batch| batch.z_order != z_order || batch.texture != texture);
    if starts_new {
        batches.push(UiBatch {
            canvas_id: String::new(),
            z_order,
            clip_rect,
            texture,
            vertices: Vec::new(),
            indices: Vec::new(),
            material: AssetId::new(DEFAULT_UI_MATERIAL),
        });
    }
    batches.last_mut().expect("UI batch was just created")
}

fn push_quad(
    batches: &mut Vec<UiBatch>,
    z_order: i32,
    clip_rect: Rect,
    texture: Option<AssetId>,
    rect: &UiRect,
    color: Color,
) {
    batch::add_quad(
        ensure_batch(batches, z_order, clip_rect, texture),
        rect,
        &[0.0, 0.0],
        &[1.0, 1.0],
        &batch::color_to_array(color),
    );
}

fn push_text(
    batches: &mut Vec<UiBatch>,
    z_order: i32,
    clip_rect: Rect,
    content: &str,
    font_size: f32,
    color: Color,
    rect: &UiRect,
) {
    if let Some(vertices) = crate::font::render_text(content, font_size, color, rect) {
        if vertices.is_empty() {
            return;
        }
        let batch = ensure_batch(
            batches,
            z_order,
            clip_rect,
            Some(AssetId::new(crate::font::FONT_ATLAS_ASSET)),
        );
        for glyph in vertices.chunks_exact(4) {
            let base = batch.vertices.len() as u32;
            batch.vertices.extend_from_slice(glyph);
            batch
                .indices
                .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    }
}

fn push_control_label(
    batches: &mut Vec<UiBatch>,
    element: &UiElement,
    clip_rect: Rect,
    label: &str,
) {
    if label.is_empty() {
        return;
    }
    let font_size = (element.rect.height * 0.6).clamp(11.0, 28.0);
    let label_rect = UiRect::new(
        element.rect.x + 8.0,
        element.rect.y + ((element.rect.height - font_size) * 0.5).max(0.0),
        (element.rect.width - 16.0).max(0.0),
        font_size,
    );
    push_text(
        batches,
        element.z_order,
        clip_rect,
        label,
        font_size,
        Color::WHITE,
        &label_rect,
    );
}

fn interaction_tint(color: Color, input: Option<&UiInputState>, element_id: ElementId) -> Color {
    let multiplier = if input
        .is_some_and(|state| state.pressed == Some(element_id) || state.capture == Some(element_id))
    {
        0.8
    } else if input.is_some_and(|state| state.hovered == Some(element_id)) {
        1.15
    } else {
        1.0
    };
    let channel = |value: u8| ((value as f32 * multiplier).round().clamp(0.0, 255.0)) as u8;
    Color::new(
        channel(color.r),
        channel(color.g),
        channel(color.b),
        color.a,
    )
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------
// Serialization hooks (field-map format for ComponentRegistry)
// ---------------------------------------------------------------------------

pub(super) const FIRST_ELEMENT_ID: u32 = 1;
pub(super) const LAST_ELEMENT_ID: u32 = u32::MAX - 1;

pub(super) fn normalize_element_id(id: u32) -> u32 {
    if (FIRST_ELEMENT_ID..=LAST_ELEMENT_ID).contains(&id) {
        id
    } else {
        FIRST_ELEMENT_ID
    }
}

pub(super) fn increment_element_id(id: u32) -> u32 {
    if id >= LAST_ELEMENT_ID {
        FIRST_ELEMENT_ID
    } else {
        id + 1
    }
}

pub(super) fn encode_vec2(value: glam::Vec2) -> Value {
    Value::Map(BTreeMap::from([
        ("x".into(), Value::Float32(value.x)),
        ("y".into(), Value::Float32(value.y)),
    ]))
}

pub(super) fn decode_vec2(value: &Value) -> Option<glam::Vec2> {
    let Value::Map(fields) = value else {
        return None;
    };
    let (Some(Value::Float32(x)), Some(Value::Float32(y))) = (fields.get("x"), fields.get("y"))
    else {
        return None;
    };
    if !x.is_finite() || !y.is_finite() {
        return None;
    }
    Some(glam::Vec2::new(*x, *y))
}

pub(super) fn encode_color(color: Color) -> Value {
    let channel = |value: u8| f32::from(value) / 255.0;
    Value::Color([
        channel(color.r),
        channel(color.g),
        channel(color.b),
        channel(color.a),
    ])
}

pub(super) fn decode_color(value: &Value) -> Option<Color> {
    let Value::Color(channels) = value else {
        return None;
    };
    if channels
        .iter()
        .any(|channel| !channel.is_finite() || !(0.0..=1.0).contains(channel))
    {
        return None;
    }
    let channel = |value: f32| (value * 255.0).round() as u8;
    Some(Color::new(
        channel(channels[0]),
        channel(channels[1]),
        channel(channels[2]),
        channel(channels[3]),
    ))
}
