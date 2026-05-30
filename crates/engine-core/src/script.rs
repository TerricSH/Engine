//! # Script subsystem integration
//!
//! Bridges [`engine_script`] with [`EngineRuntime`](crate::EngineRuntime):
//! converts between scene [`Value`](engine_serialize::Value) and
//! [`ScriptValue`](engine_script::ScriptValue), extracts script components
//! from scene entities, and drives per-frame lifecycle updates.
//!
//! This module is only compiled when the `subsystem-scripting-csharp` feature
//! is enabled.

use std::collections::BTreeMap;

use engine_scene::{EntityRecord, Scene};
use engine_script::{ScriptComponent, ScriptEngine, ScriptValue};
use engine_serialize::{AssetId, Value};

// ---------------------------------------------------------------------------
// Value conversion (engine_serialize::Value ↔ engine_script::ScriptValue)
// ---------------------------------------------------------------------------

/// Convert an [`engine_serialize::Value`] to a [`ScriptValue`] for crossing
/// the scene → script boundary.
pub fn serialize_value_to_script(v: &Value) -> ScriptValue {
    match v {
        Value::Bool(b) => ScriptValue::Bool(*b),
        Value::Int(i) => ScriptValue::Int(*i),
        Value::UInt(u) => ScriptValue::Int(*u as i64),
        Value::Float32(f) => ScriptValue::Float(*f as f64),
        Value::Float64(f) => ScriptValue::Float(*f),
        Value::Str(s) => ScriptValue::String(s.clone()),
        Value::Vec3(arr) => ScriptValue::Vec3(*arr),
        Value::List(items) => {
            ScriptValue::Array(items.iter().map(serialize_value_to_script).collect())
        }
        Value::Map(map) => ScriptValue::Map(
            map.iter()
                .map(|(k, v)| (k.clone(), serialize_value_to_script(v)))
                .collect(),
        ),
        Value::Asset(a) => ScriptValue::AssetIdWrapper(a.id.clone()),
        Value::Entity(e) => ScriptValue::EntityId(e.clone()),
        // No direct ScriptValue equivalents – map to String or Null
        Value::Quat(arr) => ScriptValue::Vec4(*arr),
        Value::Color(arr) => ScriptValue::Vec4(*arr),
        Value::Enum(s) => ScriptValue::String(s.clone()),
    }
}

/// Convert a [`ScriptValue`] back to an [`engine_serialize::Value`] for
/// scene save round-trips.
pub fn script_value_to_serialize(sv: &ScriptValue) -> Value {
    match sv {
        ScriptValue::Null => Value::Str("Null".to_string()),
        ScriptValue::Bool(b) => Value::Bool(*b),
        ScriptValue::Int(i) => Value::Int(*i),
        ScriptValue::Float(f) => Value::Float64(*f),
        ScriptValue::String(s) => Value::Str(s.clone()),
        ScriptValue::Vec3(arr) => Value::Vec3(*arr),
        ScriptValue::Vec4(arr) => Value::Quat(*arr),
        ScriptValue::EntityId(e) => Value::Entity(e.clone()),
        ScriptValue::AssetIdWrapper(id) => Value::Asset(AssetId::new(id.clone())),
        ScriptValue::Array(items) => {
            Value::List(items.iter().map(script_value_to_serialize).collect())
        }
        ScriptValue::Map(map) => Value::Map(
            map.iter()
                .map(|(k, v)| (k.clone(), script_value_to_serialize(v)))
                .collect(),
        ),
    }
}

// ---------------------------------------------------------------------------
// Scene script extraction
// ---------------------------------------------------------------------------

/// The component type ID used for script components in scene files.
pub const SCRIPT_COMPONENT_TYPE: &str = "engine.script";

/// Keys within a scene script component that carry structural metadata
/// (as opposed to user-defined script fields).
const RESERVED_SCRIPT_KEYS: &[&str] = &["assembly_id", "class_name"];

