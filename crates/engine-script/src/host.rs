//! Script host traits and types.
//!
//! Defines [`ScriptError`], [`ScriptHandle`], [`ScriptInstance`],
//! [`ScriptHost`], and the [`NullScriptHost`] default implementation.

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

/// A minimal script host that returns [`ScriptError::UnsupportedFeature`] for
/// all operations.
///
/// This is the default host used when no .NET runtime is available. It allows
/// the engine to function gracefully without scripting support.
pub struct NullScriptHost;

impl NullScriptHost {
    /// Create a new `NullScriptHost`.
    pub fn new() -> Self {
        Self
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
        _id: &str,
        _assembly_data: &[u8],
    ) -> Result<ScriptHandle, ScriptError> {
        Err(ScriptError::UnsupportedFeature(
            "NullScriptHost does not support loading assemblies".into(),
        ))
    }

    fn instantiate(
        &mut self,
        _handle: &ScriptHandle,
    ) -> Result<Box<dyn ScriptInstance>, ScriptError> {
        Err(ScriptError::UnsupportedFeature(
            "NullScriptHost does not support instantiating scripts".into(),
        ))
    }

    fn unload(&mut self, _handle: &ScriptHandle) -> Result<(), ScriptError> {
        Err(ScriptError::UnsupportedFeature(
            "NullScriptHost does not support unloading assemblies".into(),
        ))
    }
}
