#![forbid(unsafe_code)]

pub mod diagnostics;
pub use diagnostics::*;

use engine_renderer::{FrameStats, Renderer};
use engine_scene::{extract_renderer_input, Scene};
use engine_serialize::{Diagnostic, DiagnosticSeverity};

pub mod coroutine;

// ── Optional script subsystem ─────────────────────────────────────────────

#[cfg(feature = "subsystem-scripting-csharp")]
pub mod script;
#[cfg(feature = "subsystem-scripting-csharp")]
use script::{collect_scene_scripts, script_engine_state_summary};
#[cfg(feature = "subsystem-scripting-csharp")]
use engine_script::{ScriptEngine, ScriptError, ScriptHost};

// ── Engine config ─────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineConfig {
    pub application_name: String,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            application_name: "engine".to_string(),
        }
    }
}

// ── Engine runtime ────────────────────────────────────────────────────────

pub struct EngineRuntime {
    config: EngineConfig,
    renderer: Renderer,
    scene: Option<Scene>,
    collector: DiagnosticsCollector,
    pub coroutines: coroutine::CoroutineSystem,
    #[cfg(feature = "subsystem-scripting-csharp")]
    script_engine: ScriptEngine,
    /// Name of the script host to use when loading scene scripts.
    #[cfg(feature = "subsystem-scripting-csharp")]
    script_host_name: String,
}

impl EngineRuntime {
    pub fn new(config: EngineConfig) -> Self {
        Self {
            config,
            renderer: Renderer::new(),
            scene: None,
            collector: DiagnosticsCollector::new(),
            coroutines: coroutine::CoroutineSystem::new(),
            #[cfg(feature = "subsystem-scripting-csharp")]
            script_engine: ScriptEngine::new(),
            #[cfg(feature = "subsystem-scripting-csharp")]
            script_host_name: "dotnet".to_string(),
        }
    }

    pub fn config(&self) -> &EngineConfig {
        &self.config
    }

    /// Load a scene and attach any script components found on entities.
    pub fn load_scene(&mut self, scene: Scene) {
        // Attach scene scripts (if the script subsystem is enabled)
        #[cfg(feature = "subsystem-scripting-csharp")]
        self.attach_scene_scripts(&scene);

        self.scene = Some(scene);
    }

    /// Access the diagnostics collector (immutable).
    pub fn diagnostics_collector(&self) -> &DiagnosticsCollector {
        &self.collector
    }

    /// Access the diagnostics collector (mutable).
    pub fn diagnostics_collector_mut(&mut self) -> &mut DiagnosticsCollector {
        &mut self.collector
    }

    /// Build an aggregate [`RuntimeDiagnostics`] snapshot for editor/tooling.
    pub fn runtime_diagnostics(&self) -> RuntimeDiagnostics {
        RuntimeDiagnostics {
            collector: self.collector.clone(),
            reload_queue: None,
            #[cfg(feature = "subsystem-scripting-csharp")]
            script_engine_state: format!(
                "{} coroutines={}",
                script_engine_state_summary(&self.script_engine),
                self.coroutines.active_count(),
            ),
            #[cfg(not(feature = "subsystem-scripting-csharp"))]
            script_engine_state: format!("coroutines={}", self.coroutines.active_count()),
        }
    }

    /// Render one frame and record GPU statistics into the diagnostics collector.
    pub fn render_frame(&mut self, frame_index: u64) -> Result<FrameStats, Vec<Diagnostic>> {
        let scene = self.scene.as_ref().ok_or_else(|| {
            vec![Diagnostic::new(
                "SC0018",
                DiagnosticSeverity::Error,
                "engine-core",
                "no scene is loaded",
            )]
        })?;
        let input = extract_renderer_input(scene, frame_index)?;
        let result = self.renderer.draw_scene(&input);
        if let Ok(stats) = &result {
            self.collector.record_frame(frame_index, stats);
        }
        result
    }

    // ── Script subsystem public API (only when feature is enabled) ─────

    /// Register a script backend host (e.g. `ProcessHost` for C#).
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn register_script_host(&mut self, host: Box<dyn ScriptHost>) {
        self.script_engine.register_host(host);
    }

