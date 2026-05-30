//! Script host traits and types.
//!
//! Defines [`ScriptError`], [`ScriptHandle`], [`ScriptInstance`],
//! [`ScriptHost`], and the [`NullScriptHost`] default implementation.
//! Also provides [`MockHost`] and [`MockScriptInstance`] for testing.

use std::collections::HashMap;

use thiserror::Error;

use crate::value::ScriptValue;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors that can occur during script operations.
#[derive(Error, Debug, Clone)]
pub enum ScriptError {
    /// The script assembly could not be loaded.
    #[error("Failed to load assembly: {0}")]
    LoadFailed(String),

    /// The requested function does not exist in the script.
    #[error("Function not found: {0}")]
    FunctionNotFound(String),

    /// A runtime error occurred during script execution.
    #[error("Script execution error: {0}")]
    ExecutionError(String),

    /// An error in the host infrastructure (e.g. missing runtime).
    #[error("Host infrastructure error: {0}")]
    HostError(String),

    /// The requested feature is not supported by this host.
    #[error("Unsupported feature: {0}")]
    UnsupportedFeature(String),
}

// ---------------------------------------------------------------------------
// Script handle
// ---------------------------------------------------------------------------

/// A lightweight handle to a loaded script assembly.
///
/// Created by [`ScriptHost::load_assembly`] and enriched with the owning host
/// name by [`ScriptEngine::load_script`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptHandle {
    /// Opaque identifier assigned by the host backend.
    id: String,
    /// Name of the [`ScriptHost`] that owns this assembly (set by the engine).
    pub(crate) host_name: String,
}

impl ScriptHandle {
    /// Create a new handle with the given assembly identifier.
    ///
    /// This is primarily used by [`ScriptHost`] implementations during
    /// [`load_assembly`](ScriptHost::load_assembly).
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            host_name: String::new(),
        }
    }

    /// The unique identifier for this assembly within its host.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Returns `true` if the underlying assembly is still loaded.
    ///
    /// Currently always returns `true`; validity tracking will be enhanced
    /// when the .NET CoreCLR/NativeAOT hosting backend is implemented.
    pub fn is_valid(&self) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------
// Script instance trait
// ---------------------------------------------------------------------------

/// An instance of a script with its own state.
///
/// Each instance is created by a [`ScriptHost`] and can be used to call
/// functions and read/write fields on the underlying script object.
pub trait ScriptInstance {
    /// Call a function on this script instance.
    ///
    /// * `function` — name of the function to call.
    /// * `args` — slice of arguments to pass.
    ///
    /// Returns the return value, or [`ScriptError::FunctionNotFound`] if the
    /// function does not exist.
    fn call(&mut self, function: &str, args: &[ScriptValue]) -> Result<ScriptValue, ScriptError>;

    /// Set a named field on this script instance.
    fn set_field(&mut self, name: &str, value: ScriptValue) -> Result<(), ScriptError>;

    /// Read a named field from this script instance.
    ///
    /// Returns `None` if the field does not exist.
    fn get_field(&self, name: &str) -> Option<ScriptValue>;
}

// ---------------------------------------------------------------------------
// Script host trait
// ---------------------------------------------------------------------------

/// Abstract interface for a script backend (e.g. .NET CLR, Lua, WASM).
///
/// Each host manages its own assembly storage, type resolution, and instance
/// lifetime. The [`ScriptEngine`] dispatches calls to the correct host by
/// name.
pub trait ScriptHost {
    /// A human-readable name for this host backend (e.g. `"dotnet"`,
    /// `"lua"`).
    fn name(&self) -> &str;

    /// Load a script assembly from raw bytes.
    ///
    /// * `id` — a caller-chosen identifier for the assembly.
    /// * `assembly_data` — the raw assembly payload (e.g. a .NET PE file).
    ///
    /// Returns a [`ScriptHandle`] that can be used to
    /// [`instantiate`](ScriptHost::instantiate) and
    /// [`unload`](ScriptHost::unload) the assembly.
    fn load_assembly(
        &mut self,
        id: &str,
        assembly_data: &[u8],
    ) -> Result<ScriptHandle, ScriptError>;

