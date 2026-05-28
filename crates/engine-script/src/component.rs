//! Script component integration with ECS scenes.
//!
//! Provides [`ScriptComponent`] (serialisable metadata that survives scene
//! save/load), [`ScriptInstanceState`] (runtime per-instance state), and
//! [`ScriptManager`] (orchestrates lifecycle dispatch against a single host).

use std::collections::{BTreeMap, HashMap};

use engine_serialize::Diagnostic;
use serde::{Deserialize, Serialize};

use crate::lifecycle::lifecycle;
use crate::value::ScriptValue;
use crate::host::{ScriptError, ScriptHandle, ScriptHost, ScriptInstance};

// ---------------------------------------------------------------------------
// ScriptComponent — serialisable scene component
// ---------------------------------------------------------------------------

/// Metadata about a script component attached to an ECS entity.
///
/// This struct is designed to be serialised and deserialised as part of a scene
/// file so that script attachments survive save/load round-trips.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScriptComponent {
    /// Assembly identifier (e.g. an asset id or path).
    pub assembly_id: String,
    /// The script class name within the assembly.
    pub class_name: String,
    /// Serialised field values that survive scene save/load.
    pub fields: BTreeMap<String, ScriptValue>,
    /// Whether this script is enabled.
    pub enabled: bool,
}

impl ScriptComponent {
    /// Create a new script component.
    pub fn new(
        assembly_id: impl Into<String>,
        class_name: impl Into<String>,
    ) -> Self {
        Self {
            assembly_id: assembly_id.into(),
            class_name: class_name.into(),
            fields: BTreeMap::new(),
            enabled: true,
        }
    }

    /// Add a field value.
    pub fn with_field(mut self, name: impl Into<String>, value: ScriptValue) -> Self {
        self.fields.insert(name.into(), value);
        self
    }
}

// ---------------------------------------------------------------------------
// ScriptInstanceState — runtime per-instance data (not serialised)
// ---------------------------------------------------------------------------

/// Runtime state of a script instance (not serialised to scene files).
pub struct ScriptInstanceState {
    /// Handle to the loaded assembly that owns this instance.
    pub handle: ScriptHandle,
    /// The boxed script instance object.
    pub instance: Box<dyn ScriptInstance>,
    /// Whether `OnCreate` has been called.
    pub created: bool,
    /// Whether `OnStart` has been called.
    pub started: bool,
    /// The original component metadata (for field serialisation round-trips).
    pub component: ScriptComponent,
}

impl std::fmt::Debug for ScriptInstanceState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScriptInstanceState")
            .field("handle", &self.handle)
            .field("created", &self.created)
            .field("started", &self.started)
            .field("component", &self.component)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// ScriptManager — lifecycle orchestrator for a single host
// ---------------------------------------------------------------------------

/// Manages script component instances and lifecycle dispatch for a single host.
///
/// Each [`ScriptManager`] wraps one [`ScriptHost`] backend and tracks all
/// instances attached to entities. The engine creates one manager per
/// registered host.
pub struct ScriptManager {
    /// Name of the wrapped host (e.g. `"dotnet"`, `"mock"`).
    pub host_name: String,
    /// The script host backend.
    host: Box<dyn ScriptHost>,
    /// Loaded assemblies: `assembly_id → ScriptHandle`.
    assemblies: HashMap<String, ScriptHandle>,
    /// Instances indexed by entity id.
    /// `entity_id → [(class_name, instance_state)]`.
    instances: BTreeMap<String, Vec<(String, ScriptInstanceState)>>,
}

impl ScriptManager {
    /// Create a new manager wrapping the given host.
    pub fn new(host_name: String, host: Box<dyn ScriptHost>) -> Self {
        Self {
            host_name,
            host,
            assemblies: HashMap::new(),
            instances: BTreeMap::new(),
        }
    }

    /// Return a reference to the underlying host.
    pub fn host(&self) -> &dyn ScriptHost {
        self.host.as_ref()
    }

    /// Return a mutable reference to the underlying host.
    pub fn host_mut(&mut self) -> &mut dyn ScriptHost {
        self.host.as_mut()
    }

    // ── Assembly management ───────────────────────────────────────────────

