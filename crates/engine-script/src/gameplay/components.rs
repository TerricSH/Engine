use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::validation::validate_entity_id;

// ---------------------------------------------------------------------------
// Typed component access
// ---------------------------------------------------------------------------

/// Maximum fields a single `set_component` command may carry.
///
/// The curated component set has far fewer fields; the cap exists so a
/// misbehaving script cannot push unbounded payloads through the bridge.
pub const MAX_COMPONENT_FIELDS: usize = 64;

/// Maximum nesting depth for list/map component field values.
pub const MAX_COMPONENT_VALUE_DEPTH: usize = 16;

/// Maximum items in a single list component field value.
pub const MAX_COMPONENT_LIST_ITEMS: usize = 256;

/// Maximum byte length of a string-like component field value.
pub const MAX_COMPONENT_VALUE_STRING_BYTES: usize = 4096;

/// Maximum component queries the runtime buffers from one command drain.
/// Queries beyond the cap are rejected with a script diagnostic, mirroring
/// [`MAX_PENDING_PHYSICS_QUERIES`].
pub const MAX_PENDING_COMPONENT_QUERIES: usize = 256;

/// JSON-friendly field value of a script-accessible ECS component.
///
/// Component payloads cross the gameplay bridge as string-keyed maps of these
/// values so the data-only contract stays free of engine ECS types. The
/// variant set deliberately mirrors the `engine-serialize` scene `Value`
/// variants used by the registered component serde hooks; conversion helpers
/// translate between the two without loss for every supported variant except
/// entity references, which are not yet part of this surface.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum GameplayComponentValue {
    Bool(bool),
    Int(i64),
    UInt(u64),
    Float(f32),
    Str(String),
    /// Enumeration case selected by name (for example a rigid-body type).
    Enum(String),
    /// Cooked asset identifier referenced by a component field.
    Asset(String),
    Vec3([f32; 3]),
    Quat([f32; 4]),
    Color([f32; 4]),
    List(Vec<GameplayComponentValue>),
    Map(BTreeMap<String, GameplayComponentValue>),
}

impl GameplayComponentValue {
    /// Convert a scene-serialized field value into its wire form.
    ///
    /// Returns `None` for scene values that have no script-facing
    /// representation (currently only entity references).
    pub fn from_scene_value(value: &engine_serialize::Value) -> Option<Self> {
        use engine_serialize::Value as SceneValue;
        Some(match value {
            SceneValue::Bool(value) => Self::Bool(*value),
            SceneValue::Int(value) => Self::Int(*value),
            SceneValue::UInt(value) => Self::UInt(*value),
            SceneValue::Float32(value) => Self::Float(*value),
            SceneValue::Float64(value) => Self::Float(*value as f32),
            SceneValue::Str(value) => Self::Str(value.clone()),
            SceneValue::Enum(value) => Self::Enum(value.clone()),
            SceneValue::Asset(asset) => Self::Asset(asset.id.clone()),
            SceneValue::Vec3(value) => Self::Vec3(*value),
            SceneValue::Quat(value) => Self::Quat(*value),
            SceneValue::Color(value) => Self::Color(*value),
            SceneValue::List(items) => Self::List(
                items
                    .iter()
                    .map(Self::from_scene_value)
                    .collect::<Option<Vec<_>>>()?,
            ),
            SceneValue::Map(map) => Self::Map(
                map.iter()
                    .map(|(key, value)| Some((key.clone(), Self::from_scene_value(value)?)))
                    .collect::<Option<BTreeMap<_, _>>>()?,
            ),
            SceneValue::Entity(_) => return None,
        })
    }

    /// Convert this wire value back into the scene-serialized form consumed
    /// by the registered component deserialize hooks.
    pub fn to_scene_value(&self) -> engine_serialize::Value {
        use engine_serialize::Value as SceneValue;
        match self {
            Self::Bool(value) => SceneValue::Bool(*value),
            Self::Int(value) => SceneValue::Int(*value),
            Self::UInt(value) => SceneValue::UInt(*value),
            Self::Float(value) => SceneValue::Float32(*value),
            Self::Str(value) => SceneValue::Str(value.clone()),
            Self::Enum(value) => SceneValue::Enum(value.clone()),
            Self::Asset(asset) => SceneValue::Asset(engine_serialize::AssetId::new(asset)),
            Self::Vec3(value) => SceneValue::Vec3(*value),
            Self::Quat(value) => SceneValue::Quat(*value),
            Self::Color(value) => SceneValue::Color(*value),
            Self::List(items) => SceneValue::List(items.iter().map(Self::to_scene_value).collect()),
            Self::Map(map) => SceneValue::Map(
                map.iter()
                    .map(|(key, value)| (key.clone(), value.to_scene_value()))
                    .collect(),
            ),
        }
    }