    /// Create a new instance of a previously loaded assembly.
    fn instantiate(
        &mut self,
        handle: &ScriptHandle,
    ) -> Result<Box<dyn ScriptInstance>, ScriptError>;

    /// Unload an assembly and release its resources.
    ///
    /// All instances created from this assembly should be considered
    /// invalidated.
    fn unload(&mut self, handle: &ScriptHandle) -> Result<(), ScriptError>;
}

// ---------------------------------------------------------------------------
// Null script host
// ---------------------------------------------------------------------------

/// A minimal script host that tracks assemblies but does not execute scripts.
///
/// `load_assembly` and `unload` succeed (tracking the assembly by its id),
/// while `instantiate` returns [`ScriptError::UnsupportedFeature`]. This
/// allows the engine to function gracefully without a scripting runtime.
pub struct NullScriptHost {
    /// Tracked assembly handles keyed by id.
    assemblies: HashMap<String, ScriptHandle>,
}

impl NullScriptHost {
    /// Create a new `NullScriptHost`.
    pub fn new() -> Self {
        Self {
            assemblies: HashMap::new(),
        }
    }
}

impl Default for NullScriptHost {
    fn default() -> Self {
        Self::new()
    }
}

impl ScriptHost for NullScriptHost {
    fn name(&self) -> &str {
        "null"
    }

    fn load_assembly(
        &mut self,
        id: &str,
        _assembly_data: &[u8],
    ) -> Result<ScriptHandle, ScriptError> {
        let handle = ScriptHandle::new(id);
        self.assemblies.insert(id.to_string(), handle.clone());
        Ok(handle)
    }

    fn instantiate(
        &mut self,
        _handle: &ScriptHandle,
    ) -> Result<Box<dyn ScriptInstance>, ScriptError> {
        Err(ScriptError::UnsupportedFeature(
            "NullScriptHost does not support instantiating scripts".into(),
        ))
    }

