use std::collections::{BTreeMap, BTreeSet};

use engine_renderer::{Rect, UiBatch};
use engine_serialize::{AssetId, Value};
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::batch;
use crate::color::Color;
use crate::input::UiInputState;
use crate::layout::{Layout, ScaleMode};
use crate::types::{ElementId, UiElement, UiElementKind, UiRect};
use crate::DEFAULT_UI_MATERIAL;

// ---------------------------------------------------------------------------
// Canvas
// ---------------------------------------------------------------------------

/// A 2D UI canvas that owns a list of anchor-laid-out elements and produces
/// [`engine_renderer::UiBatch`]es for the render pipeline.
///
/// Elements are ordered by [`UiElement::z_order`] at batch-creation time.
/// Elements sharing the same `z_order` *and* texture are merged into a single
/// batch to reduce draw calls.
///
/// Call [`Canvas::layout_all`] after mutating element layouts to recompute
/// the pixel rectangles used by rendering and hit-testing.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Canvas {
    /// Canvas logical width in pixels.
    pub width: f32,
    /// Canvas logical height in pixels.
    pub height: f32,
    /// Ordered list of UI elements.
    pub elements: Vec<UiElement>,
    /// How the canvas scales when the viewport size changes.
    pub scale_mode: ScaleMode,
    /// Monotonically-increasing ID counter.
    next_id: u32,
}

impl Canvas {
    /// Create a new canvas with the given logical dimensions.
    ///
    /// `scale_mode` defaults to [`ScaleMode::Fixed`].
    pub fn new(width: f32, height: f32) -> Self {
        debug!(width, height, "Canvas created");
        Self {
            width,
            height,
            elements: Vec::new(),
            scale_mode: ScaleMode::Fixed,
            next_id: 1,
        }
    }

    /// Resize the canvas (does not automatically re-layout).
    pub fn resize(&mut self, width: f32, height: f32) {
        debug!(
            old_width = self.width,
            old_height = self.height,
            new_width = width,
            new_height = height,
            "Canvas resized"
        );
        self.width = width;
        self.height = height;
    }

    /// Set the next element ID counter (used during deserialization).
    pub fn set_next_id(&mut self, id: u32) {
        self.next_id = self.next_available_id(id);
    }

    /// Add a [`UiElement`], assigning it a new [`ElementId`].
    ///
    /// The element's `id` field is overwritten.  Returns the assigned id.
    pub fn add_element(&mut self, mut element: UiElement) -> ElementId {
        let id = ElementId(self.next_available_id(self.next_id));
        self.next_id = self.next_available_id(increment_element_id(id.0));
        element.id = id;
        debug!(element_id = ?id, "Element added to canvas");
        self.elements.push(element);
        id
    }

    /// Insert an element with an explicit stable ID.
    ///
    /// This is used by deferred script and network command streams where the
    /// producer must receive a usable element handle before the command is
    /// applied on the engine thread. Normal in-process callers should prefer
    /// [`Self::add_element`].
    pub fn insert_element(
        &mut self,
        id: ElementId,
        mut element: UiElement,
    ) -> Result<ElementId, crate::UiError> {
        if !(FIRST_ELEMENT_ID..=LAST_ELEMENT_ID).contains(&id.0) {
            return Err(crate::UiError::InvalidElementId(id));
        }
        if self.elements.iter().any(|existing| existing.id == id) {
            return Err(crate::UiError::DuplicateElementId(id));
        }
        element.id = id;
        self.elements.push(element);
        let next_candidate = if id.0 >= self.next_id {
            increment_element_id(id.0)
        } else {
            self.next_id
        };
        self.next_id = self.next_available_id(next_candidate);
        debug!(element_id = ?id, "Element inserted into canvas");
        Ok(id)
    }

    fn next_available_id(&self, requested: u32) -> u32 {
        let start = normalize_element_id(requested);
        let mut candidate = start;
        loop {
            if self
                .elements
                .iter()
                .all(|element| element.id.0 != candidate)
            {
                return candidate;
            }
            candidate = increment_element_id(candidate);
            assert_ne!(candidate, start, "Canvas exhausted all valid element IDs");
        }
    }

