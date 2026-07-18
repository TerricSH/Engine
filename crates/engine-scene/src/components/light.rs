use crate::Component;
use engine_serialize::Value;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::field_as_f32;

/// Kind of light source.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LightKind {
    Directional,
    Point,
    Spot,
}

/// Light component.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Light {
    pub kind: LightKind,
    pub color: [f32; 3],
    /// Lux for directional, lumens for point/spot.
    pub intensity: f32,
    /// Maximum range of the light (for culling).
    pub range: f32,
    /// Inner/outer cone angles in radians (only for Spot).
    pub spot_angles: Option<[f32; 2]>,
    /// Shadow mode: 0 = off, 1 = hard, 2 = soft.
    pub shadow_mode: u8,
    /// Light direction in entity-local space. Extraction transforms and
    /// normalizes it through the entity's complete parent hierarchy.
    pub direction: [f32; 3],
}

impl Component for Light {
    const TYPE_ID: &'static str = "engine.light";
}

// ---------------------------------------------------------------------------
// Scene field-map serde
// ---------------------------------------------------------------------------

/// Serialize a [`Light`] into the field-map layout used by scene files.
///
/// This is the single source of truth for the light field layout: scene
/// loading/saving, the component registry hooks, and the script component
/// bridge all share it.
pub fn serialize_light_fields(light: &Light) -> BTreeMap<String, Value> {
    let mut fields = BTreeMap::new();
    fields.insert(
        "kind".to_string(),
        Value::Enum(match light.kind {
            LightKind::Directional => "Directional".to_string(),
            LightKind::Point => "Point".to_string(),
            LightKind::Spot => "Spot".to_string(),
        }),
    );
    fields.insert("color".to_string(), Value::Vec3(light.color));
    fields.insert("intensity".to_string(), Value::Float32(light.intensity));
    fields.insert("range".to_string(), Value::Float32(light.range));
    if let Some(angles) = light.spot_angles {
        fields.insert(
            "spot_angles".to_string(),
            Value::List(vec![Value::Float32(angles[0]), Value::Float32(angles[1])]),
        );
    }
    fields.insert(
        "shadow_mode".to_string(),
        Value::UInt(light.shadow_mode as u64),
    );
    fields.insert("direction".to_string(), Value::Vec3(light.direction));
    fields
}

/// Build a [`Light`] from a scene field map, applying authored defaults for
/// any missing field with the same tolerance as the scene loader.
pub fn deserialize_light_fields(fields: &BTreeMap<String, Value>) -> Light {
    let kind = match fields.get("kind") {
        Some(Value::Enum(s)) if s == "Point" => LightKind::Point,
        Some(Value::Enum(s)) if s == "Spot" => LightKind::Spot,
        _ => LightKind::Directional,
    };
    let color = match fields.get("color") {
        Some(Value::Vec3(c)) => *c,
        _ => [1.0, 1.0, 1.0],
    };
    let intensity = match fields.get("intensity") {
        Some(Value::Float32(v)) => *v,
        Some(Value::Float64(v)) => *v as f32,
        _ => 1.0,
    };
    let range = match fields.get("range") {
        Some(Value::Float32(v)) => *v,
        Some(Value::Float64(v)) => *v as f32,
        _ => 10.0,
    };
    let spot_angles = match fields.get("spot_angles") {
        Some(Value::List(items)) if items.len() == 2 => {
            Some([field_as_f32(&items[0]), field_as_f32(&items[1])])
        }
        _ => None,
    };
    let shadow_mode = match fields.get("shadow_mode") {
        Some(Value::UInt(v)) => *v as u8,
        _ => 0,
    };
    let direction = match fields.get("direction") {
        Some(Value::Vec3(d)) => *d,
        _ => [0.0, -1.0, 0.0],
    };
    Light {
        kind,
        color,
        intensity,
        range,
        spot_angles,
        shadow_mode,
        direction,
    }
}

/// Registry hook: serialize a type-erased [`Light`] into its field map.
pub fn serialize_light(component: &dyn std::any::Any) -> BTreeMap<String, Value> {
    let light = component.downcast_ref::<Light>().expect("Light expected");
    serialize_light_fields(light)
}

/// Registry hook: build a type-erased [`Light`] from a field map.
pub fn deserialize_light(fields: &BTreeMap<String, Value>) -> Box<dyn std::any::Any> {
    Box::new(deserialize_light_fields(fields))
}