    fn unload(&mut self, handle: &ScriptHandle) -> Result<(), ScriptError> {
        self.assemblies.remove(handle.id());
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Mock script instance
// ---------------------------------------------------------------------------

/// A mock script instance for testing lifecycle dispatch without a runtime.
///
/// Responds to standard lifecycle callbacks (`OnCreate`, `OnStart`, `OnUpdate`,
/// `OnDestroy`) with `Ok(Null)` and supports field get/set through an internal
/// map.
pub struct MockScriptInstance {
    fields: HashMap<String, ScriptValue>,
    /// Track which lifecycle methods have been called (for test assertions).
    pub called: std::cell::RefCell<Vec<String>>,
}

impl MockScriptInstance {
    /// Create a new mock instance.
    pub fn new() -> Self {
        Self {
            fields: HashMap::new(),
            called: std::cell::RefCell::new(Vec::new()),
        }
    }
}

impl Default for MockScriptInstance {
    fn default() -> Self {
        Self::new()
    }
}

impl ScriptInstance for MockScriptInstance {
    fn call(&mut self, function: &str, _args: &[ScriptValue]) -> Result<ScriptValue, ScriptError> {
        self.called.borrow_mut().push(function.to_string());
        match function {
            "OnCreate" | "OnStart" | "OnUpdate" | "OnDestroy" => Ok(ScriptValue::Null),
            other => Err(ScriptError::FunctionNotFound(other.to_string())),
        }
    }

    fn set_field(&mut self, name: &str, value: ScriptValue) -> Result<(), ScriptError> {
        self.fields.insert(name.to_string(), value);
        Ok(())
    }

    fn get_field(&self, name: &str) -> Option<ScriptValue> {
        self.fields.get(name).cloned()
    }
}

// ---------------------------------------------------------------------------
// Mock script host
// ---------------------------------------------------------------------------

/// A mock script host for testing lifecycle dispatch without a .NET runtime.
///
/// Stores assembly data and creates [`MockScriptInstance`] objects on
/// instantiate. Useful for integration tests of [`ScriptManager`] and
/// [`ScriptEngine`].
pub struct MockHost {
    assemblies: HashMap<String, Vec<u8>>,
}

impl MockHost {
    /// Create a new mock host.
    pub fn new() -> Self {
        Self {
            assemblies: HashMap::new(),
        }
    }
}

impl Default for MockHost {
    fn default() -> Self {
        Self::new()
    }
}

impl ScriptHost for MockHost {
    fn name(&self) -> &str {
        "mock"
    }

    fn load_assembly(
        &mut self,
        id: &str,
        assembly_data: &[u8],
    ) -> Result<ScriptHandle, ScriptError> {
        self.assemblies
            .insert(id.to_string(), assembly_data.to_vec());
        Ok(ScriptHandle::new(id))
    }

    fn instantiate(
        &mut self,
        _handle: &ScriptHandle,
    ) -> Result<Box<dyn ScriptInstance>, ScriptError> {
        Ok(Box::new(MockScriptInstance::new()))
    }

    fn unload(&mut self, handle: &ScriptHandle) -> Result<(), ScriptError> {
        self.assemblies.remove(handle.id());
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ── ScriptError tests ────────────────────────────────────────────────

    #[test]
    fn script_error_load_failed_display() {
        let err = ScriptError::LoadFailed("assembly not found".to_string());
        assert_eq!(
            err.to_string(),
            "Failed to load assembly: assembly not found"
        );
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
        assert_eq!(
            err.to_string(),
            "Host infrastructure error: runtime unavailable"
        );
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

    // ── MockHost tests ───────────────────────────────────────────────────

    #[test]
    fn mock_host_name() {
        let host = MockHost::new();
        assert_eq!(host.name(), "mock");
    }

    #[test]
    fn mock_host_load_assembly_succeeds() {
        let mut host = MockHost::new();
        let handle = host.load_assembly("asm", b"data").unwrap();
        assert_eq!(handle.id(), "asm");
    }

    #[test]
    fn mock_host_instantiate_succeeds() {
        let mut host = MockHost::new();
        let handle = host.load_assembly("asm", b"data").unwrap();
        let instance = host.instantiate(&handle);
        assert!(instance.is_ok());
    }

    #[test]
    fn mock_host_unload_succeeds() {
        let mut host = MockHost::new();
        let handle = host.load_assembly("asm", b"data").unwrap();
        assert!(host.unload(&handle).is_ok());
    }

    #[test]
    fn mock_instance_call_lifecycle_succeeds() {
        let mut inst = MockScriptInstance::new();
        assert!(inst.call("OnCreate", &[]).is_ok());
        assert!(inst.call("OnStart", &[]).is_ok());
        assert!(inst.call("OnUpdate", &[]).is_ok());
        assert!(inst.call("OnDestroy", &[]).is_ok());
    }

    #[test]
    fn mock_instance_call_unknown_fails() {
        let mut inst = MockScriptInstance::new();
        let result = inst.call("UnknownMethod", &[]);
        assert!(result.is_err());
        match result {
            Err(ScriptError::FunctionNotFound(_)) => {}
            _ => panic!("Expected FunctionNotFound"),
        }
    }

    #[test]
    fn mock_instance_set_and_get_field() {
        let mut inst = MockScriptInstance::new();
        inst.set_field("speed", ScriptValue::Float(42.0)).unwrap();
        assert_eq!(inst.get_field("speed"), Some(ScriptValue::Float(42.0)));
    }

    #[test]
    fn mock_instance_get_missing_field() {
        let inst = MockScriptInstance::new();
        assert_eq!(inst.get_field("missing"), None);
    }

    #[test]
    fn mock_instance_tracks_calls() {
        let mut inst = MockScriptInstance::new();
        inst.call("OnCreate", &[]).unwrap();
        inst.call("OnStart", &[]).unwrap();
        let called = inst.called.borrow();
        assert_eq!(called.len(), 2);
        assert_eq!(called[0], "OnCreate");
        assert_eq!(called[1], "OnStart");
    }
}
