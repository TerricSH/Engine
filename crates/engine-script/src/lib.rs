//! # Engine Script
//!
//! Abstract script hosting API for the engine.
//!
//! This crate defines the types and traits for loading, instantiating, and
//! interacting with scripts (currently targeting .NET via CoreCLR/NativeAOT).
//!
//! # Safety
//!
//! This crate is **excepted** from `forbid(unsafe_code)` because .NET hosting
//! requires unsafe FFI. Every `unsafe` block **must** carry a `// SAFETY:`
//! comment explaining why the invariants are upheld.

mod engine;
mod host;
mod value;

pub use value::ScriptValue;
pub use host::{ScriptError, ScriptHandle, ScriptInstance, ScriptHost, NullScriptHost};
pub use engine::ScriptEngine;

#[cfg(test)]
mod tests {
    use super::*;

    // ── ScriptError tests ────────────────────────────────────────────────

    #[test]
    fn script_error_load_failed_display() {
        let err = ScriptError::LoadFailed("assembly not found".to_string());
        assert_eq!(err.to_string(), "Failed to load assembly: assembly not found");
    }

    #[test]
    fn script_error_function_not_found_display() {
        let err = ScriptError::FunctionNotFound("on_update".to_string());
        assert_eq!(err.to_string(), "Function not found: on_update");
    }

    #[test]
    fn script_error_execution_error_display() {
        let err = ScriptError::ExecutionError("division by zero".to_string());
        assert_eq!(err.to_string(), "Script execution error: division by zero");
    }

    #[test]
    fn script_error_host_error_display() {
        let err = ScriptError::HostError("runtime unavailable".to_string());
        assert_eq!(err.to_string(), "Host infrastructure error: runtime unavailable");
    }

    #[test]
    fn script_error_unsupported_feature_display() {
        let err = ScriptError::UnsupportedFeature("hot reload".to_string());
        assert_eq!(err.to_string(), "Unsupported feature: hot reload");
    }

    #[test]
    fn script_error_debug() {
        let err = ScriptError::LoadFailed("err".to_string());
        let debug = format!("{:?}", err);
        assert!(debug.contains("LoadFailed"));
    }

    // ── ScriptValue tests ────────────────────────────────────────────────

    #[test]
    fn script_value_null() {
        assert_eq!(ScriptValue::Null, ScriptValue::Null);
    }

    #[test]
    fn script_value_bool() {
        assert_eq!(ScriptValue::Bool(true), ScriptValue::Bool(true));
        assert_ne!(ScriptValue::Bool(true), ScriptValue::Bool(false));
    }

    #[test]
    fn script_value_int() {
        assert_eq!(ScriptValue::Int(42), ScriptValue::Int(42));
    }

    #[test]
    fn script_value_float() {
        assert_eq!(ScriptValue::Float(3.14), ScriptValue::Float(3.14));
    }

    #[test]
    fn script_value_string() {
        let s = ScriptValue::String("hello".to_string());
        assert_eq!(s, ScriptValue::String("hello".to_string()));
        assert_ne!(s, ScriptValue::String("world".to_string()));
    }

    #[test]
    fn script_value_vec3() {
        assert_eq!(
            ScriptValue::Vec3([1.0, 2.0, 3.0]),
            ScriptValue::Vec3([1.0, 2.0, 3.0])
        );
    }

    #[test]
    fn script_value_vec4() {
        assert_eq!(
            ScriptValue::Vec4([1.0, 2.0, 3.0, 4.0]),
            ScriptValue::Vec4([1.0, 2.0, 3.0, 4.0])
        );
    }

    #[test]
    fn script_value_entity_id() {
        let e = ScriptValue::EntityId("ent-001".to_string());
        assert_eq!(e, ScriptValue::EntityId("ent-001".to_string()));
    }

    #[test]
    fn script_value_asset_id_wrapper() {
        let a = ScriptValue::AssetIdWrapper("mesh-cube".to_string());
        assert_eq!(a, ScriptValue::AssetIdWrapper("mesh-cube".to_string()));
    }

