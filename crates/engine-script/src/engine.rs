//! Script engine — the top-level coordinator.
//!
//! [`ScriptEngine`] manages registered [`ScriptHost`] backends and dispatches
//! script operations (loading, instantiation, update) to the correct host.
//! It also integrates with [`ScriptManager`] for component-based lifecycle
//! management (attach/detach scripts to/from ECS entities).

use engine_serialize::Diagnostic;

use crate::component::{ScriptComponent, ScriptManager};
use crate::host::{ScriptError, ScriptHandle, ScriptHost, ScriptInstance};

/// The main script system that manages hosts and dispatches script operations.
///
/// # Architecture
///
/// Each registered host gets its own [`ScriptManager`] that tracks loaded
/// assemblies, per-entity script instances, and lifecycle dispatch. The engine
/// provides both low-level methods ( [`load_script`](Self::load_script),
/// [`instantiate`](Self::instantiate) ) and higher-level component methods
/// ( [`attach_script`](Self::attach_script),
/// [`create_instances`](Self::create_instances) ).
pub struct ScriptEngine {
    /// One manager per registered script host.
    managers: Vec<ScriptManager>,
}

impl ScriptEngine {
    /// Create a new, empty script engine with no hosts registered.
    pub fn new() -> Self {
        Self {
            managers: Vec::new(),
        }
    }

    // ── Host registration ────────────────────────────────────────────────

    /// Register a script backend.
    ///
    /// The host's [`name`](ScriptHost::name) is used as the lookup key for
    /// subsequent [`load_script`](Self::load_script) calls.
    pub fn register_host(&mut self, host: Box<dyn ScriptHost>) {
        let host_name = host.name().to_string();
        self.managers.push(ScriptManager::new(host_name, host));
    }

    /// Return the number of registered script hosts.
    pub fn host_count(&self) -> usize {
        self.managers.len()
    }

    // ─── Low-level assembly / instance API (backward compat) ────────────

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
        let manager = self
            .managers
            .iter_mut()
            .find(|m| m.host_name == host_name)
            .ok_or_else(|| {
                ScriptError::HostError(format!(
                    "No script host registered with the name '{host_name}'"
                ))
            })?;

        let mut handle = manager.load_assembly(id, data)?;
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
        let manager = self
            .managers
            .iter_mut()
            .find(|m| m.host_name == handle.host_name)
            .ok_or_else(|| {
                ScriptError::HostError(format!(
                    "The host '{name}' that created this handle is no longer registered",
                    name = handle.host_name,
                ))
            })?;

        manager.host_mut().instantiate(handle)
    }

    // ── Component-based lifecycle API ─────────────────────────────────────

    /// Attach a script component to an entity through the specified host.
    ///
    /// The assembly referenced by
    /// [`ScriptComponent::assembly_id`] must have been
    /// loaded first via [`load_script`](Self::load_script).
    ///
    /// * `entity_id` — the ECS entity identifier.
    /// * `host_name` — must match a previously registered host.
    /// * `component` — the component metadata (assembly, class, fields).
    pub fn attach_script(
        &mut self,
        entity_id: &str,
        host_name: &str,
        component: &ScriptComponent,
    ) -> Result<(), ScriptError> {
        let manager = self
            .managers
            .iter_mut()
            .find(|m| m.host_name == host_name)
            .ok_or_else(|| {
                ScriptError::HostError(format!(
                    "No script host registered with the name '{host_name}'"
                ))
            })?;

        manager.attach(entity_id, component)
    }

    /// Detach all script instances for an entity from the named host.
    pub fn detach_script(&mut self, entity_id: &str, host_name: &str) {
        if let Some(manager) = self.managers.iter_mut().find(|m| m.host_name == host_name) {
            manager.detach(entity_id);
        }
    }

    /// Call `OnCreate` on all instances across all hosts.
    ///
    /// Returns any diagnostics produced during creation.
    pub fn create_instances(&mut self) -> Vec<Diagnostic> {
        let mut all = Vec::new();
        for manager in &mut self.managers {
            all.extend(manager.create_instances());
        }
        all
    }

    /// Call `OnDestroy` on all instances across all hosts.
    ///
    /// Returns any diagnostics produced during destruction.
    pub fn destroy_instances(&mut self) -> Vec<Diagnostic> {
        let mut all = Vec::new();
        for manager in &mut self.managers {
            all.extend(manager.destroy());
        }
        all
    }

    /// Tick all running script instances across all hosts.
    ///
    /// Dispatches `OnStart` (first tick only) and `OnUpdate(dt)`.
    /// Returns any diagnostics produced during the tick.
    pub fn update(&mut self, dt: f32) -> Vec<Diagnostic> {
        let mut all = Vec::new();
        for manager in &mut self.managers {
            all.extend(manager.update(dt));
        }
        all
    }

    /// Capture the current field values for a script on an entity (for scene
    /// save round-trips).
    pub fn capture_fields(
        &self,
        entity_id: &str,
        class_name: &str,
        host_name: &str,
    ) -> Option<std::collections::BTreeMap<String, crate::value::ScriptValue>> {
        self.managers
            .iter()
            .find(|m| m.host_name == host_name)
            .and_then(|m| m.capture_fields(entity_id, class_name))
    }

    // ── Query helpers ─────────────────────────────────────────────────────

    /// Iterate over all managers.
    pub fn managers(&self) -> &[ScriptManager] {
        &self.managers
    }

    /// Iterate mutably over all managers.
    pub fn managers_mut(&mut self) -> &mut [ScriptManager] {
        &mut self.managers
    }

    /// Find a manager by host name.
    pub fn find_manager(&self, host_name: &str) -> Option<&ScriptManager> {
        self.managers.iter().find(|m| m.host_name == host_name)
    }

    /// Find a manager by host name (mutable).
    pub fn find_manager_mut(&mut self, host_name: &str) -> Option<&mut ScriptManager> {
        self.managers.iter_mut().find(|m| m.host_name == host_name)
    }
}

