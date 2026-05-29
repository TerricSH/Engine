//! # Engine Script
//!
//! Abstract script hosting API for the engine.
//!
//! This crate defines the types and traits for loading, instantiating, and
//! interacting with scripts (currently targeting .NET via CoreCLR/NativeAOT).
//!
//! # Modules
//!
//! * [`value`] — [`ScriptValue`] enum for cross-boundary data.
//! * [`host`] — [`ScriptError`], [`ScriptHandle`], [`ScriptInstance`],
//!   [`ScriptHost`] trait, plus [`NullScriptHost`] and [`MockHost`].
//! * [`engine`] — [`ScriptEngine`] coordinator.
//! * [`lifecycle`] — Standard lifecycle callback name constants.
//! * [`component`] — [`ScriptComponent`], [`ScriptInstanceState`],
//!   [`ScriptManager`] for ECS scene integration.
//! * [`protocol`] — JSON-line wire protocol for process-based hosts.
//! * [`process_host`] — [`ProcessHost`] stub for CoreCLR child-process
//!   communication.
//! * [`profile`] — [`PlatformProfile`] enum, [`PlatformConstraints`], and
//!   [`is_feature_available`] for mobile / AOT compatibility checks.
//! * [`api_compat`] — [`ApiCompatRange`] and ScriptAPI feature subset constants
//!   ([`MOBILE_SAFE_FEATURES`], [`DESKTOP_ONLY_FEATURES`]).
//!
//! # Safety
//!
//! This crate is **excepted** from `forbid(unsafe_code)` because .NET hosting
//! requires unsafe FFI. Every `unsafe` block **must** carry a `// SAFETY:`
//! comment explaining why the invariants are upheld.

mod api_compat;
mod component;
mod engine;
pub mod extension;
mod host;
pub mod ilruntime_host;
mod lifecycle;
pub mod mobile_subset;
mod process_host;
mod profile;
mod protocol;
mod value;

// Re-export lifecycle constants at the crate root for convenience.
pub use lifecycle::lifecycle::{ON_CREATE, ON_START, ON_UPDATE, ON_DESTROY};

// Core types.
pub use value::ScriptValue;
pub use host::{
    MockHost, MockScriptInstance, NullScriptHost, ScriptError, ScriptHandle, ScriptInstance,
    ScriptHost,
};
pub use engine::ScriptEngine;
pub use component::{ScriptComponent, ScriptInstanceState, ScriptManager};
pub use protocol::ScriptMessage;
pub use ilruntime_host::{ILRuntimeHost, ILRuntimeInstance};
pub use process_host::{ProcessHost, ProcessScriptInstance};

// Script API extension surface.
pub use extension::{
    ProvidedMethod, ProvidedType, ScriptApiExtensionMeta, ScriptApiExtensionRegistry,
    ScriptApiProvider,
};

// Mobile-safe API subset (data-only, no scripting backend dependency).
pub use mobile_subset::{mobile_subset_v0, ScriptApiSubset, UnsupportedPattern};

// Platform profile and API compatibility.
pub use api_compat::{ApiCompatRange, MOBILE_SAFE_FEATURES, DESKTOP_ONLY_FEATURES};
pub use profile::{is_feature_available, PlatformConstraints, PlatformProfile};

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
        assert_eq!(
            ScriptValue::Float(std::f64::consts::PI),
            ScriptValue::Float(std::f64::consts::PI)
        );
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

    // ── ScriptValue serialization ────────────────────────────────────────

    #[test]
    fn script_value_serde_roundtrip() {
        let cases = vec![
            ScriptValue::Null,
            ScriptValue::Bool(true),
            ScriptValue::Int(42),
            ScriptValue::Float(3.14),
            ScriptValue::String("hello".into()),
            ScriptValue::Vec3([1.0, 2.0, 3.0]),
            ScriptValue::Vec4([1.0, 2.0, 3.0, 4.0]),
            ScriptValue::EntityId("ent-001".into()),
            ScriptValue::AssetIdWrapper("mesh-cube".into()),
            ScriptValue::Array(vec![ScriptValue::Int(1), ScriptValue::Int(2)]),
        ];
        for val in cases {
            let json = serde_json::to_string(&val).unwrap();
            let back: ScriptValue = serde_json::from_str(&json).unwrap();
            assert_eq!(val, back, "round-trip failed for {val:?} -> {json}");
        }
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
    fn null_script_host_load_assembly_now_tracks() {
        let mut host = NullScriptHost::new();
        let result = host.load_assembly("test", b"data");
        assert!(result.is_ok());
        let handle = result.unwrap();
        assert_eq!(handle.id(), "test");
    }

    #[test]
    fn null_script_host_instantiate_fails() {
        let mut host = NullScriptHost::new();
        let handle = ScriptHandle::new("test");
        let result = host.instantiate(&handle);
        assert!(result.is_err());
        match result {
            Err(ScriptError::UnsupportedFeature(msg)) => {
                assert!(msg.contains("NullScriptHost"));
            }
            _ => panic!("Expected UnsupportedFeature error"),
        }
    }

    #[test]
    fn null_script_host_unload_now_succeeds() {
        let mut host = NullScriptHost::new();
        let handle = host.load_assembly("test", b"data").unwrap();
        let result = host.unload(&handle);
        assert!(result.is_ok());
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
        let _ = engine.update(0.016); // Should not panic
        let _ = engine.update(1.0);
    }

    #[test]
    fn script_engine_register_multiple_hosts() {
        let mut engine = ScriptEngine::new();
        engine.register_host(Box::new(NullScriptHost::new()));
        engine.register_host(Box::new(NullScriptHost::new()));
        assert_eq!(engine.host_count(), 2);
    }
}