    /// Remove an element by id.
    ///
    /// Also removes it from any parent's children list.
    /// Returns `true` if the element was found and removed.
    pub fn remove_element(&mut self, id: ElementId) -> bool {
        let pos = self.elements.iter().position(|e| e.id == id);
        if let Some(idx) = pos {
            self.elements.remove(idx);
            // Remove from any parent's children list.
            for el in &mut self.elements {
                el.children.retain(|c| *c != id);
            }
            debug!(element_id = ?id, "Element removed from canvas");
            true
        } else {
            false
        }
    }

    /// Borrow an element by id.
    pub fn get_element(&self, id: ElementId) -> Option<&UiElement> {
        self.elements.iter().find(|e| e.id == id)
    }

    /// Mutably borrow an element by id.
    pub fn get_element_mut(&mut self, id: ElementId) -> Option<&mut UiElement> {
        self.elements.iter_mut().find(|e| e.id == id)
    }

    /// Remove all elements.
    pub fn clear(&mut self) {
        let count = self.elements.len();
        self.elements.clear();
        self.next_id = 1;
        debug!(count, "Canvas cleared");
    }

    /// Resolve all element layouts into pixel rectangles.
    ///
    /// For each element, [`Layout::compute`] is called with the canvas as the
    /// parent rect.  Child elements use their parent's computed rect as the
    /// parent rect.
    ///
    /// Elements are processed in list order so parents are guaranteed to be
    /// resolved before their children.
    pub fn layout_all(&mut self) {
        let canvas_rect = UiRect::new(0.0, 0.0, self.width, self.height);

        // Build a lookup: ElementId -> index in elements slice.
        let mut id_to_idx: std::collections::HashMap<ElementId, usize> =
            std::collections::HashMap::with_capacity(self.elements.len());
        for (i, el) in self.elements.iter().enumerate() {
            id_to_idx.insert(el.id, i);
        }

        // Compute all rects in topological order (parents before children).
        // We iterate multiple times: first resolve roots (no parent), then
        // children whose parent has been resolved, until all are done.
        let n = self.elements.len();
        let mut resolved = vec![false; n];
        let mut rects = vec![UiRect::ZERO; n];

        // Compute parent for each element: which element claims this as child.
        let mut parent_of: Vec<Option<ElementId>> = vec![None; n];
        for (i, el) in self.elements.iter().enumerate() {
            for (j, other) in self.elements.iter().enumerate() {
                if i != j && other.children.contains(&el.id) {
                    parent_of[i] = Some(other.id);
                    break;
                }
            }
        }

        // Resolve iteratively: roots first, then their children, etc.
        let mut changed = true;
        while changed {
            changed = false;
            for i in 0..n {
                if resolved[i] {
                    continue;
                }
                let parent_rect = match parent_of[i] {
                    None => canvas_rect, // root → canvas
                    Some(pid) => {
                        if let Some(&p_idx) = id_to_idx.get(&pid) {
                            if resolved[p_idx] {
                                rects[p_idx]
                            } else {
                                continue;
                            }
                        } else {
                            canvas_rect // parent missing → canvas
                        }
                    }
                };
                rects[i] = Layout::compute(&parent_rect, &self.elements[i].layout);
                self.elements[i].rect = rects[i];
                resolved[i] = true;
                changed = true;
            }
        }
    }

    /// Build a list of [`UiBatch`]es from the enabled elements on this canvas.
    ///
    /// Elements are sorted by `z_order` (ascending).  Consecutive elements
    /// sharing the same `z_order` *and* texture are merged into one batch.
    /// Returns an empty Vec when there are no enabled elements.
    ///
    /// Call [`Canvas::layout_all`] before this to ensure pixel rects are current.
    pub fn build_batches(&self) -> Vec<UiBatch> {
        build_canvas_batches(self, self.width, self.height, None)
    }

    /// Build batches for a viewport and include current hover/press visuals.
    ///
    /// Element layout stays in logical Canvas coordinates. Fit-width and
    /// fit-height canvases scale the generated vertices and clipping region
    /// to the supplied viewport while retaining their aspect ratio.
    pub fn build_batches_for_viewport(
        &self,
        viewport_width: f32,
        viewport_height: f32,
        input: Option<&UiInputState>,
    ) -> Vec<UiBatch> {
        build_canvas_batches(self, viewport_width, viewport_height, input)
    }
}