impl Default for ScriptEngine {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{MockHost, NullScriptHost};

    // ── State tests (backward compat) ────────────────────────────────────

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
        engine.register_host(Box::new(MockHost::new()));
        assert_eq!(engine.host_count(), 2);
    }

    // ── Component integration tests ──────────────────────────────────────

    #[test]
    fn script_engine_attach_script_unknown_host() {
        let mut engine = ScriptEngine::new();
        let comp = ScriptComponent::new("asm", "MyScript");
        let result = engine.attach_script("entity_1", "nowhere", &comp);
        assert!(result.is_err());
    }

    #[test]
    fn script_engine_attach_and_create() {
        let mut engine = ScriptEngine::new();
        engine.register_host(Box::new(MockHost::new()));
        engine.load_script("asm", "mock", b"data").unwrap();
        let comp = ScriptComponent::new("asm", "MyScript");
        engine.attach_script("entity_1", "mock", &comp).unwrap();
        let diags = engine.create_instances();
        assert!(diags.is_empty());
    }

    #[test]
    fn script_engine_update_ticks_instances() {
        let mut engine = ScriptEngine::new();
        engine.register_host(Box::new(MockHost::new()));
        engine.load_script("asm", "mock", b"data").unwrap();
        engine
            .attach_script("entity_1", "mock", &ScriptComponent::new("asm", "MyScript"))
            .unwrap();
        engine.create_instances();
        let diags = engine.update(0.016);
        assert!(diags.is_empty());
    }

    #[test]
    fn script_engine_destroy_instances() {
        let mut engine = ScriptEngine::new();
        engine.register_host(Box::new(MockHost::new()));
        engine.load_script("asm", "mock", b"data").unwrap();
        engine
            .attach_script("entity_1", "mock", &ScriptComponent::new("asm", "MyScript"))
            .unwrap();
        engine.create_instances();
        let diags = engine.destroy_instances();
        assert!(diags.is_empty());
    }

    #[test]
    fn script_engine_detach_removes_entity() {
        let mut engine = ScriptEngine::new();
        engine.register_host(Box::new(MockHost::new()));
        engine.load_script("asm", "mock", b"data").unwrap();
        engine
            .attach_script("entity_1", "mock", &ScriptComponent::new("asm", "MyScript"))
            .unwrap();
        assert_eq!(engine.managers()[0].instance_count(), 1);
        engine.detach_script("entity_1", "mock");
        assert_eq!(engine.managers()[0].instance_count(), 0);
    }

    #[test]
    fn script_engine_capture_fields() {
        let mut engine = ScriptEngine::new();
        engine.register_host(Box::new(MockHost::new()));
        engine.load_script("asm", "mock", b"data").unwrap();
        let comp = ScriptComponent::new("asm", "MyScript")
            .with_field("speed", crate::value::ScriptValue::Float(50.0));
        engine.attach_script("entity_1", "mock", &comp).unwrap();
        let fields = engine.capture_fields("entity_1", "MyScript", "mock");
        assert!(fields.is_some());
    }

    #[test]
    fn script_engine_find_manager() {
        let mut engine = ScriptEngine::new();
        engine.register_host(Box::new(MockHost::new()));
        assert!(engine.find_manager("mock").is_some());
        assert!(engine.find_manager("nowhere").is_none());
    }
}
