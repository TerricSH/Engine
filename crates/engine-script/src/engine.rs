//! Script engine — the top-level coordinator.
//!
//! [`ScriptEngine`] manages registered [`ScriptHost`] backends and dispatches
//! script operations (loading, instantiation, update) to the correct host.

use crate::host::{ScriptError, ScriptHandle, ScriptHost, ScriptInstance};

/// The main script system that manages hosts and dispatches script operations.
pub struct ScriptEngine {
    /// Registered backends, stored as `(name, boxed_host)` pairs.
    hosts: Vec<(String, Box<dyn ScriptHost>)>,
}

impl ScriptEngine {
    /// Create a new, empty script engine with no hosts registered.
    pub fn new() -> Self {
        Self {
            hosts: Vec::new(),
        }
    }

    /// Register a script backend.
    ///
    /// The host's [`name`](ScriptHost::name) is used as the lookup key for
    /// subsequent [`load_script`](Self::load_script) calls.
    pub fn register_host(&mut self, host: Box<dyn ScriptHost>) {
        let name = host.name().to_string();
        self.hosts.push((name, host));
    }

    /// Load a script assembly through the named host.
    ///
    /// * `id` — a caller-chosen identifier for this assembly.
    /// * `host_name` — must match the [`name`](ScriptHost::name) of a
    ///   previously registered host.
    /// * `data` — the raw assembly bytes.
    ///
    /// Returns a [`ScriptHandle`] that can be passed to
    /// [`instantiate`](Self::instantiate).
    pub fn load_script(
        &mut self,
        id: &str,
        host_name: &str,
        data: &[u8],
    ) -> Result<ScriptHandle, ScriptError> {
        let host = self
            .hosts
            .iter_mut()
            .find(|(name, _)| name == host_name)
            .map(|(_, host)| host)
            .ok_or_else(|| {
                ScriptError::HostError(format!(
                    "No script host registered with the name '{host_name}'"
                ))
            })?;

        let mut handle = host.load_assembly(id, data)?;
        handle.host_name = host_name.to_string();
        Ok(handle)
    }

    /// Create a new script instance from a previously loaded assembly.
    ///
    /// The [`ScriptHandle`] must have been returned by
    /// [`load_script`](Self::load_script) and the originating host must still
    /// be registered.
    pub fn instantiate(
        &mut self,
        handle: &ScriptHandle,
    ) -> Result<Box<dyn ScriptInstance>, ScriptError> {
        let host = self
            .hosts
            .iter_mut()
            .find(|(name, _)| name == &handle.host_name)
            .map(|(_, host)| host)
            .ok_or_else(|| {
                ScriptError::HostError(format!(
                    "The host '{name}' that created this handle is no longer registered",
                    name = handle.host_name,
                ))
            })?;

        host.instantiate(handle)
    }

    /// Tick all running script instances.
    ///
    /// The actual per-instance update dispatch will be implemented when the
    /// .NET CoreCLR/NativeAOT hosting backend is added. Currently this is a
    /// no-op that emits a trace log.
    pub fn update(&mut self, _dt: f32) {
        tracing::trace!("ScriptEngine::update({_dt}) — no instances tracked at engine level");
    }

    /// Return the number of registered script hosts.
    pub fn host_count(&self) -> usize {
        self.hosts.len()
    }
}

impl Default for ScriptEngine {
    fn default() -> Self {
        Self::new()
    }
}
