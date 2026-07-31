use super::batching::{
    decode_color, decode_vec2, encode_color, encode_vec2, increment_element_id, FIRST_ELEMENT_ID,
    LAST_ELEMENT_ID,
};
use super::*;

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

pub(super) fn encode_element(element: &UiElement) -> Value {
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
pub(super) fn deserialize_canvas(fields: &BTreeMap<String, Value>) -> Box<dyn std::any::Any> {
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