    /// Load an assembly through the host and cache the handle.
    pub fn load_assembly(
        &mut self,
        id: &str,
        data: &[u8],
    ) -> Result<ScriptHandle, ScriptError> {
        if self.assemblies.contains_key(id) {
            return Ok(self.assemblies[id].clone());
        }
        let handle = self.host.load_assembly(id, data)?;
        self.assemblies.insert(id.to_string(), handle.clone());
        Ok(handle)
    }

    /// Unload an assembly and invalidate all instances that depend on it.
    pub fn unload_assembly(&mut self, id: &str) -> Result<(), ScriptError> {
        if let Some(handle) = self.assemblies.remove(id) {
            // Remove any instances that belong to this assembly
            for (_entity_id, scripts) in self.instances.iter_mut() {
                scripts.retain(|(_, state)| state.handle.id() != handle.id());
            }
            self.host.unload(&handle)?;
        }
        Ok(())
    }

    // ── Instance lifecycle ────────────────────────────────────────────────

    /// Attach a script component to an entity.
    ///
    /// The assembly must have been loaded first via
    /// [`load_assembly`](Self::load_assembly). This method instantiates the
    /// class and applies the component's serialised fields.
    pub fn attach(
        &mut self,
        entity_id: &str,
        component: &ScriptComponent,
    ) -> Result<(), ScriptError> {
        let handle = self
            .assemblies
            .get(&component.assembly_id)
            .ok_or_else(|| {
                ScriptError::LoadFailed(format!(
                    "Assembly '{}' has not been loaded. Call load_assembly first.",
                    component.assembly_id
                ))
            })?;

        let mut instance = self.host.instantiate(handle)?;

        // Apply serialised fields to the instance
        for (name, value) in &component.fields {
            instance.set_field(name, value.clone())?;
        }

        let state = ScriptInstanceState {
            handle: handle.clone(),
            instance,
            created: false,
            started: false,
            component: component.clone(),
        };

        self.instances
            .entry(entity_id.to_string())
            .or_default()
            .push((component.class_name.clone(), state));

        Ok(())
    }

    /// Detach all script instances for an entity.
    pub fn detach(&mut self, entity_id: &str) {
        self.instances.remove(entity_id);
    }

    /// Detach a specific script class from an entity.
    pub fn detach_class(&mut self, entity_id: &str, class_name: &str) {
        if let Some(scripts) = self.instances.get_mut(entity_id) {
            scripts.retain(|(cn, _)| cn != class_name);
        }
    }

