#![forbid(unsafe_code)]

pub mod diagnostics;
pub use diagnostics::*;

use engine_renderer::{FrameStats, Renderer};
use engine_scene::{
    extract_renderer_input_from_world, validate_scene, ComponentRegistry, Scene,
    SceneLoadDiagnostic, World, WorldSlot,
};
use engine_serialize::{Diagnostic, DiagnosticSeverity};
use std::sync::Arc;

pub mod ffi_init;
pub mod game_loop;

// ── Optional script subsystem ─────────────────────────────────────────────

#[cfg(feature = "subsystem-scripting-csharp")]
pub mod script;
#[cfg(feature = "subsystem-scripting-csharp")]
use engine_script::{ScriptEngine, ScriptError, ScriptHost};
#[cfg(feature = "subsystem-scripting-csharp")]
use script::{collect_scene_scripts, script_engine_state_summary};

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

/// Configures an [`EngineRuntime`] before its shared component registry is
/// frozen behind an [`Arc`].
///
/// Character-controller components are always registered. Physics components
/// are additionally registered when the `gameplay` feature is enabled. Other
/// subsystems can register their extensions through
/// [`component_registry_mut`](Self::component_registry_mut) before calling
/// [`build`](Self::build).
pub struct EngineRuntimeBuilder {
    config: EngineConfig,
    component_registry: ComponentRegistry,
}

impl EngineRuntimeBuilder {
    pub fn new(config: EngineConfig) -> Self {
        let mut component_registry = ComponentRegistry::new();
        component_registry.register_core();
        engine_character::register_character_extensions(&mut component_registry, None);
        #[cfg(feature = "gameplay")]
        engine_physics::register_physics_extensions(&mut component_registry, None);

        Self {
            config,
            component_registry,
        }
    }

    /// Inspect the component extensions that will be shared by the runtime.
    pub fn component_registry(&self) -> &ComponentRegistry {
        &self.component_registry
    }

    /// Register additional component extensions before building the runtime.
    pub fn component_registry_mut(&mut self) -> &mut ComponentRegistry {
        &mut self.component_registry
    }

    pub fn build(self) -> EngineRuntime {
        EngineRuntime::from_parts(self.config, Arc::new(self.component_registry))
    }
}

impl Default for EngineRuntimeBuilder {
    fn default() -> Self {
        Self::new(EngineConfig::default())
    }
}

// ── Engine runtime ────────────────────────────────────────────────────────

pub struct EngineRuntime {
    config: EngineConfig,
    renderer: Renderer,
    scene: Option<Scene>,
    world_slot: WorldSlot,
    component_registry: Arc<ComponentRegistry>,
    collector: DiagnosticsCollector,
    #[cfg(feature = "subsystem-scripting-csharp")]
    script_engine: ScriptEngine,
    /// Name of the script host to use when loading scene scripts.
    #[cfg(feature = "subsystem-scripting-csharp")]
    script_host_name: String,
}

impl EngineRuntime {
    pub fn new(config: EngineConfig) -> Self {
        Self::builder(config).build()
    }

    pub fn builder(config: EngineConfig) -> EngineRuntimeBuilder {
        EngineRuntimeBuilder::new(config)
    }