    #[test]
    fn script_value_array() {
        let arr = ScriptValue::Array(vec![ScriptValue::Int(1), ScriptValue::Int(2)]);
        assert_eq!(
            arr,
            ScriptValue::Array(vec![ScriptValue::Int(1), ScriptValue::Int(2)])
        );
    }

    #[test]
    fn script_value_map() {
        use std::collections::BTreeMap;
        let mut map = BTreeMap::new();
        map.insert("key".to_string(), ScriptValue::Bool(true));
        let val = ScriptValue::Map(map);
        assert_eq!(format!("{:?}", val), "Map({\"key\": Bool(true)})");
    }

    // ── ScriptHandle tests ───────────────────────────────────────────────

    #[test]
    fn script_handle_new_creates_handle() {
        let handle = ScriptHandle::new("assembly-001");
        assert_eq!(handle.id(), "assembly-001");
    }

    #[test]
    fn script_handle_is_valid() {
        let handle = ScriptHandle::new("test");
        assert!(handle.is_valid());
    }

    #[test]
    fn script_handle_id_returns_id() {
        let handle = ScriptHandle::new("my-script");
        assert_eq!(handle.id(), "my-script");
    }

    #[test]
    fn script_handle_equality() {
        let a = ScriptHandle::new("same");
        let b = ScriptHandle::new("same");
        let c = ScriptHandle::new("other");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn script_handle_debug() {
        let handle = ScriptHandle::new("my-handle");
        let debug = format!("{:?}", handle);
        assert!(debug.contains("ScriptHandle"));
    }

    // ── NullScriptHost tests ─────────────────────────────────────────────

    #[test]
    fn null_script_host_name() {
        let host = NullScriptHost::new();
        assert_eq!(host.name(), "null");
    }

    #[test]
    fn null_script_host_load_assembly_fails() {
        let mut host = NullScriptHost::new();
        let result = host.load_assembly("test", b"data");
        assert!(result.is_err());
        match result {
            Err(ScriptError::UnsupportedFeature(msg)) => {
                assert!(msg.contains("NullScriptHost"));
            }
            _ => panic!("Expected UnsupportedFeature error"),
        }
    }

    #[test]
    fn null_script_host_instantiate_fails() {
        let mut host = NullScriptHost::new();
        let handle = ScriptHandle::new("test");
        let result = host.instantiate(&handle);
        assert!(result.is_err());
    }

    #[test]
    fn null_script_host_unload_fails() {
        let mut host = NullScriptHost::new();
        let handle = ScriptHandle::new("test");
        let result = host.unload(&handle);
        assert!(result.is_err());
    }

    #[test]
    fn null_script_host_new() {
        let host = NullScriptHost::new();
        assert_eq!(host.name(), "null");
    }

    // ── ScriptEngine tests ───────────────────────────────────────────────

    #[test]
    fn script_engine_new_is_empty() {
        let engine = ScriptEngine::new();
        assert_eq!(engine.host_count(), 0);
    }

    #[test]
    fn script_engine_default_is_empty() {
        let engine = ScriptEngine::default();
        assert_eq!(engine.host_count(), 0);
    }

    #[test]
    fn script_engine_register_host_increases_count() {
        let mut engine = ScriptEngine::new();
        engine.register_host(Box::new(NullScriptHost::new()));
        assert_eq!(engine.host_count(), 1);
    }

    #[test]
    fn script_engine_load_script_unknown_host() {
        let mut engine = ScriptEngine::new();
        let result = engine.load_script("test", "unknown_host", b"data");
        assert!(result.is_err());
    }

    #[test]
    fn script_engine_update_no_panic() {
        let mut engine = ScriptEngine::new();
        engine.update(0.016); // Should not panic
        engine.update(1.0);
    }

    #[test]
    fn script_engine_register_multiple_hosts() {
        let mut engine = ScriptEngine::new();
        engine.register_host(Box::new(NullScriptHost::new()));
        engine.register_host(Box::new(NullScriptHost::new()));
        assert_eq!(engine.host_count(), 2);
    }
}