/// Try to extract a [`ScriptComponent`] from an entity's component map.
///
/// Returns `None` if the entity does not carry a component with type ID
/// [`SCRIPT_COMPONENT_TYPE`].
pub fn extract_script_component(entity: &EntityRecord) -> Option<ScriptComponent> {
    let comp = entity.components.get(SCRIPT_COMPONENT_TYPE)?;

    // Pull structured fields
    let assembly_id = match comp.fields.get("assembly_id") {
        Some(Value::Str(s)) => s.clone(),
        _ => return None,
    };
    let class_name = match comp.fields.get("class_name") {
        Some(Value::Str(s)) => s.clone(),
        _ => return None,
    };

    // Remaining fields (excluding reserved keys) become script-visible fields
    let mut fields = BTreeMap::new();
    for (key, val) in &comp.fields {
        if !RESERVED_SCRIPT_KEYS.contains(&key.as_str()) {
            fields.insert(key.clone(), serialize_value_to_script(val));
        }
    }

    Some(
        ScriptComponent::new(assembly_id, class_name)
            .with_fields(fields)
            .with_enabled(comp.enabled),
    )
}

/// Iterate every entity in a scene and collect all script components,
/// returning `(entity_id, ScriptComponent)` pairs.
pub fn collect_scene_scripts(scene: &Scene) -> Vec<(String, ScriptComponent)> {
    let mut result = Vec::new();
    for entity in &scene.entities {
        if let Some(sc) = extract_script_component(entity) {
            result.push((entity.persistent_id.clone(), sc));
        }
    }
    result
}

// ---------------------------------------------------------------------------
// Script engine lifecycle helpers
// ---------------------------------------------------------------------------