fn build_canvas_batches(
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

const FIRST_ELEMENT_ID: u32 = 1;
const LAST_ELEMENT_ID: u32 = u32::MAX - 1;

fn normalize_element_id(id: u32) -> u32 {
    if (FIRST_ELEMENT_ID..=LAST_ELEMENT_ID).contains(&id) {
        id
    } else {
        FIRST_ELEMENT_ID
    }
}

fn increment_element_id(id: u32) -> u32 {
    if id >= LAST_ELEMENT_ID {
        FIRST_ELEMENT_ID
    } else {
        id + 1
    }
}

fn encode_vec2(value: glam::Vec2) -> Value {
    Value::Map(BTreeMap::from([
        ("x".into(), Value::Float32(value.x)),
        ("y".into(), Value::Float32(value.y)),
    ]))
}

fn decode_vec2(value: &Value) -> Option<glam::Vec2> {
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

fn encode_color(color: Color) -> Value {
    let channel = |value: u8| f32::from(value) / 255.0;
    Value::Color([
        channel(color.r),
        channel(color.g),
        channel(color.b),
        channel(color.a),
    ])
}

fn decode_color(value: &Value) -> Option<Color> {
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

fn encode_optional_string(fields: &mut BTreeMap<String, Value>, key: &str, value: &Option<String>) {
    if let Some(value) = value {
        fields.insert(key.into(), Value::Str(value.clone()));
    }
}

fn decode_optional_string(fields: &BTreeMap<String, Value>, key: &str) -> Option<Option<String>> {
    match fields.get(key) {
        None => Some(None),
        Some(Value::Str(value)) => Some(Some(value.clone())),
        Some(_) => None,
    }
}

fn encode_element_kind(kind: &UiElementKind) -> Value {
    let mut fields = BTreeMap::new();
    match kind {
        UiElementKind::Panel { color } => {
            fields.insert("type".into(), Value::Enum("Panel".into()));
            fields.insert("color".into(), encode_color(*color));
        }
        UiElementKind::Image { texture_id, color } => {
            fields.insert("type".into(), Value::Enum("Image".into()));
            fields.insert("texture_id".into(), Value::Asset(AssetId::new(texture_id)));
            fields.insert("color".into(), encode_color(*color));
        }
        UiElementKind::Text {
            content,
            font_size,
            color,
        } => {
            fields.insert("type".into(), Value::Enum("Text".into()));
            fields.insert("content".into(), Value::Str(content.clone()));
            fields.insert("font_size".into(), Value::Float32(*font_size));
            fields.insert("color".into(), encode_color(*color));
        }
        UiElementKind::Button {
            label,
            normal_color,
            hover_color,
            pressed_color,
            callback_id,
        } => {
            fields.insert("type".into(), Value::Enum("Button".into()));
            fields.insert("label".into(), Value::Str(label.clone()));
            fields.insert("normal_color".into(), encode_color(*normal_color));
            fields.insert("hover_color".into(), encode_color(*hover_color));
            fields.insert("pressed_color".into(), encode_color(*pressed_color));
            encode_optional_string(&mut fields, "callback_id", callback_id);
        }
        UiElementKind::Toggle {
            label,
            is_on,
            color_on,
            color_off,
            callback_id,
        } => {
            fields.insert("type".into(), Value::Enum("Toggle".into()));
            fields.insert("label".into(), Value::Str(label.clone()));
            fields.insert("is_on".into(), Value::Bool(*is_on));
            fields.insert("color_on".into(), encode_color(*color_on));
            fields.insert("color_off".into(), encode_color(*color_off));
            encode_optional_string(&mut fields, "callback_id", callback_id);
        }
        UiElementKind::Checkbox {
            label,
            checked,
            color,
            callback_id,
        } => {
            fields.insert("type".into(), Value::Enum("Checkbox".into()));
            fields.insert("label".into(), Value::Str(label.clone()));
            fields.insert("checked".into(), Value::Bool(*checked));
            fields.insert("color".into(), encode_color(*color));
            encode_optional_string(&mut fields, "callback_id", callback_id);
        }
        UiElementKind::Slider {
            label,
            value,
            min,
            max,
            callback_id,
        } => {
            fields.insert("type".into(), Value::Enum("Slider".into()));
            fields.insert("label".into(), Value::Str(label.clone()));
            fields.insert("value".into(), Value::Float32(*value));
            fields.insert("min".into(), Value::Float32(*min));
            fields.insert("max".into(), Value::Float32(*max));
            encode_optional_string(&mut fields, "callback_id", callback_id);
        }
        UiElementKind::ScrollView {
            scroll_x,
            scroll_y,
            content_width,
            content_height,
            color,
        } => {
            fields.insert("type".into(), Value::Enum("ScrollView".into()));
            fields.insert("scroll_x".into(), Value::Float32(*scroll_x));
            fields.insert("scroll_y".into(), Value::Float32(*scroll_y));
            fields.insert("content_width".into(), Value::Float32(*content_width));
            fields.insert("content_height".into(), Value::Float32(*content_height));
            fields.insert("color".into(), encode_color(*color));
        }
    }
    Value::Map(fields)
}

fn string_field<'a>(fields: &'a BTreeMap<String, Value>, key: &str) -> Option<&'a str> {
    match fields.get(key) {
        Some(Value::Str(value)) => Some(value),
        _ => None,
    }
}

fn float_field(fields: &BTreeMap<String, Value>, key: &str) -> Option<f32> {
    match fields.get(key) {
        Some(Value::Float32(value)) if value.is_finite() => Some(*value),
        _ => None,
    }
}

fn color_field(fields: &BTreeMap<String, Value>, key: &str) -> Option<Color> {
    decode_color(fields.get(key)?)
}

fn decode_element_kind(value: &Value) -> Option<UiElementKind> {
    let Value::Map(fields) = value else {
        return None;
    };
    let kind = match fields.get("type") {
        Some(Value::Enum(kind) | Value::Str(kind)) => kind.as_str(),
        _ => return None,
    };
    match kind {
        "Panel" => Some(UiElementKind::Panel {
            color: color_field(fields, "color")?,
        }),
        "Image" => {
            let texture_id = match fields.get("texture_id") {
                Some(Value::Asset(asset)) => asset.id.clone(),
                Some(Value::Str(asset)) => asset.clone(),
                _ => return None,
            };
            Some(UiElementKind::Image {
                texture_id,
                color: color_field(fields, "color")?,
            })
        }
        "Text" => Some(UiElementKind::Text {
            content: string_field(fields, "content")?.into(),
            font_size: float_field(fields, "font_size")?,
            color: color_field(fields, "color")?,
        }),
        "Button" => Some(UiElementKind::Button {
            label: string_field(fields, "label")?.into(),
            normal_color: color_field(fields, "normal_color")?,
            hover_color: color_field(fields, "hover_color")?,
            pressed_color: color_field(fields, "pressed_color")?,
            callback_id: decode_optional_string(fields, "callback_id")?,
        }),
        "Toggle" => Some(UiElementKind::Toggle {
            label: string_field(fields, "label")?.into(),
            is_on: match fields.get("is_on") {
                Some(Value::Bool(value)) => *value,
                _ => return None,
            },
            color_on: color_field(fields, "color_on")?,
            color_off: color_field(fields, "color_off")?,
            callback_id: decode_optional_string(fields, "callback_id")?,
        }),
        "Checkbox" => Some(UiElementKind::Checkbox {
            label: string_field(fields, "label")?.into(),
            checked: match fields.get("checked") {
                Some(Value::Bool(value)) => *value,
                _ => return None,
            },
            color: color_field(fields, "color")?,
            callback_id: decode_optional_string(fields, "callback_id")?,
        }),
        "Slider" => Some(UiElementKind::Slider {
            label: string_field(fields, "label")?.into(),
            value: float_field(fields, "value")?,
            min: float_field(fields, "min")?,
            max: float_field(fields, "max")?,
            callback_id: decode_optional_string(fields, "callback_id")?,
        }),
        "ScrollView" => Some(UiElementKind::ScrollView {
            scroll_x: float_field(fields, "scroll_x")?,
            scroll_y: float_field(fields, "scroll_y")?,
            content_width: float_field(fields, "content_width")?,
            content_height: float_field(fields, "content_height")?,
            color: color_field(fields, "color")?,
        }),
        _ => None,
    }
}

fn encode_layout(layout: Layout) -> Value {
    Value::Map(BTreeMap::from([
        ("anchor_min".into(), encode_vec2(layout.anchor_min)),
        ("anchor_max".into(), encode_vec2(layout.anchor_max)),
        ("offset_min".into(), encode_vec2(layout.offset_min)),
        ("offset_max".into(), encode_vec2(layout.offset_max)),
    ]))
}

fn decode_layout(value: &Value) -> Option<Layout> {
    let Value::Map(fields) = value else {
        return None;
    };
    Some(Layout::new(
        decode_vec2(fields.get("anchor_min")?)?,
        decode_vec2(fields.get("anchor_max")?)?,
        decode_vec2(fields.get("offset_min")?)?,
        decode_vec2(fields.get("offset_max")?)?,
    ))
}

fn encode_element(element: &UiElement) -> Value {
    Value::Map(BTreeMap::from([
        ("id".into(), Value::UInt(u64::from(element.id.0))),
        ("kind".into(), encode_element_kind(&element.kind)),
        ("layout".into(), encode_layout(element.layout)),
        ("z_order".into(), Value::Int(i64::from(element.z_order))),
        ("enabled".into(), Value::Bool(element.enabled)),
        (
            "children".into(),
            Value::List(
                element
                    .children
                    .iter()
                    .map(|child| Value::UInt(u64::from(child.0)))
                    .collect(),
            ),
        ),
    ]))
}

fn decode_element_id(value: &Value) -> Option<ElementId> {
    let Value::UInt(value) = value else {
        return None;
    };
    let value = u32::try_from(*value).ok()?;
    ((FIRST_ELEMENT_ID..=LAST_ELEMENT_ID).contains(&value)).then_some(ElementId(value))
}

fn decode_children(value: Option<&Value>) -> Vec<ElementId> {
    let Some(Value::List(values)) = value else {
        return Vec::new();
    };
    values.iter().filter_map(decode_element_id).collect()
}

fn decode_element(value: &Value) -> Option<UiElement> {
    let Value::Map(fields) = value else {
        return None;
    };
    let id = decode_element_id(fields.get("id")?)?;
    let kind = decode_element_kind(fields.get("kind")?)?;
    let layout = decode_layout(fields.get("layout")?)?;
    let z_order = match fields.get("z_order") {
        Some(Value::Int(value)) => i32::try_from(*value).ok()?,
        _ => return None,
    };
    let enabled = match fields.get("enabled") {
        Some(Value::Bool(value)) => *value,
        _ => return None,
    };
    Some(UiElement {
        id,
        kind,
        layout,
        z_order,
        enabled,
        children: decode_children(fields.get("children")),
        rect: UiRect::ZERO,
    })
}

fn creates_child_cycle(
    parent: ElementId,
    child: ElementId,
    parent_of: &BTreeMap<ElementId, ElementId>,
) -> bool {
    let mut ancestor = Some(parent);
    while let Some(id) = ancestor {
        if id == child {
            return true;
        }
        ancestor = parent_of.get(&id).copied();
    }
    false
}

/// Turn decoded child references into a deterministic forest.
///
/// Unknown IDs, self-links, duplicate links, second parents, and edges that
/// would close a cycle are dropped. For ambiguous data, the first link in
/// serialized element/child order wins.
fn sanitize_element_children(elements: &mut [UiElement]) {
    let known_ids: BTreeSet<_> = elements.iter().map(|element| element.id).collect();
    let mut parent_of = BTreeMap::new();

    for element in elements {
        let parent = element.id;
        let mut accepted = Vec::new();
        let mut seen_children = BTreeSet::new();
        for child in std::mem::take(&mut element.children) {
            let valid = known_ids.contains(&child)
                && child != parent
                && seen_children.insert(child)
                && !parent_of.contains_key(&child)
                && !creates_child_cycle(parent, child, &parent_of);
            if valid {
                parent_of.insert(child, parent);
                accepted.push(child);
            } else {
                tracing::warn!(?parent, ?child, "discarding invalid Canvas child link");
            }
        }
        element.children = accepted;
    }
}

fn decode_elements(value: Option<&Value>) -> Vec<UiElement> {
    let Some(value) = value else {
        return Vec::new();
    };
    let Value::List(values) = value else {
        tracing::warn!("discarding malformed Canvas elements field");
        return Vec::new();
    };

    let mut elements = Vec::with_capacity(values.len());
    let mut ids = BTreeSet::new();
    for value in values {
        let Some(element) = decode_element(value) else {
            tracing::warn!("discarding malformed Canvas element");
            continue;
        };
        if !ids.insert(element.id) {
            tracing::warn!(id = element.id.0, "discarding duplicate Canvas element ID");
            continue;
        }
        elements.push(element);
    }
    sanitize_element_children(&mut elements);
    elements
}

/// Serialize a [`Canvas`] component into a readable field-map contract.
///
/// Persistent element data is stored under `elements` as a `Value::List` of
/// maps. Computed rectangles are omitted and recalculated by
/// [`Canvas::layout_all`]. Image textures use `Value::Asset` so recursive
/// scene dependency collection can discover them.
pub(crate) fn serialize_canvas(component: &dyn std::any::Any) -> BTreeMap<String, Value> {
    let canvas = component.downcast_ref::<Canvas>().expect("Canvas expected");
    let mut fields = BTreeMap::new();

    fields.insert("width".into(), Value::Float32(canvas.width));
    fields.insert("height".into(), Value::Float32(canvas.height));
    fields.insert(
        "scale_mode".into(),
        Value::Enum(format!("{:?}", canvas.scale_mode)),
    );
    fields.insert("next_id".into(), Value::UInt(u64::from(canvas.next_id)));
    fields.insert(
        "element_count".into(),
        Value::UInt(canvas.elements.len() as u64),
    );
    fields.insert(
        "elements".into(),
        Value::List(canvas.elements.iter().map(encode_element).collect()),
    );

    fields
}

/// Deserialize a [`Canvas`] from a field map.
///
/// Metadata-only field maps from older scenes remain valid and produce an
/// empty canvas. Malformed element records and duplicate/invalid IDs are
/// dropped. Invalid child links are normalized to a deterministic forest;
/// see [`sanitize_element_children`] for the first-link-wins policy.
fn deserialize_canvas(fields: &BTreeMap<String, Value>) -> Box<dyn std::any::Any> {
    let width = match fields.get("width") {
        Some(Value::Float32(value)) if value.is_finite() && *value >= 0.0 => *value,
        _ => 800.0,
    };
    let height = match fields.get("height") {
        Some(Value::Float32(value)) if value.is_finite() && *value >= 0.0 => *value,
        _ => 600.0,
    };
    let mut canvas = Canvas::new(width, height);

    if let Some(Value::Enum(mode) | Value::Str(mode)) = fields.get("scale_mode") {
        canvas.scale_mode = match mode.as_str() {
            "FitWidth" => ScaleMode::FitWidth,
            "FitHeight" => ScaleMode::FitHeight,
            _ => ScaleMode::Fixed,
        };
    }

    canvas.elements = decode_elements(fields.get("elements"));

    let requested_next_id = match fields.get("next_id") {
        Some(Value::UInt(value)) => u32::try_from(*value).unwrap_or(FIRST_ELEMENT_ID),
        _ => FIRST_ELEMENT_ID,
    };
    let derived_next_id = canvas
        .elements
        .iter()
        .map(|element| element.id.0)
        .max()
        .map(increment_element_id)
        .unwrap_or(FIRST_ELEMENT_ID);
    let next_id = if derived_next_id == FIRST_ELEMENT_ID
        && canvas
            .elements
            .iter()
            .any(|element| element.id.0 == LAST_ELEMENT_ID)
    {
        requested_next_id
    } else {
        requested_next_id.max(derived_next_id)
    };
    canvas.set_next_id(next_id);

    Box::new(canvas)
}

// ECS Component
// ---------------------------------------------------------------------------

impl engine_scene::Component for Canvas {
    const TYPE_ID: &'static str = "engine.canvas";
}

// ---------------------------------------------------------------------------
// ECS registration
// ---------------------------------------------------------------------------

/// Register UI extensions (Canvas component) with the engine's component
/// registry.
///
/// Call this during engine initialisation so that the Canvas component
/// type is recognised by the ECS world.
pub fn register_ui_extensions(component_registry: &mut engine_scene::registry::ComponentRegistry) {
    use engine_scene::registry::{ComponentExtension, ComponentMeta};
    use engine_scene::{Component, ComponentStorageDyn, SparseSet};

    let _ = component_registry.register(ComponentExtension {
        meta: ComponentMeta {
            type_id: Canvas::TYPE_ID,
            display_name: "UI Canvas",
            schema_version: (0, 1, 0),
            has_editor: true,
            // Scripts drive canvases through the retained `UICanvas` managed
            // handles, never the generic Components bridge.
            script_access: engine_scene::registry::ScriptAccess::DedicatedApi,
        },
        storage_factory: || -> Box<dyn ComponentStorageDyn> {
            Box::new(SparseSet::<Canvas>::new())
        },
        serialize: Some(serialize_canvas),
        deserialize: Some(deserialize_canvas),
    });
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
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
}
