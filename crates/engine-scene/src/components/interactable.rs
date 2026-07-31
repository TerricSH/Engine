use std::collections::BTreeMap;

use engine_serialize::Value;
use serde::{Deserialize, Serialize};

use crate::Component;

use super::field_as_f32;

/// Scene-authored marker and presentation data for a player-usable entity.
///
/// Game-specific effects remain in project scripts. The engine owns only the
/// stable targeting contract: whether the entity can be used, the prompt and
/// action key shown to gameplay code, its maximum use distance, and whether
/// the project may offer a grab interaction.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Interactable {
    pub enabled: bool,
    pub prompt: String,
    pub action: String,
    pub max_distance: f32,
    pub grabbable: bool,
}

impl Default for Interactable {
    fn default() -> Self {
        Self {
            enabled: true,
            prompt: "Use".to_string(),
            action: "use".to_string(),
            max_distance: 3.0,
            grabbable: false,
        }
    }
}

impl Component for Interactable {
    const TYPE_ID: &'static str = "engine.interactable";
}

pub fn serialize_interactable_fields(interactable: &Interactable) -> BTreeMap<String, Value> {
    BTreeMap::from([
        ("enabled".to_string(), Value::Bool(interactable.enabled)),
        (
            "prompt".to_string(),
            Value::Str(interactable.prompt.clone()),
        ),
        (
            "action".to_string(),
            Value::Str(interactable.action.clone()),
        ),
        (
            "max_distance".to_string(),
            Value::Float32(interactable.max_distance),
        ),
        ("grabbable".to_string(), Value::Bool(interactable.grabbable)),
    ])
}

pub fn deserialize_interactable_fields(fields: &BTreeMap<String, Value>) -> Interactable {
    let defaults = Interactable::default();
    Interactable {
        enabled: match fields.get("enabled") {
            Some(Value::Bool(value)) => *value,
            _ => defaults.enabled,
        },
        prompt: match fields.get("prompt") {
            Some(Value::Str(value)) => value.clone(),
            _ => defaults.prompt,
        },
        action: match fields.get("action") {
            Some(Value::Str(value)) => value.clone(),
            _ => defaults.action,
        },
        max_distance: fields
            .get("max_distance")
            .map(field_as_f32)
            .unwrap_or(defaults.max_distance),
        grabbable: match fields.get("grabbable") {
            Some(Value::Bool(value)) => *value,
            _ => defaults.grabbable,
        },
    }
}

pub fn validate_interactable_fields(fields: &BTreeMap<String, Value>) -> Result<(), String> {
    let interactable = deserialize_interactable_fields(fields);
    if interactable.prompt.len() > 256
        || interactable
            .prompt
            .chars()
            .any(|character| character.is_control() && character != '\n' && character != '\t')
    {
        return Err(
            "interactable prompt must contain at most 256 UTF-8 bytes and no unsupported control characters"
                .into(),
        );
    }
    if interactable.action.is_empty()
        || interactable.action.len() > 64
        || !interactable
            .action
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err(
            "interactable action must contain 1 to 64 ASCII letters, digits, hyphens, or underscores"
                .into(),
        );
    }
    if !interactable.max_distance.is_finite() || !(0.1..=100.0).contains(&interactable.max_distance)
    {
        return Err("interactable max_distance must be finite and in the range 0.1..=100".into());
    }
    Ok(())
}

pub fn serialize_interactable(component: &dyn std::any::Any) -> BTreeMap<String, Value> {
    let interactable = component
        .downcast_ref::<Interactable>()
        .expect("Interactable expected");
    serialize_interactable_fields(interactable)
}

pub fn deserialize_interactable(fields: &BTreeMap<String, Value>) -> Box<dyn std::any::Any> {
    Box::new(deserialize_interactable_fields(fields))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_map_roundtrip_preserves_interaction_contract() {
        let interactable = Interactable {
            enabled: true,
            prompt: "Open door".into(),
            action: "open".into(),
            max_distance: 2.5,
            grabbable: false,
        };
        let fields = serialize_interactable_fields(&interactable);
        assert!(validate_interactable_fields(&fields).is_ok());
        assert_eq!(deserialize_interactable_fields(&fields), interactable);
    }

    #[test]
    fn validation_bounds_untrusted_prompt_action_and_distance() {
        for fields in [
            BTreeMap::from([("action".into(), Value::Str("../use".into()))]),
            BTreeMap::from([("prompt".into(), Value::Str("x".repeat(257)))]),
            BTreeMap::from([("max_distance".into(), Value::Float32(f32::NAN))]),
            BTreeMap::from([("max_distance".into(), Value::Float32(101.0))]),
        ] {
            assert!(validate_interactable_fields(&fields).is_err());
        }
    }
}