    /// Call `OnCreate` on every instance that hasn't been created yet.
    ///
    /// Returns any diagnostics produced during creation.
    pub fn create_instances(&mut self) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        for (_entity_id, scripts) in self.instances.iter_mut() {
            for (_class_name, state) in scripts.iter_mut() {
                if !state.created {
                    match state.instance.call(lifecycle::ON_CREATE, &[]) {
                        Ok(_) => {
                            state.created = true;
                        }
                        Err(e) => {
                            let mut diag = Diagnostic::new(
                                "SCRIPT_CREATE_FAILED",
                                engine_serialize::DiagnosticSeverity::Error,
                                "script",
                                format!("OnCreate failed for '{}': {e}", state.component.class_name),
                            );
                            diag.entity = Some(state.handle.id().to_string());
                            diagnostics.push(diag);
                        }
                    }
                }
            }
        }
        diagnostics
    }

    /// Tick all started instances, calling `OnStart` first if needed, then
    /// `OnUpdate(dt)`.
    ///
    /// Returns any diagnostics produced during the tick.
    pub fn update(&mut self, dt: f32) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();

        for (_entity_id, scripts) in self.instances.iter_mut() {
            for (_class_name, state) in scripts.iter_mut() {
                // Skip disabled instances
                if !state.component.enabled {
                    continue;
                }

                // Call OnStart before the first update
                if state.created && !state.started {
                    match state.instance.call(lifecycle::ON_START, &[]) {
                        Ok(_) => {
                            state.started = true;
                        }
                        Err(e) => {
                            let mut diag = Diagnostic::new(
                                "SCRIPT_START_FAILED",
                                engine_serialize::DiagnosticSeverity::Error,
                                "script",
                                format!(
                                    "OnStart failed for '{}': {e}",
                                    state.component.class_name
                                ),
                            );
                            diag.entity = Some(state.handle.id().to_string());
                            diagnostics.push(diag);
                            continue;
                        }
                    }
                }

                // Call OnUpdate
                if state.started {
                    let dt_arg = ScriptValue::Float(dt as f64);
                    match state.instance.call(lifecycle::ON_UPDATE, &[dt_arg]) {
                        Ok(_) => {}
                        Err(e) => {
                            let mut diag = Diagnostic::new(
                                "SCRIPT_UPDATE_FAILED",
                                engine_serialize::DiagnosticSeverity::Error,
                                "script",
                                format!(
                                    "OnUpdate failed for '{}': {e}",
                                    state.component.class_name
                                ),
                            );
                            diag.entity = Some(state.handle.id().to_string());
                            diagnostics.push(diag);
                        }
                    }
                }
            }
        }

        diagnostics
    }

    /// Call `OnDestroy` on every instance.
    ///
    /// Returns any diagnostics produced during destruction.
    pub fn destroy(&mut self) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();

        for (_entity_id, scripts) in self.instances.iter_mut() {
            for (_class_name, state) in scripts.iter_mut() {
                if state.created {
                    match state.instance.call(lifecycle::ON_DESTROY, &[]) {
                        Ok(_) => {}
                        Err(e) => {
                            let mut diag = Diagnostic::new(
                                "SCRIPT_DESTROY_FAILED",
                                engine_serialize::DiagnosticSeverity::Error,
                                "script",
                                format!(
                                    "OnDestroy failed for '{}': {e}",
                                    state.component.class_name
                                ),
                            );
                            diag.entity = Some(state.handle.id().to_string());
                            diagnostics.push(diag);
                        }
                    }
                }
            }
        }

        diagnostics
    }

    /// Remove all instances (without calling OnDestroy).
    pub fn clear(&mut self) {
        self.instances.clear();
    }

    // ── Query helpers ─────────────────────────────────────────────────────

    /// Number of tracked script instances.
    pub fn instance_count(&self) -> usize {
        self.instances.values().map(|v| v.len()).sum()
    }

    /// Number of loaded assemblies.
    pub fn assembly_count(&self) -> usize {
        self.assemblies.len()
    }

    /// Iterate over all instances (entity_id, class_name, state).
    pub fn iter_instances(
        &self,
    ) -> impl Iterator<Item = (&str, &str, &ScriptInstanceState)> {
        self.instances
            .iter()
            .flat_map(|(eid, scripts)| {
                scripts
                    .iter()
                    .map(move |(cn, state)| (eid.as_str(), cn.as_str(), state))
            })
    }

    /// Get the serialisable fields from an instance (for scene save).
    pub fn capture_fields(
        &self,
        entity_id: &str,
        class_name: &str,
    ) -> Option<BTreeMap<String, ScriptValue>> {
        self.instances.get(entity_id).and_then(|scripts| {
            scripts
                .iter()
                .find(|(cn, _)| cn == class_name)
                .map(|(_, state)| {
                    // Start with the original fields, then overlay any runtime
                    // changes by attempting to read each field back.
                    let mut fields = state.component.fields.clone();
                    for key in fields.keys().cloned().collect::<Vec<_>>() {
                        if let Some(val) = state.instance.get_field(&key) {
                            fields.insert(key, val);
                        }
                    }
                    fields
                })
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::MockHost;

    fn make_manager() -> ScriptManager {
        ScriptManager::new("mock".to_string(), Box::new(MockHost::new()))
    }

    #[test]
    fn script_component_new() {
        let c = ScriptComponent::new("asm", "MyScript");
        assert_eq!(c.assembly_id, "asm");
        assert_eq!(c.class_name, "MyScript");
        assert!(c.enabled);
        assert!(c.fields.is_empty());
    }

    #[test]
    fn script_component_with_field() {
        let c = ScriptComponent::new("a", "B")
            .with_field("speed", ScriptValue::Float(100.0));
        assert_eq!(c.fields.len(), 1);
        assert_eq!(
            c.fields.get("speed"),
            Some(&ScriptValue::Float(100.0))
        );
    }

    #[test]
    fn script_manager_new_is_empty() {
        let m = make_manager();
        assert_eq!(m.instance_count(), 0);
        assert_eq!(m.assembly_count(), 0);
        assert_eq!(m.host_name, "mock");
    }

    #[test]
    fn script_manager_load_assembly() {
        let mut m = make_manager();
        m.load_assembly("test_asm", b"mock_data").unwrap();
        assert_eq!(m.assembly_count(), 1);
    }

    #[test]
    fn script_manager_load_assembly_dedup() {
        let mut m = make_manager();
        m.load_assembly("test_asm", b"mock_data").unwrap();
        let h1 = m.load_assembly("test_asm", b"mock_data").unwrap();
        let h2 = m.assemblies.get("test_asm").unwrap();
        assert_eq!(h1.id(), h2.id());
    }

    #[test]
    fn script_manager_attach_needs_loaded_assembly() {
        let mut m = make_manager();
        let c = ScriptComponent::new("missing_asm", "MyScript");
        let result = m.attach("entity_1", &c);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("has not been loaded"));
    }

    #[test]
    fn script_manager_attach_and_instance_count() {
        let mut m = make_manager();
        m.load_assembly("asm", b"data").unwrap();
        let c = ScriptComponent::new("asm", "MyScript");
        m.attach("entity_1", &c).unwrap();
        assert_eq!(m.instance_count(), 1);
    }

    #[test]
    fn script_manager_create_instances_calls_oncreate() {
        let mut m = make_manager();
        m.load_assembly("asm", b"data").unwrap();
        m.attach("entity_1", &ScriptComponent::new("asm", "MyScript"))
            .unwrap();
        let diags = m.create_instances();
        assert!(diags.is_empty());
        // Verify created flag is set
        let (_, _, state) = m.iter_instances().next().unwrap();
        assert!(state.created);
    }

    #[test]
    fn script_manager_update_triggers_onstart_and_onupdate() {
        let mut m = make_manager();
        m.load_assembly("asm", b"data").unwrap();
        m.attach("entity_1", &ScriptComponent::new("asm", "MyScript"))
            .unwrap();
        m.create_instances();
        let diags = m.update(0.016);
        assert!(diags.is_empty());
        let (_, _, state) = m.iter_instances().next().unwrap();
        assert!(state.started);
    }

    #[test]
    fn script_manager_destroy_calls_ondestroy() {
        let mut m = make_manager();
        m.load_assembly("asm", b"data").unwrap();
        m.attach("entity_1", &ScriptComponent::new("asm", "MyScript"))
            .unwrap();
        m.create_instances();
        let diags = m.destroy();
        assert!(diags.is_empty());
    }

    #[test]
    fn script_manager_detach_removes_entity() {
        let mut m = make_manager();
        m.load_assembly("asm", b"data").unwrap();
        m.attach("entity_1", &ScriptComponent::new("asm", "MyScript"))
            .unwrap();
        assert_eq!(m.instance_count(), 1);
        m.detach("entity_1");
        assert_eq!(m.instance_count(), 0);
    }

    #[test]
    fn script_manager_capture_fields() {
        let mut m = make_manager();
        m.load_assembly("asm", b"data").unwrap();
        let c = ScriptComponent::new("asm", "MyScript")
            .with_field("speed", ScriptValue::Float(50.0));
        m.attach("entity_1", &c).unwrap();
        let fields = m.capture_fields("entity_1", "MyScript");
        assert!(fields.is_some());
        assert_eq!(
            fields.unwrap().get("speed"),
            Some(&ScriptValue::Float(50.0))
        );
    }

    #[test]
    fn script_manager_disabled_instances_skip_update() {
        let mut m = make_manager();
        m.load_assembly("asm", b"data").unwrap();
        let mut c = ScriptComponent::new("asm", "MyScript");
        c.enabled = false;
        m.attach("entity_1", &c).unwrap();
        m.create_instances();
        let diags = m.update(0.016);
        assert!(diags.is_empty());
        // Should NOT have started since it's disabled
        let (_, _, state) = m.iter_instances().next().unwrap();
        assert!(!state.started);
    }
}