    /// Validate one untrusted field value received from a script host.
    ///
    /// `depth` tracks list/map nesting so deeply nested payloads are rejected
    /// before they can exhaust the deserializer stack.
    pub fn validate(&self, depth: usize) -> Result<(), String> {
        if depth > MAX_COMPONENT_VALUE_DEPTH {
            return Err(format!(
                "component field values must not nest deeper than {MAX_COMPONENT_VALUE_DEPTH} levels"
            ));
        }
        match self {
            Self::Bool(_) | Self::Int(_) | Self::UInt(_) => Ok(()),
            Self::Float(value) => {
                if value.is_finite() {
                    Ok(())
                } else {
                    Err("component float fields must be finite".into())
                }
            }
            Self::Vec3(values) => {
                if values.iter().all(|value| value.is_finite()) {
                    Ok(())
                } else {
                    Err("component vector fields must contain only finite values".into())
                }
            }
            Self::Quat(values) | Self::Color(values) => {
                if values.iter().all(|value| value.is_finite()) {
                    Ok(())
                } else {
                    Err("component vector fields must contain only finite values".into())
                }
            }
            Self::Str(value) | Self::Enum(value) | Self::Asset(value) => {
                if value.len() <= MAX_COMPONENT_VALUE_STRING_BYTES
                    && !value.chars().any(char::is_control)
                {
                    Ok(())
                } else {
                    Err(format!(
                        "component string fields must contain at most {MAX_COMPONENT_VALUE_STRING_BYTES} bytes and no control characters"
                    ))
                }
            }
            Self::List(items) => {
                if items.len() > MAX_COMPONENT_LIST_ITEMS {
                    return Err(format!(
                        "component list fields must contain at most {MAX_COMPONENT_LIST_ITEMS} items"
                    ));
                }
                for item in items {
                    item.validate(depth + 1)?;
                }
                Ok(())
            }
            Self::Map(map) => {
                if map.len() > MAX_COMPONENT_FIELDS {
                    return Err(format!(
                        "component map fields must contain at most {MAX_COMPONENT_FIELDS} entries"
                    ));
                }
                for (key, value) in map {
                    validate_component_field_name(key)?;
                    value.validate(depth + 1)?;
                }
                Ok(())
            }
        }
    }
}

/// Validate a component field name received from untrusted script code.
pub fn validate_component_field_name(name: &str) -> Result<(), String> {
    let valid = !name.is_empty()
        && name.len() <= 128
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'));
    if valid {
        Ok(())
    } else {
        Err(format!(
            "invalid component field name {name:?}: expected 1 to 128 ASCII letters, digits, underscores, hyphens, or dots"
        ))
    }
}

/// Validate a component type key received from untrusted script code.
///
/// Type keys are stable registry identifiers such as `engine.audio_source`,
/// never paths, so they share the wire-safe identifier alphabet of entity ids.
pub fn validate_component_type_key(type_key: &str) -> Result<(), String> {
    let valid = !type_key.is_empty()
        && type_key.len() <= 128
        && type_key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'));
    if valid {
        Ok(())
    } else {
        Err(format!(
            "invalid component type key {type_key:?}: expected a registered key such as 'engine.audio_source' containing 1 to 128 ASCII letters, digits, underscores, hyphens, or dots"
        ))
    }
}

/// Validate the complete field payload of a `set_component` command.
pub fn validate_component_fields(
    fields: &BTreeMap<String, GameplayComponentValue>,
) -> Result<(), String> {
    if fields.len() > MAX_COMPONENT_FIELDS {
        return Err(format!(
            "set_component accepts at most {MAX_COMPONENT_FIELDS} fields per command"
        ));
    }
    for (name, value) in fields {
        validate_component_field_name(name)?;
        value.validate(0)?;
    }
    Ok(())
}

/// Active component query requested by a script through the gameplay bridge.
///
/// Queries travel as deferred gameplay commands exactly like physics queries:
/// the engine validates them, snapshots the requested component through its
/// registered scene serde hooks at the frame boundary, and delivers the
/// matching [`GameplayComponentQueryResult`] with the next frame's snapshot.
/// Scripts correlate requests and results through the caller-chosen
/// `query_id`; scripts never receive raw ECS handles.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GameplayComponentQuery {
    /// Script-chosen correlator echoed back with the result.
    pub query_id: u32,
    /// Persistent entity id to read from — never a raw ECS handle.
    pub entity_id: String,
    /// Registered component type key (for example `engine.audio_source`).
    pub component_type: String,
}

impl GameplayComponentQuery {
    /// Validate untrusted query data received from a script host.
    pub fn validate(&self) -> Result<(), String> {
        validate_entity_id(&self.entity_id)?;
        validate_component_type_key(&self.component_type)
    }
}

/// Outcome of a script component query, delivered with the next frame
/// snapshot following the frame that issued the query.
///
/// Results are frame-local: they appear in exactly one snapshot and are not
/// repeated. Every result echoes the issuing query's `query_id`, `entity_id`,
/// and `component_type` so scripts can match them without extra bookkeeping.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GameplayComponentQueryResult {
    /// The entity exists and carries the requested component; `fields` is the
    /// component's scene-serialized snapshot converted to wire values.
    Snapshot {
        /// Correlator from the issuing query.
        query_id: u32,
        /// Persistent entity id the snapshot was read from.
        entity_id: String,
        /// Component type key that was read.
        component_type: String,
        /// Field snapshot keyed by field name.
        fields: BTreeMap<String, GameplayComponentValue>,
    },
    /// The entity does not exist or does not carry the requested component.
    Missing {
        /// Correlator from the issuing query.
        query_id: u32,
        /// Persistent entity id that was probed.
        entity_id: String,
        /// Component type key that was probed.
        component_type: String,
    },
}

/// A validated component query paired with the entity that owns the script
/// instance that issued it.
#[derive(Clone, Debug, PartialEq)]
pub struct OwnedGameplayComponentQuery {
    pub entity_id: String,
    pub query: GameplayComponentQuery,
}
