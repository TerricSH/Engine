use crate::Component;
use engine_serialize::{AssetId, Value};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Marks an entity as renderable with a mesh and material.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Renderable {
    pub mesh_asset: String,
    pub material_asset: String,
    pub visible: bool,
    pub cast_shadows: bool,
    pub render_layer: String,
}

impl Component for Renderable {
    const TYPE_ID: &'static str = "engine.renderable";
}

/// Serialize a renderable using the same field names as scene files.
pub fn serialize_renderable_fields(renderable: &Renderable) -> BTreeMap<String, Value> {
    BTreeMap::from([
        (
            "mesh".to_string(),
            Value::Asset(AssetId::new(renderable.mesh_asset.clone())),
        ),
        (
            "material".to_string(),
            Value::Asset(AssetId::new(renderable.material_asset.clone())),
        ),
        ("visible".to_string(), Value::Bool(renderable.visible)),
        (
            "cast_shadows".to_string(),
            Value::Bool(renderable.cast_shadows),
        ),
        (
            "render_layer".to_string(),
            Value::Str(renderable.render_layer.clone()),
        ),
    ])
}

/// Deserialize a complete renderable field map. Registry validation runs
/// before this hook, so defaults only serve scene compatibility.
pub fn deserialize_renderable_fields(fields: &BTreeMap<String, Value>) -> Renderable {
    Renderable {
        mesh_asset: match fields.get("mesh") {
            Some(Value::Asset(asset)) => asset.id.clone(),
            _ => String::new(),
        },
        material_asset: match fields.get("material") {
            Some(Value::Asset(asset)) => asset.id.clone(),
            _ => String::new(),
        },
        visible: matches!(fields.get("visible"), None | Some(Value::Bool(true))),
        cast_shadows: matches!(fields.get("cast_shadows"), None | Some(Value::Bool(true))),
        render_layer: match fields.get("render_layer") {
            Some(Value::Str(layer)) => layer.clone(),
            _ => "Default".to_string(),
        },
    }
}

pub fn serialize_renderable(component: &dyn std::any::Any) -> BTreeMap<String, Value> {
    component
        .downcast_ref::<Renderable>()
        .map(serialize_renderable_fields)
        .unwrap_or_default()
}

pub fn deserialize_renderable(fields: &BTreeMap<String, Value>) -> Box<dyn std::any::Any> {
    Box::new(deserialize_renderable_fields(fields))
}

/// Validate script/scene field maps before replacing the live renderable.
pub fn validate_renderable_fields(fields: &BTreeMap<String, Value>) -> Result<(), String> {
    for required in ["mesh", "material"] {
        match fields.get(required) {
            Some(Value::Asset(asset)) if valid_identifier(&asset.id, 256) => {}
            Some(_) => return Err(format!("renderable field '{required}' must be an asset")),
            None => return Err(format!("renderable field '{required}' is required")),
        }
    }
    for (name, value) in fields {
        let valid = match name.as_str() {
            "mesh" | "material" => {
                matches!(value, Value::Asset(asset) if valid_identifier(&asset.id, 256))
            }
            "visible" | "cast_shadows" => matches!(value, Value::Bool(_)),
            "render_layer" => {
                matches!(value, Value::Str(layer) if valid_identifier(layer, 128))
            }
            _ => false,
        };
        if !valid {
            return Err(format!(
                "renderable field '{name}' has an unsupported name or value"
            ));
        }
    }
    Ok(())
}

fn valid_identifier(value: &str, max_bytes: usize) -> bool {
    !value.is_empty() && value.len() <= max_bytes && !value.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renderable_field_map_roundtrips_and_validates() {
        let renderable = Renderable {
            mesh_asset: "ship.mesh".into(),
            material_asset: "ship.paint.red".into(),
            visible: true,
            cast_shadows: false,
            render_layer: "Vehicle".into(),
        };
        let fields = serialize_renderable_fields(&renderable);
        validate_renderable_fields(&fields).unwrap();
        let decoded = deserialize_renderable_fields(&fields);
        assert_eq!(decoded.mesh_asset, renderable.mesh_asset);
        assert_eq!(decoded.material_asset, renderable.material_asset);
        assert_eq!(decoded.visible, renderable.visible);
        assert_eq!(decoded.cast_shadows, renderable.cast_shadows);
        assert_eq!(decoded.render_layer, renderable.render_layer);
    }

    #[test]
    fn renderable_validation_rejects_unknown_or_missing_fields() {
        let mut fields = serialize_renderable_fields(&Renderable {
            mesh_asset: "ship.mesh".into(),
            material_asset: "ship.material".into(),
            visible: true,
            cast_shadows: true,
            render_layer: "Default".into(),
        });
        fields.remove("material");
        assert!(validate_renderable_fields(&fields).is_err());
        fields.insert(
            "material".into(),
            Value::Asset(AssetId::new("ship.material")),
        );
        fields.insert("unknown".into(), Value::Bool(true));
        assert!(validate_renderable_fields(&fields).is_err());
    }
}