    /// Enable direct P/Invoke access for a C# runtime hosted in this process.
    ///
    /// This loads the version-matched `engine_ffi` native library, validates
    /// its callback-table ABI, and binds it to this process's active World
    /// bridge. Call it before executing managed code in an in-process CLR.
    /// Out-of-process [`engine_script::ProcessHost`] users must use IPC and
    /// should not install this bridge.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn install_in_process_csharp_ffi(
        &self,
    ) -> Result<(), engine_ffi::host_bridge::HostBridgeError> {
        ffi_init::install_cdylib_bridge()
    }

    fn from_parts(config: EngineConfig, component_registry: Arc<ComponentRegistry>) -> Self {
        // Initialise the FFI callback registry so extern "C" entry points
        // can dispatch to real implementations immediately. The active world
        // slot is selected later when a scene is loaded.
        ffi_init::initialise();

        Self {
            config,
            renderer: Renderer::new(),
            scene: None,
            world_slot: WorldSlot::new(),
            component_registry,
            collector: DiagnosticsCollector::new(),
            #[cfg(feature = "subsystem-scripting-csharp")]
            script_engine: ScriptEngine::new(),
            #[cfg(feature = "subsystem-scripting-csharp")]
            script_host_name: "dotnet".to_string(),
        }
    }

    pub fn config(&self) -> &EngineConfig {
        &self.config
    }

    /// Shared registry used by strict scene loading and registry-less worlds.
    pub fn component_registry(&self) -> &Arc<ComponentRegistry> {
        &self.component_registry
    }

    /// Load a scene into the runtime ECS World.
    ///
    /// This compatibility name now delegates to the strict transactional
    /// loader. Keeping a Scene-only fallback produced identity transforms and
    /// made normal cameras and positioned objects render incorrectly.
    pub fn load_scene(&mut self, scene: Scene) -> Result<(), Vec<Diagnostic>> {
        self.load_scene_to_world(scene)
    }

    /// Load a scene and also build the ECS World from it.
    ///
    /// This is the recommended entry point for runtime games that need
    /// transforms, physics, and gameplay logic in addition to rendering.
    pub fn load_scene_to_world(&mut self, scene: Scene) -> Result<(), Vec<Diagnostic>> {
        let validation_diagnostics = validate_scene(&scene);
        let validation_failed = validation_diagnostics.iter().any(|diagnostic| {
            matches!(
                diagnostic.severity,
                DiagnosticSeverity::Error | DiagnosticSeverity::Fatal
            )
        });
        if !validation_diagnostics.is_empty() {
            self.collector
                .push_scene_diags(validation_diagnostics.clone());
        }
        if validation_failed {
            return Err(validation_diagnostics);
        }

        #[cfg(feature = "subsystem-scripting-csharp")]
        let world_scene = {
            // `engine.script` is scene-only metadata consumed by the script
            // subsystem. Keep it in the retained Scene, but do not ask the ECS
            // component registry to materialise it. No other type is ignored.
            let mut world_scene = scene.clone();
            for entity in &mut world_scene.entities {
                entity.components.remove(script::SCRIPT_COMPONENT_TYPE);
            }
            world_scene
        };
        #[cfg(feature = "subsystem-scripting-csharp")]
        let world_scene = &world_scene;
        #[cfg(not(feature = "subsystem-scripting-csharp"))]
        let world_scene = &scene;

        // Build outside the WorldSlot lock. A failed load leaves the currently
        // active World, Scene, and FFI binding untouched.
        let world = match World::try_from_scene_with_registry(
            world_scene,
            Arc::clone(&self.component_registry),
        ) {
            Ok(world) => world,
            Err(error) => {
                let diagnostics = error
                    .diagnostics
                    .into_iter()
                    .map(scene_load_diagnostic)
                    .collect::<Vec<_>>();
                self.collector.push_scene_diags(diagnostics.clone());
                return Err(diagnostics);
            }
        };

        self.world_slot.replace(world);
        engine_ffi::world_bridge::activate_world(&self.world_slot, &self.component_registry);

        // Attach scripts only after activating the new world so managed
        // OnCreate callbacks cannot observe the previous scene.
        #[cfg(feature = "subsystem-scripting-csharp")]
        self.attach_scene_scripts(&scene);

        self.scene = Some(scene);
        Ok(())
    }

    /// Directly set an existing ECS World as the runtime's active world.
    ///
    /// This is the preferred entry point when building a World manually
    /// via `World::new()` + `create_entity()` + `add_component()`.
    /// Unlike `load_scene_to_world` it avoids the `to_scene()/from_scene()`
    /// serialisation round-trip.
    ///
    /// The world must contain at least one enabled [`Camera`] component
    /// and at least one enabled [`Renderable`] component for extraction
    /// to produce a valid frame.
    pub fn set_world(&mut self, mut world: World) {
        // A caller-provided registry is authoritative. Otherwise install the
        // runtime registry before serialising so external components survive
        // the Scene snapshot used by legacy rendering and scripts.
        let effective_registry = if let Some(registry) = world.component_registry() {
            Arc::clone(registry)
        } else {
            let registry = Arc::clone(&self.component_registry);
            world.set_shared_component_registry(Arc::clone(&registry));
            registry
        };

        // Derive a Scene snapshot for inspection and script attachment. Frame
        // extraction always reads the active World.
        let scene = world.to_scene();

        self.world_slot.replace(world);
        engine_ffi::world_bridge::activate_world(&self.world_slot, &effective_registry);

        #[cfg(feature = "subsystem-scripting-csharp")]
        self.attach_scene_scripts(&scene);

        self.scene = Some(scene);
    }

    /// Execute a closure with immutable access to the active ECS world.
    pub fn with_world<F, R>(&self, f: F) -> Option<R>
    where
        F: FnOnce(&World) -> R,
    {
        self.world_slot.with_world(f)
    }

    /// Execute a closure with mutable access to the active ECS world.
    pub fn with_world_mut<F, R>(&self, f: F) -> Option<R>
    where
        F: FnOnce(&mut World) -> R,
    {
        self.world_slot.with_world_mut(f)
    }

    /// Returns `true` when an ECS world is loaded.
    pub fn has_world(&self) -> bool {
        self.world_slot.has_world()
    }

    /// Mutable access to the renderer for backend configuration and mesh uploads.
    pub fn renderer_mut(&mut self) -> &mut Renderer {
        &mut self.renderer
    }

    /// Immutable access to the loaded scene (if any).
    pub fn scene_ref(&self) -> Option<&Scene> {
        self.scene.as_ref()
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
                engine_ffi::coroutine::active_managed_coroutine_count(),
            ),
            #[cfg(not(feature = "subsystem-scripting-csharp"))]
            script_engine_state: format!(
                "coroutines={}",
                engine_ffi::coroutine::active_managed_coroutine_count()
            ),
        }
    }

    /// Render one frame and record GPU statistics into the diagnostics collector.
    ///
    /// Extraction reads transforms from the active ECS World so objects render
    /// at their correct position. A retained Scene snapshot without a World is
    /// rejected instead of silently rendering identity transforms.
    pub fn render_frame(&mut self, frame_index: u64) -> Result<FrameStats, Vec<Diagnostic>> {
        let input = if let Some(result) = self
            .world_slot
            .with_world(|world| extract_renderer_input_from_world(world, frame_index))
        {
            result?
        } else if self.scene.is_some() {
            return Err(vec![Diagnostic::new(
                "SC0019",
                DiagnosticSeverity::Error,
                "engine-core",
                "a scene snapshot exists without an active World; reload it through load_scene",
            )]);
        } else {
            return Err(vec![Diagnostic::new(
                "SC0018",
                DiagnosticSeverity::Error,
                "engine-core",
                "no scene is loaded",
            )]);
        };
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
    /// Dispatches completed async callbacks, advances native coroutine state,
    /// then calls `OnStart`/`OnUpdate(dt)` on every active script instance.
    /// Resulting script diagnostics are pushed into the collector.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn tick_scripts(&mut self, dt: f32) {
        engine_ffi::world_bridge::activate_coroutine_runtime(&self.world_slot);
        engine_ffi::r#async::dispatch_main_thread_callbacks();
        engine_ffi::coroutine::tick_managed_coroutines(dt);
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

fn scene_load_diagnostic(diagnostic: SceneLoadDiagnostic) -> Diagnostic {
    let (code, entity_id, component_type_id, storage_type_id) = match &diagnostic {
        SceneLoadDiagnostic::UnknownComponent {
            entity_id,
            component_type_id,
        } => ("SC0030", entity_id, component_type_id, None),
        SceneLoadDiagnostic::MissingDeserializeHook {
            entity_id,
            component_type_id,
        } => ("SC0031", entity_id, component_type_id, None),
        SceneLoadDiagnostic::StorageFactoryTypeMismatch {
            entity_id,
            component_type_id,
            storage_type_id,
        } => (
            "SC0032",
            entity_id,
            component_type_id,
            Some(storage_type_id),
        ),
        SceneLoadDiagnostic::StorageInsertTypeMismatch {
            entity_id,
            component_type_id,
        } => ("SC0033", entity_id, component_type_id, None),
    };

    let mut mapped = Diagnostic::new(
        code,
        DiagnosticSeverity::Error,
        "engine-core.scene-loader",
        diagnostic.to_string(),
    )
    .entity(entity_id.clone())
    .path(format!(
        "entities[{entity_id}].components[{component_type_id}]"
    ));
    mapped
        .fields
        .insert("component_type_id".to_string(), component_type_id.clone());
    if let Some(storage_type_id) = storage_type_id {
        mapped
            .fields
            .insert("storage_type_id".to_string(), storage_type_id.clone());
    }
    mapped
}

impl Drop for EngineRuntime {
    fn drop(&mut self) {
        // Only clear the process-wide binding when this runtime is still the
        // active one. A newer runtime must remain connected.
        engine_ffi::world_bridge::deactivate_world(&self.world_slot);
        engine_ffi::world_bridge::deactivate_coroutine_runtime(&self.world_slot);
        self.world_slot.clear();
    }
}

// ── Backend factory (feature-gated) ─────────────────────────────────────

/// Create a Vulkan backend renderer from raw window handles.
///
/// This is the engine-level entry point for Vulkan initialisation.
/// Callers in the sandbox / application layer call this function once
/// during startup and pass the returned [`BackendRenderer`] to
/// [`Renderer::set_backend`](engine_renderer::Renderer::set_backend).
#[cfg(feature = "backend-vulkan")]
pub fn create_vulkan_backend_renderer(
    display_handle: raw_window_handle::RawDisplayHandle,
    window_handle: raw_window_handle::RawWindowHandle,
    width: u32,
    height: u32,
    enable_validation: bool,
    cache_dir: Option<&std::path::Path>,
) -> Result<Box<dyn engine_renderer::BackendRenderer>, String> {
    let device = render_vulkan::device_impl::VulkanDevice::new(
        display_handle,
        window_handle,
        width,
        height,
        enable_validation,
        cache_dir,
    )
    .map_err(|e| format!("VulkanDevice creation failed: {e}"))?;
    Ok(Box::new(render_vulkan::scene_renderer::SceneRenderer::new(
        device, width, height,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    static FFI_WORLD_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn serial_ffi_world_test() -> std::sync::MutexGuard<'static, ()> {
        FFI_WORLD_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn insert_empty_component(scene: &mut Scene, type_id: &str) -> String {
        let entity = scene.entities.first_mut().expect("sample scene entity");
        entity.components.insert(
            type_id.to_string(),
            engine_scene::ComponentRecord {
                schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: std::collections::BTreeMap::new(),
            },
        );
        entity.persistent_id.clone()
    }

    struct AOnlyComponent;

    struct BOnlyComponent;

    impl engine_scene::Component for AOnlyComponent {
        const TYPE_ID: &'static str = "test.a_only";
    }

    impl engine_scene::Component for BOnlyComponent {
        const TYPE_ID: &'static str = "test.b_only";
    }

    fn a_only_storage() -> Box<dyn engine_scene::ComponentStorageDyn> {
        Box::new(engine_scene::SparseSet::<AOnlyComponent>::new())
    }

    fn b_only_storage() -> Box<dyn engine_scene::ComponentStorageDyn> {
        Box::new(engine_scene::SparseSet::<BOnlyComponent>::new())
    }

    fn serialize_a_only(
        _component: &dyn std::any::Any,
    ) -> std::collections::BTreeMap<String, engine_serialize::Value> {
        std::collections::BTreeMap::new()
    }

    fn deserialize_a_only(
        _fields: &std::collections::BTreeMap<String, engine_serialize::Value>,
    ) -> Box<dyn std::any::Any> {
        Box::new(AOnlyComponent)
    }

    fn deserialize_b_only(
        _fields: &std::collections::BTreeMap<String, engine_serialize::Value>,
    ) -> Box<dyn std::any::Any> {
        Box::new(BOnlyComponent)
    }

    fn register_a_only(registry: &mut ComponentRegistry) {
        registry
            .register(engine_scene::ComponentExtension {
                meta: engine_scene::ComponentMeta {
                    type_id: <AOnlyComponent as engine_scene::Component>::TYPE_ID,
                    display_name: "A Only",
                    schema_version: (0, 1, 0),
                    has_editor: false,
                    has_script_binding: true,
                },
                storage_factory: a_only_storage,
                serialize: Some(serialize_a_only),
                deserialize: Some(deserialize_a_only),
            })
            .expect("register A-only test extension");
    }

    fn register_b_only(registry: &mut ComponentRegistry) {
        registry
            .register(engine_scene::ComponentExtension {
                meta: engine_scene::ComponentMeta {
                    type_id: <BOnlyComponent as engine_scene::Component>::TYPE_ID,
                    display_name: "B Only",
                    schema_version: (0, 1, 0),
                    has_editor: false,
                    has_script_binding: true,
                },
                storage_factory: b_only_storage,
                serialize: Some(serialize_a_only),
                deserialize: Some(deserialize_b_only),
            })
            .expect("register B-only test extension");
    }

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
    fn runtime_builder_registers_character_extensions_by_default() {
        let builder = EngineRuntimeBuilder::default();
        assert!(builder
            .component_registry()
            .is_registered("engine.character_controller"));
    }

    #[cfg(feature = "gameplay")]
    #[test]
    fn runtime_builder_registers_physics_extensions_with_gameplay() {
        let builder = EngineRuntimeBuilder::default();
        assert!(builder
            .component_registry()
            .is_registered("engine.physics.rigid_body"));
        assert!(builder
            .component_registry()
            .is_registered("engine.physics.collider"));
        assert!(builder
            .component_registry()
            .is_registered("engine.physics.physics_material"));
    }

    #[test]
    fn runtime_builder_exposes_registry_before_build() {
        let mut builder = EngineRuntimeBuilder::default();
        register_a_only(builder.component_registry_mut());
        assert!(builder.component_registry().is_registered("test.a_only"));
    }

    #[test]
    fn ffi_component_table_changes_only_when_a_runtime_activates() {
        let _guard = serial_ffi_world_test();
        let mut builder = EngineRuntimeBuilder::default();
        register_a_only(builder.component_registry_mut());
        let mut runtime_a = builder.build();
        runtime_a.set_world(World::new());

        let a_only = engine_ffi::component::lookup_component_type("A Only")
            .expect("A-only extension should be exposed while A is active");
        assert_eq!(
            engine_ffi::component::lookup_engine_type_id(a_only),
            Some("test.a_only")
        );
        let character = engine_ffi::component::lookup_component_type("Character Controller")
            .expect("character extension should be exposed to FFI");
        assert_eq!(
            engine_ffi::component::lookup_engine_type_id(character),
            Some("engine.character_controller")
        );

        // Merely constructing B must not mutate A's active type table.
        let mut runtime_b = EngineRuntime::new(EngineConfig::default());
        assert!(engine_ffi::component::lookup_component_type("A Only").is_some());

        // Activating B atomically replaces both the slot and type table.
        runtime_b.set_world(World::new());
        assert!(engine_ffi::component::lookup_component_type("A Only").is_none());
        assert!(engine_ffi::component::lookup_component_type("Character Controller").is_some());

        // Core metadata currently has no serialise/deserialise hooks, so it
        // must not be advertised as an FFI-readable component.
        assert!(engine_ffi::component::lookup_component_type("Transform").is_none());
    }

    #[test]
    fn ffi_component_ids_are_stable_across_active_registry_order_and_membership() {
        let _guard = serial_ffi_world_test();
        let mut first_builder = EngineRuntimeBuilder::default();
        register_a_only(first_builder.component_registry_mut());
        register_b_only(first_builder.component_registry_mut());
        let mut first = first_builder.build();
        first.set_world(World::new());
        let a_id = engine_ffi::component::lookup_component_type("A Only").expect("A ID");
        let b_id = engine_ffi::component::lookup_component_type("B Only").expect("B ID");

        let mut reordered_builder = EngineRuntimeBuilder::default();
        register_b_only(reordered_builder.component_registry_mut());
        register_a_only(reordered_builder.component_registry_mut());
        let mut reordered = reordered_builder.build();
        reordered.set_world(World::new());
        assert_eq!(
            engine_ffi::component::lookup_component_type("A Only"),
            Some(a_id)
        );
        assert_eq!(
            engine_ffi::component::lookup_component_type("B Only"),
            Some(b_id)
        );

        let mut b_only_builder = EngineRuntimeBuilder::default();
        register_b_only(b_only_builder.component_registry_mut());
        let mut b_only = b_only_builder.build();
        b_only.set_world(World::new());
        assert!(engine_ffi::component::lookup_component_type("A Only").is_none());
        assert!(engine_ffi::component::lookup_engine_type_id(a_id).is_none());
        assert_eq!(
            engine_ffi::component::lookup_engine_type_id(b_id),
            Some("test.b_only")
        );
    }

    #[cfg(feature = "gameplay")]
    #[test]
    fn gameplay_physics_extensions_are_exposed_to_ffi() {
        let _guard = serial_ffi_world_test();
        let mut runtime = EngineRuntime::new(EngineConfig::default());
        runtime.set_world(World::new());
        let rigid_body = engine_ffi::component::lookup_component_type("RigidBody")
            .expect("physics extension should be exposed to FFI");
        assert_eq!(
            engine_ffi::component::lookup_engine_type_id(rigid_body),
            Some("engine.physics.rigid_body")
        );
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
        assert!(
            rd.script_engine_state.contains("coroutines=0"),
            "missing coroutines=0"
        );
        assert!(rd.reload_queue.is_none());
    }

    #[test]
    fn strict_scene_load_installs_the_runtime_registry() {
        let _guard = serial_ffi_world_test();
        let mut runtime = EngineRuntime::new(EngineConfig::default());
        let runtime_registry = std::sync::Arc::clone(runtime.component_registry());

        runtime
            .load_scene_to_world(engine_scene::sample_scene())
            .expect("sample scene should load");

        assert_eq!(
            runtime.with_world(|world| {
                std::sync::Arc::ptr_eq(
                    world.component_registry().expect("world registry"),
                    &runtime_registry,
                )
            }),
            Some(true)
        );
    }

    #[test]
    fn unknown_component_failure_keeps_active_world_and_scene() {
        let _guard = serial_ffi_world_test();
        let mut runtime = EngineRuntime::new(EngineConfig::default());
        let mut old_world = World::new();
        old_world.create_entity();
        runtime.set_world(old_world);
        let old_scene = runtime.scene_ref().cloned().expect("old scene snapshot");

        let mut invalid_scene = engine_scene::sample_scene();
        let entity_id = insert_empty_component(&mut invalid_scene, "third.party.missing");
        let diagnostics = runtime
            .load_scene_to_world(invalid_scene)
            .expect_err("unknown component must fail strict loading");

        assert_eq!(runtime.with_world(World::alive_count), Some(1));
        assert_eq!(runtime.scene_ref(), Some(&old_scene));
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "SC0030")
            .expect("mapped unknown-component diagnostic");
        assert_eq!(diagnostic.entity.as_deref(), Some(entity_id.as_str()));
        assert_eq!(
            diagnostic
                .fields
                .get("component_type_id")
                .map(String::as_str),
            Some("third.party.missing")
        );
        assert_eq!(
            diagnostic.path.as_deref(),
            Some(format!("entities[{entity_id}].components[third.party.missing]").as_str())
        );

        // The process-wide FFI bridge must still target the previous World.
        let spawned = engine_ffi::world_bridge::entity_spawn();
        assert_ne!(spawned, engine_ffi::types::FfiEntityId::INVALID);
        assert_eq!(runtime.with_world(World::alive_count), Some(2));
    }

    #[test]
    fn validation_failures_keep_active_world_and_scene() {
        let _guard = serial_ffi_world_test();
        let mut runtime = EngineRuntime::new(EngineConfig::default());
        let mut old_world = World::new();
        old_world.create_entity();
        runtime.set_world(old_world);
        let old_scene = runtime.scene_ref().cloned().expect("old scene snapshot");

        let mut duplicate = engine_scene::sample_scene();
        duplicate.entities.push(duplicate.entities[0].clone());
        let duplicate_diagnostics = runtime
            .load_scene_to_world(duplicate)
            .expect_err("duplicate entity must fail validation");
        assert!(duplicate_diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SC0015"));
        assert_eq!(runtime.with_world(World::alive_count), Some(1));
        assert_eq!(runtime.scene_ref(), Some(&old_scene));

        let mut missing_parent = engine_scene::sample_scene();
        let mut orphan = missing_parent.entities[0].clone();
        orphan.persistent_id = "orphan".to_string();
        orphan.parent = Some("missing-parent".to_string());
        missing_parent.entities.push(orphan);
        let parent_diagnostics = runtime
            .load_scene_to_world(missing_parent)
            .expect_err("missing parent must fail validation");
        assert!(parent_diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SC0016"));
        assert_eq!(runtime.with_world(World::alive_count), Some(1));
        assert_eq!(runtime.scene_ref(), Some(&old_scene));
    }

    #[test]
    fn set_world_installs_runtime_registry_when_missing() {
        let _guard = serial_ffi_world_test();
        let mut runtime = EngineRuntime::new(EngineConfig::default());
        let runtime_registry = std::sync::Arc::clone(runtime.component_registry());

        runtime.set_world(World::new());

        assert_eq!(
            runtime.with_world(|world| {
                std::sync::Arc::ptr_eq(
                    world.component_registry().expect("world registry"),
                    &runtime_registry,
                )
            }),
            Some(true)
        );
    }

    #[test]
    fn set_world_preserves_an_existing_registry() {
        let _guard = serial_ffi_world_test();
        let mut runtime = EngineRuntime::new(EngineConfig::default());
        let mut custom_registry = ComponentRegistry::new();
        register_a_only(&mut custom_registry);
        let custom_registry = std::sync::Arc::new(custom_registry);
        let mut world = World::new();
        world.set_shared_component_registry(std::sync::Arc::clone(&custom_registry));

        runtime.set_world(world);

        assert_eq!(
            runtime.with_world(|world| {
                std::sync::Arc::ptr_eq(
                    world.component_registry().expect("world registry"),
                    &custom_registry,
                )
            }),
            Some(true)
        );
        assert!(engine_ffi::component::lookup_component_type("A Only").is_some());
        assert!(engine_ffi::component::lookup_component_type("Character Controller").is_none());
    }

    #[test]
    fn engine_runtime_can_replace_the_active_world_repeatedly() {
        let _guard = serial_ffi_world_test();
        let mut runtime = EngineRuntime::new(EngineConfig::default());

        let mut first = World::new();
        first.create_entity();
        runtime.set_world(first);

        runtime.set_world(World::new());
        let spawned = engine_ffi::world_bridge::entity_spawn();
        assert_ne!(spawned, engine_ffi::types::FfiEntityId::INVALID);
        assert_eq!(runtime.with_world(World::alive_count), Some(1));
    }

    #[test]
    fn moving_runtime_keeps_ffi_world_binding_valid() {
        let _guard = serial_ffi_world_test();
        let mut runtime = EngineRuntime::new(EngineConfig::default());
        runtime.set_world(World::new());

        let mut runtimes = vec![runtime];
        let moved_runtime = runtimes.pop().expect("moved runtime");

        let spawned = engine_ffi::world_bridge::entity_spawn();
        assert_ne!(spawned, engine_ffi::types::FfiEntityId::INVALID);
        assert_eq!(moved_runtime.with_world(World::alive_count), Some(1));
    }

    #[test]
    fn dropping_runtime_makes_its_ffi_world_unavailable() {
        let _guard = serial_ffi_world_test();
        {
            let mut runtime = EngineRuntime::new(EngineConfig::default());
            runtime.set_world(World::new());
        }

        assert_eq!(
            engine_ffi::world_bridge::entity_spawn(),
            engine_ffi::types::FfiEntityId::INVALID
        );
    }

    #[test]
    fn dropping_old_runtime_does_not_deactivate_new_runtime() {
        let _guard = serial_ffi_world_test();
        let mut old_runtime = EngineRuntime::new(EngineConfig::default());
        old_runtime.set_world(World::new());
        let mut current_runtime = EngineRuntime::new(EngineConfig::default());
        current_runtime.set_world(World::new());

        drop(old_runtime);
        let spawned = engine_ffi::world_bridge::entity_spawn();
        assert_ne!(spawned, engine_ffi::types::FfiEntityId::INVALID);
        assert_eq!(current_runtime.with_world(World::alive_count), Some(1));
    }

    #[test]
    fn compatibility_scene_load_replaces_and_activates_the_world() {
        let _guard = serial_ffi_world_test();
        let mut runtime = EngineRuntime::new(EngineConfig::default());
        runtime.set_world(World::new());
        runtime
            .load_scene(engine_scene::sample_scene())
            .expect("sample scene should load into a World");

        assert!(runtime.has_world());
        assert_ne!(
            engine_ffi::world_bridge::entity_spawn(),
            engine_ffi::types::FfiEntityId::INVALID
        );
    }

    // ── Script subsystem tests ──────────────────────────────────────────

    #[cfg(feature = "subsystem-scripting-csharp")]
    #[test]
    fn in_process_csharp_bridge_installs_the_native_cdylib() {
        let _guard = serial_ffi_world_test();
        let mut runtime = EngineRuntime::new(EngineConfig::default());
        runtime.set_world(World::new());

        runtime
            .install_in_process_csharp_ffi()
            .expect("matching engine_ffi cdylib should install");

        let path =
            engine_ffi::host_bridge::loaded_cdylib_path().expect("installed native library path");
        assert!(path.exists());
        assert_eq!(
            std::env::var("ENGINE_FFI_HOST_PID").ok(),
            Some(std::process::id().to_string())
        );
    }

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
        let _guard = serial_ffi_world_test();
        use engine_scene::ComponentRecord;
        use engine_script::MockHost;
        use engine_serialize::SchemaVersion;
        use std::collections::BTreeMap;

        let config = EngineConfig::default();
        let mut runtime = EngineRuntime::new(config);
        runtime.register_script_host(Box::new(MockHost::new()));
        // Match the host name used by MockHost
        runtime.set_script_host_name("mock");

        // Create a minimal scene with a script component
        let mut script_fields = BTreeMap::new();
        script_fields.insert(
            "assembly_id".into(),
            engine_serialize::Value::Str("asm".into()),
        );
        script_fields.insert(
            "class_name".into(),
            engine_serialize::Value::Str("MyScript".into()),
        );

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
        runtime
            .load_scene_to_world(scene)
            .expect("engine.script metadata should be allowed");

        // After load_scene, the script engine should have an instance
        assert_eq!(runtime.script_engine.host_count(), 1);
        let after = runtime.script_engine.managers()[0].instance_count();
        assert_eq!(after, 1, "script instance should have been created");

        // Tick should not produce errors
        runtime.tick_scripts(0.016);
    }

    #[cfg(feature = "subsystem-scripting-csharp")]
    #[test]
    fn script_feature_does_not_ignore_other_unknown_component_types() {
        let _guard = serial_ffi_world_test();
        let mut runtime = EngineRuntime::new(EngineConfig::default());
        let mut scene = engine_scene::sample_scene();
        insert_empty_component(&mut scene, "engine.script::assembly");

        let diagnostics = runtime
            .load_scene_to_world(scene)
            .expect_err("only the exact engine.script type is scene-only");

        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "SC0030"
                && diagnostic
                    .fields
                    .get("component_type_id")
                    .is_some_and(|type_id| type_id == "engine.script::assembly")
        }));
        assert!(!runtime.has_world());
        assert!(runtime.scene_ref().is_none());
    }
}