    /// Load a script assembly through the named host.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn load_script_assembly(
        &mut self,
        id: &str,
        host_name: &str,
        data: &[u8],
    ) -> Result<(), ScriptError> {
        self.script_engine.load_script(id, host_name, data)?;
        Ok(())
    }

    /// Direct access to the script engine.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn script_engine(&self) -> &ScriptEngine {
        &self.script_engine
    }

    /// Mutable access to the script engine.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn script_engine_mut(&mut self) -> &mut ScriptEngine {
        &mut self.script_engine
    }

    /// Set the script host name used for scene-attached scripts.
    ///
    /// Must match the [`name`](ScriptHost::name) of a registered host.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn set_script_host_name(&mut self, name: impl Into<String>) {
        self.script_host_name = name.into();
    }

    /// Tick all scripts — call this each frame before `render_frame`.
    ///
    /// Dispatches `OnStart`/`OnUpdate(dt)` on every active script instance
    /// and pushes any resulting diagnostics into the collector.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn tick_scripts(&mut self, dt: f32) {
        let diags = self.script_engine.update(dt);
        self.collector.push_script_diags(diags);
    }

    // ── Private helpers ─────────────────────────────────────────────────

    /// Iterate scene entities and attach any `"engine.script"` components.
    #[cfg(feature = "subsystem-scripting-csharp")]
    fn attach_scene_scripts(&mut self, scene: &Scene) {
        let scripts = collect_scene_scripts(scene);
        let host_name = &self.script_host_name;
        for (entity_id, component) in &scripts {
            // The assembly must have been loaded externally (e.g. via
            // `load_script_assembly`). If it hasn't, the attach will
            // produce a ScriptError and we push a diagnostic.
            match self
                .script_engine
                .attach_script(entity_id, host_name, component)
            {
                Ok(()) => {}
                Err(e) => {
                    let diag = Diagnostic::new(
                        "SCR_ATTACH_FAILED",
                        DiagnosticSeverity::Error,
                        "engine-core",
                        format!(
                            "Failed to attach script '{}' to entity '{}': {e}",
                            component.class_name, entity_id
                        ),
                    );
                    self.collector.push_script_diags(vec![diag]);
                }
            }
        }

        // Call OnCreate on all newly-attached instances
        let create_diags = self.script_engine.create_instances();
        self.collector.push_script_diags(create_diags);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── EngineConfig tests ───────────────────────────────────────────────

    #[test]
    fn engine_config_defaults() {
        let config = EngineConfig::default();
        assert_eq!(config.application_name, "engine");
    }

    #[test]
    fn engine_config_debug() {
        let config = EngineConfig::default();
        let debug = format!("{:?}", config);
        assert!(debug.contains("EngineConfig"));
    }

    #[test]
    fn engine_config_partial_eq() {
        let a = EngineConfig::default();
        let b = EngineConfig::default();
        let c = EngineConfig {
            application_name: "custom".to_string(),
        };
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn engine_config_clone() {
        let config = EngineConfig::default();
        let cloned = config.clone();
        assert_eq!(config, cloned);
    }

    // ── EngineRuntime tests ──────────────────────────────────────────────

    #[test]
    fn engine_runtime_creation() {
        let config = EngineConfig::default();
        let runtime = EngineRuntime::new(config.clone());
        assert_eq!(*runtime.config(), config);
    }

    #[test]
    fn engine_runtime_config_accessor() {
        let config = EngineConfig::default();
        let runtime = EngineRuntime::new(config);
        let retrieved = runtime.config();
        assert_eq!(retrieved.application_name, "engine");
    }

    #[test]
    fn engine_runtime_render_frame_without_scene_fails() {
        let config = EngineConfig::default();
        let mut runtime = EngineRuntime::new(config);
        let result = runtime.render_frame(0);
        assert!(result.is_err());
    }

    #[test]
    fn engine_runtime_diagnostics_collector() {
        let config = EngineConfig::default();
        let runtime = EngineRuntime::new(config);
        let collector = runtime.diagnostics_collector();
        assert!(collector.all().is_empty());
    }

    #[test]
    fn engine_runtime_runtime_diagnostics() {
        let config = EngineConfig::default();
        let runtime = EngineRuntime::new(config);
        let rd = runtime.runtime_diagnostics();
        assert!(rd.script_engine_state.contains("coroutines=0"), "missing coroutines=0");
        assert!(rd.reload_queue.is_none());
    }

    // ── Script subsystem tests ──────────────────────────────────────────

    #[cfg(feature = "subsystem-scripting-csharp")]
    #[test]
    fn engine_runtime_script_host_registration() {
        use engine_script::MockHost;

        let config = EngineConfig::default();
        let mut runtime = EngineRuntime::new(config);

        assert_eq!(runtime.script_engine.host_count(), 0);
        runtime.register_script_host(Box::new(MockHost::new()));
        assert_eq!(runtime.script_engine.host_count(), 1);
    }

    #[cfg(feature = "subsystem-scripting-csharp")]
    #[test]
    fn engine_runtime_tick_scripts_no_panic() {
        let config = EngineConfig::default();
        let mut runtime = EngineRuntime::new(config);

        // Tick with no hosts registered — should not panic
        runtime.tick_scripts(0.016);
    }

    #[cfg(feature = "subsystem-scripting-csharp")]
    #[test]
    fn engine_runtime_load_scene_with_scripts() {
        use engine_script::MockHost;
        use std::collections::BTreeMap;
        use engine_scene::ComponentRecord;
        use engine_serialize::SchemaVersion;

        let config = EngineConfig::default();
        let mut runtime = EngineRuntime::new(config);
        runtime.register_script_host(Box::new(MockHost::new()));
        // Match the host name used by MockHost
        runtime.set_script_host_name("mock");

        // Create a minimal scene with a script component
        let mut script_fields = BTreeMap::new();
        script_fields.insert("assembly_id".into(), engine_serialize::Value::Str("asm".into()));
        script_fields.insert("class_name".into(), engine_serialize::Value::Str("MyScript".into()));

        let mut components = BTreeMap::new();
        components.insert(
            "engine.script".to_string(),
            ComponentRecord {
                schema_version: SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: script_fields,
            },
        );

        let scene = engine_scene::Scene {
            schema_version: SchemaVersion::new(0, 1, 0),
            engine_version: "0.1.0".to_string(),
            scene_id: "test".to_string(),
            name: "test".to_string(),
            entities: vec![engine_scene::EntityRecord {
                persistent_id: "ent-1".to_string(),
                parent: None,
                name: Some("Entity".to_string()),
                enabled: true,
                components,
            }],
            scene_settings: engine_scene::SceneSettings::default(),
            dependencies: vec![],
            diagnostics_policy: engine_scene::DiagnosticsPolicy::Strict,
        };

        // Pre-load the assembly that the script references
        runtime
            .load_script_assembly("asm", "mock", b"mock_data")
            .unwrap();

        // Load scene — should attach scripts
        runtime.load_scene(scene);

        // After load_scene, the script engine should have an instance
        assert_eq!(runtime.script_engine.host_count(), 1);
        let after = runtime.script_engine.managers()[0].instance_count();
        assert_eq!(after, 1, "script instance should have been created");

        // Tick should not produce errors
        runtime.tick_scripts(0.016);
    }
}