/// A human-readable summary of the script engine's current state for
/// the diagnostics panel.
pub fn script_engine_state_summary(engine: &ScriptEngine) -> String {
    let host_count = engine.host_count();
    let mut total_assemblies = 0usize;
    let mut total_instances = 0usize;
    for mgr in engine.managers() {
        total_assemblies += mgr.assembly_count();
        total_instances += mgr.instance_count();
    }
    format!("hosts={host_count} assemblies={total_assemblies} instances={total_instances}")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use engine_scene::{ComponentRecord, SceneSettings};
    use engine_serialize::SchemaVersion;

    fn make_entity(eid: &str, script_fields: BTreeMap<String, Value>) -> EntityRecord {
        let mut components = BTreeMap::new();
        components.insert(
            SCRIPT_COMPONENT_TYPE.to_string(),
            ComponentRecord {
                schema_version: SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: script_fields,
            },
        );
        EntityRecord {
            persistent_id: eid.to_string(),
            parent: None,
            name: Some(eid.to_string()),
            enabled: true,
            components,
        }
    }

    #[test]
    fn extract_valid_script_component() {
        let mut fields = BTreeMap::new();
        fields.insert("assembly_id".into(), Value::Str("asm-001".into()));
        fields.insert("class_name".into(), Value::Str("MyBehaviour".into()));
        fields.insert("speed".into(), Value::Float64(10.0));

        let entity = make_entity("ent-1", fields);
        let sc = extract_script_component(&entity).unwrap();
        assert_eq!(sc.assembly_id, "asm-001");
        assert_eq!(sc.class_name, "MyBehaviour");
        assert_eq!(sc.fields.get("speed"), Some(&ScriptValue::Float(10.0)));
    }

    #[test]
    fn extract_script_missing_assembly_id() {
        let mut fields = BTreeMap::new();
        fields.insert("class_name".into(), Value::Str("B".into()));
        let entity = make_entity("ent-2", fields);
        assert!(extract_script_component(&entity).is_none());
    }

    #[test]
    fn extract_script_no_component() {
        let entity = EntityRecord {
            persistent_id: "ent-3".to_string(),
            parent: None,
            name: None,
            enabled: true,
            components: BTreeMap::new(),
        };
        assert!(extract_script_component(&entity).is_none());
    }

    #[test]
    fn extract_script_reserved_keys_excluded_from_fields() {
        let mut fields = BTreeMap::new();
        fields.insert("assembly_id".into(), Value::Str("asm".into()));
        fields.insert("class_name".into(), Value::Str("T".into()));
        fields.insert("custom".into(), Value::Bool(true));
        let entity = make_entity("ent-4", fields);
        let sc = extract_script_component(&entity).unwrap();
        assert!(sc.fields.contains_key("custom"));
        assert!(!sc.fields.contains_key("assembly_id"));
        assert!(!sc.fields.contains_key("class_name"));
    }

    #[test]
    fn collect_scene_scripts_empty() {
        let scene = Scene {
            schema_version: SchemaVersion::new(0, 1, 0),
            engine_version: "0.1.0".to_string(),
            scene_id: "test".to_string(),
            name: "test".to_string(),
            entities: vec![],
            scene_settings: SceneSettings::default(),
            dependencies: vec![],
            diagnostics_policy: engine_scene::DiagnosticsPolicy::Strict,
        };
        let scripts = collect_scene_scripts(&scene);
        assert!(scripts.is_empty());
    }

    #[test]
    fn collect_scene_scripts_multiple() {
        let mut fields1 = BTreeMap::new();
        fields1.insert("assembly_id".into(), Value::Str("a".into()));
        fields1.insert("class_name".into(), Value::Str("A".into()));

        let mut fields2 = BTreeMap::new();
        fields2.insert("assembly_id".into(), Value::Str("b".into()));
        fields2.insert("class_name".into(), Value::Str("B".into()));

        let scene = Scene {
            schema_version: SchemaVersion::new(0, 1, 0),
            engine_version: "0.1.0".to_string(),
            scene_id: "t".to_string(),
            name: "t".to_string(),
            entities: vec![make_entity("e1", fields1), make_entity("e2", fields2)],
            scene_settings: SceneSettings::default(),
            dependencies: vec![],
            diagnostics_policy: engine_scene::DiagnosticsPolicy::Strict,
        };
        let scripts = collect_scene_scripts(&scene);
        assert_eq!(scripts.len(), 2);
    }

    // ── Value conversion ────────────────────────────────────────────────

    #[test]
    fn serialize_value_to_script_bool() {
        assert_eq!(
            serialize_value_to_script(&Value::Bool(true)),
            ScriptValue::Bool(true)
        );
    }

    #[test]
    fn serialize_value_to_script_int() {
        assert_eq!(
            serialize_value_to_script(&Value::Int(42)),
            ScriptValue::Int(42)
        );
    }

    #[test]
    fn serialize_value_to_script_float() {
        assert_eq!(
            serialize_value_to_script(&Value::Float64(3.14)),
            ScriptValue::Float(3.14)
        );
    }

    #[test]
    fn serialize_value_to_script_string() {
        assert_eq!(
            serialize_value_to_script(&Value::Str("hi".into())),
            ScriptValue::String("hi".into())
        );
    }

    #[test]
    fn serialize_value_to_script_vec3() {
        assert_eq!(
            serialize_value_to_script(&Value::Vec3([1.0, 0.0, 0.0])),
            ScriptValue::Vec3([1.0, 0.0, 0.0])
        );
    }

    #[test]
    fn serialize_value_to_script_list() {
        let v = Value::List(vec![Value::Int(1), Value::Int(2)]);
        let sv = serialize_value_to_script(&v);
        assert_eq!(
            sv,
            ScriptValue::Array(vec![ScriptValue::Int(1), ScriptValue::Int(2)])
        );
    }

    #[test]
    fn script_to_serialize_roundtrip() {
        let cases = vec![
            ScriptValue::Bool(true),
            ScriptValue::Int(42),
            ScriptValue::Float(3.14),
            ScriptValue::String("hello".into()),
            ScriptValue::Vec3([1.0, 0.0, 0.0]),
            ScriptValue::EntityId("ent-001".into()),
            ScriptValue::AssetIdWrapper("mesh-cube".into()),
            ScriptValue::Array(vec![ScriptValue::Int(1)]),
        ];
        for sv in cases {
            let v = script_value_to_serialize(&sv);
            let back = serialize_value_to_script(&v);
            assert_eq!(sv, back, "roundtrip failed for {sv:?}");
        }
    }

    #[test]
    fn script_engine_state_summary_empty() {
        let engine = ScriptEngine::new();
        let summary = script_engine_state_summary(&engine);
        assert!(summary.contains("hosts=0"));
        assert!(summary.contains("assemblies=0"));
        assert!(summary.contains("instances=0"));
    }
}
