#![forbid(unsafe_code)]

pub mod diagnostics;
pub use diagnostics::*;
pub mod cooked_assets;
pub use cooked_assets::*;

use engine_asset::{AssetHandle, AssetRegistry};
use engine_renderer::{
    AssetId, DebugDrawRegistry, FrameStats, MaterialUpload, MeshUpload, MeshVertexFormat,
    RenderExtensionRegistry, Renderer, TextureUpload,
};
use engine_scene::{
    extract_renderer_input_from_world, validate_scene, AssetTypeRegistry, ComponentRegistry, Scene,
    SceneLoadDiagnostic, World, WorldSlot,
};
use engine_serialize::{Diagnostic, DiagnosticSeverity};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

pub mod ffi_init;
pub mod game_loop;
#[cfg(feature = "runtime-subsystems")]
pub use game_loop::RuntimeUiEvent;

// ── Optional script subsystem ─────────────────────────────────────────────

#[cfg(feature = "subsystem-scripting-csharp")]
pub mod script;
#[cfg(feature = "subsystem-scripting-csharp")]
use engine_script::{
    GameplayCommand, GameplayContext, GameplayEntitySnapshot, GameplayInputTransitions,
    GameplayInputValue, GameplayPhysicsEvent, GameplayUiEvent, ScriptEngine, ScriptError,
    ScriptHost, ScriptTransform,
};
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

/// Deferred request emitted by a game script and consumed by the project
/// scene manager at a safe frame boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SceneLoadRequest {
    pub scene_id: String,
    pub requested_by: String,
}

/// Configures an [`EngineRuntime`] before its shared component registry is
/// frozen behind an [`Arc`].
///
/// Character-controller components are always registered. Physics components
/// are additionally registered when the `gameplay` feature is enabled. The
/// `runtime-subsystems` feature installs the existing UI, audio, animation, and
/// navigation extensions on the same registries used by strict scene loading
/// and rendering. Hosts can add further extensions before calling
/// [`build`](Self::build).
pub struct EngineRuntimeBuilder {
    config: EngineConfig,
    component_registry: ComponentRegistry,
    asset_type_registry: AssetTypeRegistry,
    render_extension_registry: RenderExtensionRegistry,
    debug_draw_registry: DebugDrawRegistry,
    #[cfg(feature = "runtime-subsystems")]
    animation_extensions: engine_animation::AnimationExtensionHandles,
}

impl EngineRuntimeBuilder {
    pub fn new(config: EngineConfig) -> Self {
        let mut component_registry = ComponentRegistry::new();
        #[cfg(feature = "runtime-subsystems")]
        let mut asset_type_registry = AssetTypeRegistry::new();
        #[cfg(not(feature = "runtime-subsystems"))]
        let asset_type_registry = AssetTypeRegistry::new();
        #[cfg(feature = "runtime-subsystems")]
        let mut render_extension_registry = RenderExtensionRegistry::new();
        #[cfg(not(feature = "runtime-subsystems"))]
        let render_extension_registry = RenderExtensionRegistry::new();
        let mut debug_draw_registry = DebugDrawRegistry::new();
        component_registry.register_core();
        engine_character::register_character_extensions(
            &mut component_registry,
            Some(&mut debug_draw_registry),
        );
        #[cfg(feature = "gameplay")]
        engine_physics::register_physics_extensions(
            &mut component_registry,
            Some(&mut debug_draw_registry),
        );

        #[cfg(feature = "runtime-subsystems")]
        engine_ui::register_ui_extensions(&mut component_registry);
        #[cfg(feature = "runtime-subsystems")]
        engine_audio::register_audio_extensions(&mut component_registry, &mut asset_type_registry);
        #[cfg(feature = "runtime-subsystems")]
        let animation_extensions = engine_animation::register_animation_extensions(
            &mut component_registry,
            &mut asset_type_registry,
            &mut render_extension_registry,
            &mut debug_draw_registry,
        );
        #[cfg(feature = "runtime-subsystems")]
        engine_nav::register_nav_extensions(
            &mut component_registry,
            Some(&mut debug_draw_registry),
            &mut asset_type_registry,
        );

        Self {
            config,
            component_registry,
            asset_type_registry,
            render_extension_registry,
            debug_draw_registry,
            #[cfg(feature = "runtime-subsystems")]
            animation_extensions,
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

    /// Inspect registered asset cook/load extensions.
    pub fn asset_type_registry(&self) -> &AssetTypeRegistry {
        &self.asset_type_registry
    }

    /// Register an additional asset type before building the runtime.
    pub fn asset_type_registry_mut(&mut self) -> &mut AssetTypeRegistry {
        &mut self.asset_type_registry
    }

    /// Inspect render-data producers that will run before every frame draw.
    pub fn render_extension_registry(&self) -> &RenderExtensionRegistry {
        &self.render_extension_registry
    }

    /// Register an additional render-data producer before building the runtime.
    pub fn render_extension_registry_mut(&mut self) -> &mut RenderExtensionRegistry {
        &mut self.render_extension_registry
    }

    /// Inspect subsystem debug-draw providers retained by the runtime.
    pub fn debug_draw_registry(&self) -> &DebugDrawRegistry {
        &self.debug_draw_registry
    }

    /// Register an additional debug-draw provider before building the runtime.
    pub fn debug_draw_registry_mut(&mut self) -> &mut DebugDrawRegistry {
        &mut self.debug_draw_registry
    }

    /// Shared animation queues used by the later animation update stage.
    #[cfg(feature = "runtime-subsystems")]
    pub fn animation_extension_handles(&self) -> &engine_animation::AnimationExtensionHandles {
        &self.animation_extensions
    }

    pub fn build(self) -> EngineRuntime {
        EngineRuntime::from_parts(
            self.config,
            Arc::new(self.component_registry),
            self.asset_type_registry,
            self.render_extension_registry,
            self.debug_draw_registry,
            #[cfg(feature = "runtime-subsystems")]
            self.animation_extensions,
        )
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
    asset_registry: AssetRegistry,
    loaded_cooked_asset_ids: BTreeSet<AssetId>,
    loaded_extension_asset_ids: BTreeMap<String, BTreeSet<AssetId>>,
    scene: Option<Scene>,
    world_slot: WorldSlot,
    component_registry: Arc<ComponentRegistry>,
    asset_type_registry: AssetTypeRegistry,
    render_extension_registry: RenderExtensionRegistry,
    debug_draw_registry: DebugDrawRegistry,
    #[cfg(feature = "runtime-subsystems")]
    animation_extensions: engine_animation::AnimationExtensionHandles,
    collector: DiagnosticsCollector,
    #[cfg(feature = "subsystem-scripting-csharp")]
    script_engine: ScriptEngine,
    /// Name of the script host to use when loading scene scripts.
    #[cfg(feature = "subsystem-scripting-csharp")]
    script_host_name: String,
    /// Last resolved project input snapshot, also used for OnCreate when a
    /// GameLoop loads a scene before its first update.
    #[cfg(feature = "subsystem-scripting-csharp")]
    script_input_actions: std::collections::BTreeMap<String, GameplayInputValue>,
    #[cfg(feature = "subsystem-scripting-csharp")]
    pending_scene_request: Option<SceneLoadRequest>,
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

    fn from_parts(
        config: EngineConfig,
        component_registry: Arc<ComponentRegistry>,
        asset_type_registry: AssetTypeRegistry,
        render_extension_registry: RenderExtensionRegistry,
        debug_draw_registry: DebugDrawRegistry,
        #[cfg(feature = "runtime-subsystems")]
        animation_extensions: engine_animation::AnimationExtensionHandles,
    ) -> Self {
        // Initialise the FFI callback registry so extern "C" entry points
        // can dispatch to real implementations immediately. The active world
        // slot is selected later when a scene is loaded.
        ffi_init::initialise();

        let mut asset_registry = AssetRegistry::new();
        install_builtin_render_assets(&mut asset_registry);

        Self {
            config,
            renderer: Renderer::new(),
            asset_registry,
            loaded_cooked_asset_ids: BTreeSet::new(),
            loaded_extension_asset_ids: BTreeMap::new(),
            scene: None,
            world_slot: WorldSlot::new(),
            component_registry,
            asset_type_registry,
            render_extension_registry,
            debug_draw_registry,
            #[cfg(feature = "runtime-subsystems")]
            animation_extensions,
            collector: DiagnosticsCollector::new(),
            #[cfg(feature = "subsystem-scripting-csharp")]
            script_engine: ScriptEngine::new(),
            #[cfg(feature = "subsystem-scripting-csharp")]
            script_host_name: "dotnet".to_string(),
            #[cfg(feature = "subsystem-scripting-csharp")]
            script_input_actions: std::collections::BTreeMap::new(),
            #[cfg(feature = "subsystem-scripting-csharp")]
            pending_scene_request: None,
        }
    }

    pub fn config(&self) -> &EngineConfig {
        &self.config
    }

    /// Shared registry used by strict scene loading and registry-less worlds.
    pub fn component_registry(&self) -> &Arc<ComponentRegistry> {
        &self.component_registry
    }

    /// Asset-type extensions installed for this runtime configuration.
    pub fn asset_type_registry(&self) -> &AssetTypeRegistry {
        &self.asset_type_registry
    }

    /// Retrieve an extension-owned cooked asset from the shared typed cache.
    ///
    /// The type ID prevents callers from accidentally treating a built-in
    /// render asset with the same ID as a subsystem asset. The concrete type
    /// must match the value returned by that extension's registered loader.
    pub fn extension_asset<T: Send + Sync + 'static>(
        &self,
        type_id: &str,
        id: &AssetId,
    ) -> Option<AssetHandle<T>> {
        if !self.loaded_extension_asset_ids.get(type_id)?.contains(id) {
            return None;
        }
        self.asset_registry.get::<T>(id)
    }

    /// Number of currently installed cooked assets owned by an extension.
    pub fn extension_asset_count(&self, type_id: &str) -> usize {
        self.loaded_extension_asset_ids
            .get(type_id)
            .map_or(0, BTreeSet::len)
    }

    /// Producers invoked after ECS extraction and before asset synchronisation.
    pub fn render_extension_registry(&self) -> &RenderExtensionRegistry {
        &self.render_extension_registry
    }

    /// Debug providers retained for tooling and a future frame debug pass.
    pub fn debug_draw_registry(&self) -> &DebugDrawRegistry {
        &self.debug_draw_registry
    }

    /// Shared animation queues used by the animation update stage.
    #[cfg(feature = "runtime-subsystems")]
    pub fn animation_extension_handles(&self) -> &engine_animation::AnimationExtensionHandles {
        &self.animation_extensions
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

        #[cfg(feature = "subsystem-scripting-csharp")]
        self.clear_scene_script_instances();

        self.world_slot.replace(world);
        engine_ffi::world_bridge::activate_world(&self.world_slot, &self.component_registry);

        // Attach scripts only after activating the new world so managed
        // OnCreate callbacks cannot observe the previous scene.
        #[cfg(feature = "subsystem-scripting-csharp")]
        {
            self.pending_scene_request = None;
            self.attach_scene_scripts(&scene);
        }

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

        #[cfg(feature = "subsystem-scripting-csharp")]
        self.clear_scene_script_instances();

        self.world_slot.replace(world);
        engine_ffi::world_bridge::activate_world(&self.world_slot, &effective_registry);

        #[cfg(feature = "subsystem-scripting-csharp")]
        {
            self.pending_scene_request = None;
            self.attach_scene_scripts(&scene);
        }

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

    /// Runtime asset cache used by automatic renderer-resource synchronisation.
    pub fn asset_registry(&self) -> &AssetRegistry {
        &self.asset_registry
    }

    /// Mutable runtime asset cache. Replacing a typed render upload here is
    /// observed on the next frame and propagated to the active GPU backend.
    pub fn asset_registry_mut(&mut self) -> &mut AssetRegistry {
        &mut self.asset_registry
    }

    /// Register or replace a GPU-ready mesh asset.
    pub fn register_mesh_asset(&mut self, upload: MeshUpload) -> AssetHandle<MeshUpload> {
        let id = upload.mesh_id.clone();
        self.asset_registry.insert_typed(id, upload)
    }

    /// Register or replace a GPU-ready texture asset.
    pub fn register_texture_asset(&mut self, upload: TextureUpload) -> AssetHandle<TextureUpload> {
        let id = upload.texture_id.clone();
        self.asset_registry.insert_typed(id, upload)
    }

    /// Register or replace a GPU-ready material asset.
    pub fn register_material_asset(
        &mut self,
        upload: MaterialUpload,
    ) -> AssetHandle<MaterialUpload> {
        let id = upload.material_id.clone();
        self.asset_registry.insert_typed(id, upload)
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
        self.render_frame_with_ui(frame_index, Vec::new())
    }

    /// Render one frame and append caller-produced UI batches to the extracted
    /// scene input.
    ///
    /// Tooling and game hosts build immediate-mode UI outside the ECS world.
    /// This entry point keeps that UI on the same validated renderer path as
    /// the 3D scene instead of requiring callers to bypass [`EngineRuntime`].
    pub fn render_frame_with_ui(
        &mut self,
        frame_index: u64,
        ui_batches: Vec<engine_renderer::UiBatch>,
    ) -> Result<FrameStats, Vec<Diagnostic>> {
        let mut input = if let Some(result) = self
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
        self.render_extension_registry
            .produce_all(&mut input, frame_index);
        input.ui_batches.extend(ui_batches);
        if let Err(diagnostics) = self.sync_render_assets(&input) {
            self.collector.push_asset_diags(diagnostics.clone());
            return Err(diagnostics);
        }
        let result = self.renderer.draw_scene(&input);
        if let Ok(stats) = &result {
            self.collector.record_frame(frame_index, stats);
        }
        result
    }

    fn sync_render_assets(
        &mut self,
        input: &engine_renderer::RenderFrameInput,
    ) -> Result<(), Vec<Diagnostic>> {
        let mut material_ids = std::collections::BTreeMap::new();
        let mut mesh_ids = std::collections::BTreeMap::new();
        for drawable in &input.drawables {
            material_ids.insert(drawable.material.id.clone(), drawable.material.clone());
            mesh_ids.insert(drawable.mesh.id.clone(), drawable.mesh.clone());
        }
        for item in &input.skinned_items {
            material_ids.insert(item.material.id.clone(), item.material.clone());
            mesh_ids.insert(item.mesh.id.clone(), item.mesh.clone());
        }

        let mut materials = Vec::new();
        let mut texture_ids = std::collections::BTreeMap::new();
        for id in material_ids.values() {
            let Some(handle) = self.asset_registry.get::<MaterialUpload>(id) else {
                // A backend-provided or manually uploaded material remains a
                // valid source; Vulkan also has an explicit fallback material.
                continue;
            };
            let upload = handle.get().clone();
            validate_registered_asset_id("material", id, &upload.material_id)?;
            if let Some(texture) = &upload.base_color_texture {
                texture_ids.insert(texture.id.clone(), texture.clone());
            }
            materials.push(upload);
        }
        for batch in &input.ui_batches {
            if let Some(texture) = &batch.texture {
                texture_ids.insert(texture.id.clone(), texture.clone());
            }
        }

        let mut textures = Vec::new();
        for id in texture_ids.values() {
            let Some(handle) = self.asset_registry.get::<TextureUpload>(id) else {
                // The texture may have been uploaded directly by an embedding
                // application; the backend validates that dependency.
                continue;
            };
            let upload = handle.get().clone();
            validate_registered_asset_id("texture", id, &upload.texture_id)?;
            textures.push(upload);
        }

        let mut meshes = Vec::new();
        for id in mesh_ids.values() {
            let Some(handle) = self.asset_registry.get::<MeshUpload>(id) else {
                // Preserve explicit low-level uploads. Missing meshes still
                // fail closed in the backend before command recording.
                continue;
            };
            let upload = handle.get().clone();
            validate_registered_asset_id("mesh", id, &upload.mesh_id)?;
            meshes.push(upload);
        }

        for upload in textures {
            let receipt = self.renderer.upload_texture(upload)?;
            self.collector.push_asset_diags(receipt.warnings);
        }
        for upload in materials {
            let receipt = self.renderer.upload_material(upload)?;
            self.collector.push_asset_diags(receipt.warnings);
        }
        for upload in meshes {
            let receipt = self.renderer.upload_mesh(upload)?;
            self.collector.push_asset_diags(receipt.warnings);
        }
        Ok(())
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

    /// Atomically replace the complete script runtime after a caller has
    /// prepared its host and assemblies in isolation.
    ///
    /// The candidate must contain exactly one host with `host_name`.  This is
    /// checked before the active engine is touched, so a malformed candidate
    /// leaves the previous host, assemblies, and instances available.  A
    /// successful replacement cannot accumulate duplicate hosts from prior
    /// reloads because the complete [`ScriptEngine`] is swapped at once.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn replace_script_engine(
        &mut self,
        candidate: ScriptEngine,
        host_name: impl Into<String>,
    ) -> Result<(), ScriptError> {
        let host_name = host_name.into();
        let matching_hosts = candidate
            .managers()
            .iter()
            .filter(|manager| manager.host_name == host_name)
            .count();
        if matching_hosts != 1 {
            return Err(ScriptError::HostError(format!(
                "replacement script engine must contain exactly one host named '{host_name}', found {matching_hosts}"
            )));
        }

        self.clear_scene_script_instances();
        self.script_engine = candidate;
        self.script_host_name = host_name;
        self.pending_scene_request = None;
        Ok(())
    }

    /// Set the script host name used for scene-attached scripts.
    ///
    /// Must match the [`name`](ScriptHost::name) of a registered host.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn set_script_host_name(&mut self, name: impl Into<String>) {
        self.script_host_name = name.into();
    }

    /// Store the resolved input values used by the next script lifecycle call.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn set_script_input_actions(
        &mut self,
        input_actions: std::collections::BTreeMap<String, GameplayInputValue>,
    ) {
        self.script_input_actions = input_actions;
    }

    /// Take the next deferred script scene-load request, if scripting is
    /// enabled and a script emitted one during OnCreate/OnUpdate.
    pub fn take_pending_scene_request(&mut self) -> Option<SceneLoadRequest> {
        #[cfg(feature = "subsystem-scripting-csharp")]
        {
            self.pending_scene_request.take()
        }
        #[cfg(not(feature = "subsystem-scripting-csharp"))]
        {
            None
        }
    }

    /// Tick all scripts — call this each frame before `render_frame`.
    ///
    /// Dispatches completed async callbacks, advances native coroutine state,
    /// then calls `OnStart`/`OnUpdate(dt)` on every active script instance.
    /// Resulting script diagnostics are pushed into the collector.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn tick_scripts(&mut self, dt: f32) {
        self.tick_scripts_with_input(dt, &std::collections::BTreeMap::new());
    }

    /// Tick scripts with the resolved project input snapshot for this frame.
    ///
    /// Process hosts receive entity Transform and input data before lifecycle
    /// methods run. Their queued Transform writes are validated and committed
    /// to the ECS world after every script has completed its update.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn tick_scripts_with_input(
        &mut self,
        dt: f32,
        input_actions: &std::collections::BTreeMap<String, GameplayInputValue>,
    ) {
        self.tick_scripts_with_input_and_physics(
            dt,
            input_actions,
            &std::collections::BTreeMap::new(),
        );
    }

    /// Tick scripts with input and entity-relative physics events.
    ///
    /// Physics events are frame snapshots. Callers must pass an empty map on
    /// frames without a physics step so stale contacts cannot be observed.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn tick_scripts_with_input_and_physics(
        &mut self,
        dt: f32,
        input_actions: &std::collections::BTreeMap<String, GameplayInputValue>,
        physics_events: &std::collections::BTreeMap<String, Vec<GameplayPhysicsEvent>>,
    ) {
        self.tick_scripts_with_frame_input(
            dt,
            input_actions,
            &GameplayInputTransitions::default(),
            physics_events,
        );
    }

    /// Tick scripts with the complete resolved frame-input snapshot.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn tick_scripts_with_frame_input(
        &mut self,
        dt: f32,
        input_actions: &std::collections::BTreeMap<String, GameplayInputValue>,
        input_transitions: &GameplayInputTransitions,
        physics_events: &std::collections::BTreeMap<String, Vec<GameplayPhysicsEvent>>,
    ) {
        self.tick_scripts_with_frame_input_and_ui(
            dt,
            input_actions,
            input_transitions,
            physics_events,
            &[],
        );
    }

    /// Tick scripts with the complete frame snapshot, including retained UI
    /// clicks drained by the owning [`GameLoop`](crate::game_loop::GameLoop).
    /// The same immutable event slice is copied into every script context.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn tick_scripts_with_frame_input_and_ui(
        &mut self,
        dt: f32,
        input_actions: &std::collections::BTreeMap<String, GameplayInputValue>,
        input_transitions: &GameplayInputTransitions,
        physics_events: &std::collections::BTreeMap<String, Vec<GameplayPhysicsEvent>>,
        ui_events: &[GameplayUiEvent],
    ) {
        self.script_input_actions.clone_from(input_actions);
        engine_ffi::world_bridge::activate_coroutine_runtime(&self.world_slot);
        engine_ffi::r#async::dispatch_main_thread_callbacks();
        engine_ffi::coroutine::tick_managed_coroutines(dt);
        let contexts = self.script_gameplay_contexts(
            input_actions,
            input_transitions,
            physics_events,
            ui_events,
        );
        let mut diagnostics = self.script_engine.set_gameplay_contexts(&contexts);
        diagnostics.extend(self.script_engine.update(dt));
        let (commands, command_diagnostics) = self.script_engine.drain_gameplay_commands();
        diagnostics.extend(command_diagnostics);
        diagnostics.extend(self.apply_script_gameplay_commands(commands));
        self.collector.push_script_diags(diagnostics);
    }

    // ── Private helpers ─────────────────────────────────────────────────

    /// Destroy and remove all script instances attached to the active scene.
    #[cfg(feature = "subsystem-scripting-csharp")]
    fn clear_scene_script_instances(&mut self) {
        // OnDestroy must run while the previous World is still active.  The
        // manager then needs an explicit clear because destroy() deliberately
        // preserves instance records for lifecycle inspection.
        let destroy_diags = self.script_engine.destroy_instances();
        self.collector.push_script_diags(destroy_diags);
        for manager in self.script_engine.managers_mut() {
            manager.clear();
        }
    }

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

        // OnCreate receives the owning entity's Transform. Input actions are
        // frame data and start empty until GameLoop performs its first update.
        let contexts = self.script_gameplay_contexts(
            &self.script_input_actions,
            &GameplayInputTransitions::default(),
            &std::collections::BTreeMap::new(),
            &[],
        );
        let context_diags = self.script_engine.set_gameplay_contexts(&contexts);
        self.collector.push_script_diags(context_diags);

        // Call OnCreate on all newly-attached instances
        let create_diags = self.script_engine.create_instances();
        self.collector.push_script_diags(create_diags);

        // OnCreate may change Transform. Commit those commands immediately so
        // the first-frame context does not overwrite the managed change.
        let (commands, mut command_diags) = self.script_engine.drain_gameplay_commands();
        command_diags.extend(self.apply_script_gameplay_commands(commands));
        self.collector.push_script_diags(command_diags);
    }

    #[cfg(feature = "subsystem-scripting-csharp")]
    fn script_gameplay_contexts(
        &self,
        input_actions: &std::collections::BTreeMap<String, GameplayInputValue>,
        input_transitions: &GameplayInputTransitions,
        physics_events: &std::collections::BTreeMap<String, Vec<GameplayPhysicsEvent>>,
        ui_events: &[GameplayUiEvent],
    ) -> std::collections::BTreeMap<String, GameplayContext> {
        let entity_ids = self
            .script_engine
            .managers()
            .iter()
            .flat_map(|manager| manager.iter_instances().map(|(entity_id, _, _)| entity_id))
            .map(str::to_owned)
            .collect::<std::collections::BTreeSet<_>>();

        let entities = self.script_gameplay_entity_snapshots();

        entity_ids
            .into_iter()
            .map(|entity_id| {
                let context = GameplayContext {
                    transform: entities
                        .get(&entity_id)
                        .and_then(|snapshot| snapshot.transform.clone()),
                    entity_id: entity_id.clone(),
                    input_actions: input_actions.clone(),
                    input_transitions: input_transitions.clone(),
                    physics_events: physics_events.get(&entity_id).cloned().unwrap_or_default(),
                    ui_events: ui_events.to_vec(),
                    entities: entities.clone(),
                };
                (entity_id, context)
            })
            .collect()
    }

    #[cfg(feature = "subsystem-scripting-csharp")]
    fn script_gameplay_entity_snapshots(
        &self,
    ) -> std::collections::BTreeMap<String, GameplayEntitySnapshot> {
        self.world_slot
            .with_world(|world| {
                world
                    .persistent_entities()
                    .map(|(entity_id, entity)| {
                        let transform = world
                            .get::<engine_scene::components::Transform>(entity)
                            .map(|transform| ScriptTransform {
                                translation: transform.translation.to_array(),
                                rotation: transform.rotation.to_array(),
                                scale: transform.scale.to_array(),
                            });
                        (entity_id.to_owned(), GameplayEntitySnapshot { transform })
                    })
                    .collect::<std::collections::BTreeMap<_, _>>()
            })
            .unwrap_or_default()
    }

    #[cfg(feature = "subsystem-scripting-csharp")]
    fn apply_script_gameplay_commands(
        &mut self,
        commands: Vec<engine_script::OwnedGameplayCommand>,
    ) -> Vec<Diagnostic> {
        if commands.is_empty() {
            return Vec::new();
        }
        if self.world_slot.with_world(|_| ()).is_none() {
            return vec![Diagnostic::new(
                "SCRIPT_WORLD_MISSING",
                DiagnosticSeverity::Error,
                "script",
                "gameplay commands could not be applied because no World is active",
            )];
        }

        let mut diagnostics = Vec::new();
        let mut scene_request: Option<SceneLoadRequest> = None;
        for engine_script::OwnedGameplayCommand { entity_id, command } in commands {
            match command {
                GameplayCommand::SetTransform { transform } => {
                    apply_script_transform_command(
                        &self.world_slot,
                        &entity_id,
                        &entity_id,
                        transform,
                        &mut diagnostics,
                    );
                }
                GameplayCommand::SetEntityTransform {
                    entity_id: target_id,
                    transform,
                } => {
                    if !script_command_owner_exists(&self.world_slot, &entity_id) {
                        diagnostics.push(script_owner_missing_diagnostic(
                            &entity_id,
                            "set another entity's Transform",
                        ));
                        continue;
                    }
                    if let Err(reason) = engine_script::validate_entity_id(&target_id) {
                        let mut diagnostic = Diagnostic::new(
                            "SCRIPT_ENTITY_TARGET_INVALID",
                            DiagnosticSeverity::Error,
                            "script",
                            format!(
                                "script entity '{entity_id}' requested an invalid Transform target: {reason}"
                            ),
                        );
                        diagnostic.entity = Some(entity_id);
                        diagnostics.push(diagnostic);
                        continue;
                    }
                    apply_script_transform_command(
                        &self.world_slot,
                        &entity_id,
                        &target_id,
                        transform,
                        &mut diagnostics,
                    );
                }
                GameplayCommand::CreateEntity {
                    entity_id: target_id,
                    transform,
                } => {
                    if !script_command_owner_exists(&self.world_slot, &entity_id) {
                        diagnostics.push(script_owner_missing_diagnostic(
                            &entity_id,
                            "create a persistent entity",
                        ));
                        continue;
                    }
                    if let Err(reason) = engine_script::validate_entity_id(&target_id) {
                        let mut diagnostic = Diagnostic::new(
                            "SCRIPT_ENTITY_CREATE_ID_INVALID",
                            DiagnosticSeverity::Error,
                            "script",
                            format!(
                                "script entity '{entity_id}' requested an invalid entity creation target: {reason}"
                            ),
                        );
                        diagnostic.entity = Some(entity_id);
                        diagnostics.push(diagnostic);
                        continue;
                    }
                    create_script_entity(
                        &self.world_slot,
                        &entity_id,
                        &target_id,
                        transform,
                        &mut diagnostics,
                    );
                }
                GameplayCommand::DestroySelf => {
                    destroy_script_entity(
                        &self.world_slot,
                        &mut self.script_engine,
                        &entity_id,
                        &entity_id,
                        &mut diagnostics,
                    );
                }
                GameplayCommand::DestroyEntity {
                    entity_id: target_id,
                } => {
                    if !script_command_owner_exists(&self.world_slot, &entity_id) {
                        diagnostics.push(script_owner_missing_diagnostic(
                            &entity_id,
                            "destroy another entity",
                        ));
                        continue;
                    }
                    if let Err(reason) = engine_script::validate_entity_id(&target_id) {
                        let mut diagnostic = Diagnostic::new(
                            "SCRIPT_ENTITY_TARGET_INVALID",
                            DiagnosticSeverity::Error,
                            "script",
                            format!(
                                "script entity '{entity_id}' requested an invalid destroy target: {reason}"
                            ),
                        );
                        diagnostic.entity = Some(entity_id);
                        diagnostics.push(diagnostic);
                        continue;
                    }
                    destroy_script_entity(
                        &self.world_slot,
                        &mut self.script_engine,
                        &entity_id,
                        &target_id,
                        &mut diagnostics,
                    );
                }
                GameplayCommand::LoadScene { scene_id } => {
                    if !script_command_owner_exists(&self.world_slot, &entity_id) {
                        diagnostics.push(script_owner_missing_diagnostic(
                            &entity_id,
                            &format!("request scene '{scene_id}'"),
                        ));
                        continue;
                    }
                    if let Err(reason) = engine_script::validate_scene_id(&scene_id) {
                        let mut diagnostic = Diagnostic::new(
                            "SCRIPT_SCENE_REQUEST_INVALID",
                            DiagnosticSeverity::Error,
                            "script",
                            format!("script entity '{entity_id}' {reason}"),
                        );
                        diagnostic.entity = Some(entity_id);
                        diagnostics.push(diagnostic);
                        continue;
                    }
                    let request = SceneLoadRequest {
                        scene_id,
                        requested_by: entity_id.clone(),
                    };
                    if let Some(existing) = &scene_request {
                        if existing != &request {
                            let mut diagnostic = Diagnostic::new(
                                "SCRIPT_SCENE_REQUEST_CONFLICT",
                                DiagnosticSeverity::Error,
                                "script",
                                format!(
                                    "script entity '{}' requested scene '{}' after '{}' already requested '{}'; the first request wins",
                                    request.requested_by,
                                    request.scene_id,
                                    existing.requested_by,
                                    existing.scene_id,
                                ),
                            );
                            diagnostic.entity = Some(request.requested_by);
                            diagnostics.push(diagnostic);
                        }
                    } else {
                        scene_request = Some(request);
                    }
                }
            }
        }

        if let Some(scene_request) = scene_request {
            if let Some(existing) = &self.pending_scene_request {
                if existing != &scene_request {
                    let mut diagnostic = Diagnostic::new(
                        "SCRIPT_SCENE_REQUEST_CONFLICT",
                        DiagnosticSeverity::Error,
                        "script",
                        format!(
                            "script entity '{}' requested scene '{}' while '{}' already has a pending request for '{}'; the first request wins",
                            scene_request.requested_by,
                            scene_request.scene_id,
                            existing.requested_by,
                            existing.scene_id,
                        ),
                    );
                    diagnostic.entity = Some(scene_request.requested_by);
                    diagnostics.push(diagnostic);
                }
            } else {
                self.pending_scene_request = Some(scene_request);
            }
        }
        diagnostics
    }
}

#[cfg(feature = "subsystem-scripting-csharp")]
fn validate_script_transform(transform: &ScriptTransform) -> Result<(), &'static str> {
    engine_script::validate_script_transform(transform).map_err(|reason| match reason.as_str() {
        "translation, rotation, and scale must contain only finite values" => {
            "translation, rotation, and scale must contain only finite values"
        }
        "rotation quaternion must not be zero length" => {
            "rotation quaternion must not be zero length"
        }
        _ => "Transform is invalid",
    })
}

#[cfg(feature = "subsystem-scripting-csharp")]
fn script_command_owner_exists(world_slot: &WorldSlot, entity_id: &str) -> bool {
    world_slot
        .with_world(|world| world.entity_by_persistent_id(entity_id).is_some())
        .unwrap_or(false)
}

#[cfg(feature = "subsystem-scripting-csharp")]
fn script_owner_missing_diagnostic(entity_id: &str, action: &str) -> Diagnostic {
    let mut diagnostic = Diagnostic::new(
        "SCRIPT_COMMAND_OWNER_MISSING",
        DiagnosticSeverity::Error,
        "script",
        format!("script entity '{entity_id}' no longer exists and cannot {action}"),
    );
    diagnostic.entity = Some(entity_id.to_owned());
    diagnostic
}

#[cfg(feature = "subsystem-scripting-csharp")]
fn apply_script_transform_command(
    world_slot: &WorldSlot,
    requested_by: &str,
    target_id: &str,
    transform: ScriptTransform,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if let Err(reason) = validate_script_transform(&transform) {
        let mut diagnostic = Diagnostic::new(
            "SCRIPT_TRANSFORM_INVALID",
            DiagnosticSeverity::Error,
            "script",
            format!(
                "script entity '{requested_by}' produced an invalid Transform for entity '{target_id}': {reason}"
            ),
        );
        diagnostic.entity = Some(target_id.to_owned());
        diagnostics.push(diagnostic);
        return;
    }
    let applied = world_slot
        .with_world_mut(|world| {
            let entity = world.entity_by_persistent_id(target_id)?;
            let current = world.get_mut::<engine_scene::components::Transform>(entity)?;
            current.translation = glam::Vec3::from_array(transform.translation);
            // Managed callers may construct a finite but non-unit quaternion.
            current.rotation = glam::Quat::from_array(transform.rotation).normalize();
            current.scale = glam::Vec3::from_array(transform.scale);
            Some(())
        })
        .flatten()
        .is_some();
    if !applied {
        let mut diagnostic = Diagnostic::new(
            "SCRIPT_TRANSFORM_TARGET_MISSING",
            DiagnosticSeverity::Error,
            "script",
            format!(
                "script entity '{requested_by}' targeted entity '{target_id}', which no longer exists or has no Transform"
            ),
        );
        diagnostic.entity = Some(target_id.to_owned());
        diagnostics.push(diagnostic);
    }
}

#[cfg(feature = "subsystem-scripting-csharp")]
fn create_script_entity(
    world_slot: &WorldSlot,
    requested_by: &str,
    target_id: &str,
    transform: ScriptTransform,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if let Err(reason) = validate_script_transform(&transform) {
        let mut diagnostic = Diagnostic::new(
            "SCRIPT_ENTITY_CREATE_TRANSFORM_INVALID",
            DiagnosticSeverity::Error,
            "script",
            format!(
                "script entity '{requested_by}' produced an invalid Transform while creating entity '{target_id}': {reason}"
            ),
        );
        diagnostic.entity = Some(target_id.to_owned());
        diagnostics.push(diagnostic);
        return;
    }

    let created = world_slot.with_world_mut(|world| {
        let entity = world.create_persistent_entity(target_id.to_owned())?;
        world.add_component(
            entity,
            engine_scene::components::Transform {
                translation: glam::Vec3::from_array(transform.translation),
                rotation: glam::Quat::from_array(transform.rotation).normalize(),
                scale: glam::Vec3::from_array(transform.scale),
                ..Default::default()
            },
        );
        Ok::<_, engine_scene::PersistentEntityCreateError>(())
    });

    match created {
        Some(Ok(())) => {}
        Some(Err(engine_scene::PersistentEntityCreateError::DuplicateId(_))) => {
            let mut diagnostic = Diagnostic::new(
                "SCRIPT_ENTITY_CREATE_CONFLICT",
                DiagnosticSeverity::Error,
                "script",
                format!(
                    "script entity '{requested_by}' could not create entity '{target_id}' because that persistent ID already exists; the first creation wins"
                ),
            );
            diagnostic.entity = Some(target_id.to_owned());
            diagnostics.push(diagnostic);
        }
        Some(Err(error)) => {
            let mut diagnostic = Diagnostic::new(
                "SCRIPT_ENTITY_CREATE_FAILED",
                DiagnosticSeverity::Error,
                "script",
                format!(
                    "script entity '{requested_by}' could not create entity '{target_id}': {error}"
                ),
            );
            diagnostic.entity = Some(target_id.to_owned());
            diagnostics.push(diagnostic);
        }
        None => diagnostics.push(Diagnostic::new(
            "SCRIPT_WORLD_MISSING",
            DiagnosticSeverity::Error,
            "script",
            format!(
                "script entity '{requested_by}' could not create entity '{target_id}' because no World is active"
            ),
        )),
    }
}

#[cfg(feature = "subsystem-scripting-csharp")]
fn destroy_script_entity(
    world_slot: &WorldSlot,
    script_engine: &mut ScriptEngine,
    requested_by: &str,
    target_id: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let target = world_slot
        .with_world(|world| world.entity_by_persistent_id(target_id))
        .flatten();
    let Some(target) = target else {
        let mut diagnostic = Diagnostic::new(
            "SCRIPT_DESTROY_TARGET_MISSING",
            DiagnosticSeverity::Error,
            "script",
            format!(
                "script entity '{requested_by}' tried to destroy entity '{target_id}', but that entity does not exist"
            ),
        );
        diagnostic.entity = Some(target_id.to_owned());
        diagnostics.push(diagnostic);
        return;
    };

    // OnDestroy runs while the entity and World are still valid. The manager
    // removes the instances immediately afterwards, so the destroyed entity
    // cannot be ticked again on the next frame.
    diagnostics.extend(script_engine.destroy_entity_instances(target_id));
    let destroyed = world_slot
        .with_world_mut(|world| world.destroy_entity(target))
        .unwrap_or(false);
    if !destroyed {
        let mut diagnostic = Diagnostic::new(
            "SCRIPT_DESTROY_FAILED",
            DiagnosticSeverity::Error,
            "script",
            format!("entity '{target_id}' became stale before it could be destroyed"),
        );
        diagnostic.entity = Some(target_id.to_owned());
        diagnostics.push(diagnostic);
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

fn validate_registered_asset_id(
    kind: &str,
    requested: &AssetId,
    embedded: &AssetId,
) -> Result<(), Vec<Diagnostic>> {
    if requested == embedded {
        return Ok(());
    }
    let mut diagnostic = Diagnostic::new(
        "AS0001",
        DiagnosticSeverity::Error,
        "engine-core.assets",
        format!(
            "registered {kind} asset '{}' embeds mismatched id '{}'",
            requested.id, embedded.id
        ),
    );
    diagnostic.asset = Some(requested.clone());
    Err(vec![diagnostic])
}

fn install_builtin_render_assets(registry: &mut AssetRegistry) {
    let mesh = engine_asset::mesh::create_test_cube();
    let (vertex_bytes, index_bytes, index_count, _) =
        engine_asset::mesh::mesh_data_to_upload_bytes(&mesh);
    let content_hash =
        engine_asset::compute_content_hash(&[vertex_bytes.as_slice(), index_bytes.as_slice()]);
    let mesh_id = AssetId::new("mesh-cube");
    registry.insert_typed(
        mesh_id.clone(),
        MeshUpload {
            mesh_id,
            vertex_format: MeshVertexFormat::Pbr32,
            vertex_count: u32::try_from(mesh.positions.len()).unwrap_or(u32::MAX),
            vertex_bytes,
            index_format: engine_renderer::IndexFormat::U32,
            index_count,
            index_bytes,
            bounds: engine_renderer::AxisAlignedBox {
                min: mesh.bounds.0.to_array(),
                max: mesh.bounds.1.to_array(),
            },
            content_hash,
        },
    );

    let material_id = AssetId::new("mat-default");
    registry.insert_typed(
        material_id.clone(),
        MaterialUpload {
            material_id,
            base_color: [1.0, 1.0, 1.0, 1.0],
            metallic: 0.0,
            roughness: 1.0,
            ambient_occlusion: 1.0,
            base_color_texture: None,
            transparency: engine_renderer::Transparency::Opaque,
            double_sided: false,
            content_hash: engine_asset::compute_content_hash(&[b"mat-default-v1"]),
        },
    );
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

    struct RecordingBackend {
        uploads: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
        rendered_ui_batch_counts: Option<std::sync::Arc<std::sync::Mutex<Vec<usize>>>>,
    }

    struct CountingRenderExtension {
        calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    impl engine_renderer::RenderExtensionProducer for CountingRenderExtension {
        fn name(&self) -> &str {
            "test-counting-extension"
        }

        fn produce(&self, _input: &mut engine_renderer::RenderFrameInput, _frame_index: u64) {
            self.calls
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    impl engine_renderer::BackendRenderer for RecordingBackend {
        fn render_frame(
            &mut self,
            input: &engine_renderer::RenderFrameInput,
        ) -> Result<FrameStats, Vec<Diagnostic>> {
            if let Some(counts) = &self.rendered_ui_batch_counts {
                counts
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(input.ui_batches.len());
            }
            Ok(FrameStats::default())
        }

        fn execute_pass(
            &mut self,
            input: &engine_renderer::RenderFrameInput,
            _pass: &engine_renderer::render_graph::PassNode,
            _frame_stats: &mut FrameStats,
        ) -> Result<(), Vec<Diagnostic>> {
            if let Some(counts) = &self.rendered_ui_batch_counts {
                counts
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(input.ui_batches.len());
            }
            Ok(())
        }

        fn abort_frame(&mut self) -> Result<(), Vec<Diagnostic>> {
            Ok(())
        }

        fn upload_mesh(
            &mut self,
            upload: MeshUpload,
        ) -> Result<engine_renderer::UploadReceipt, Vec<Diagnostic>> {
            self.uploads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(format!("mesh:{}", upload.mesh_id.id));
            Ok(engine_renderer::UploadReceipt::new(1))
        }

        fn upload_texture(
            &mut self,
            upload: TextureUpload,
        ) -> Result<engine_renderer::UploadReceipt, Vec<Diagnostic>> {
            self.uploads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(format!("texture:{}", upload.texture_id.id));
            Ok(engine_renderer::UploadReceipt::new(1))
        }

        fn upload_material(
            &mut self,
            upload: MaterialUpload,
        ) -> Result<engine_renderer::UploadReceipt, Vec<Diagnostic>> {
            self.uploads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(format!("material:{}", upload.material_id.id));
            Ok(engine_renderer::UploadReceipt::new(1))
        }
    }

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

    #[cfg(feature = "runtime-subsystems")]
    #[test]
    fn runtime_builder_registers_runtime_subsystem_extensions() {
        let builder = EngineRuntimeBuilder::default();
        for component in [
            "engine.canvas",
            "engine.audio_source",
            "engine.audio_listener",
            "engine.animation_player",
            "engine.skeleton",
            "engine.ik_target",
            "engine.nav_agent",
        ] {
            assert!(
                builder.component_registry().is_registered(component),
                "missing component extension {component}"
            );
        }
        for asset_type in [
            "audio_clip",
            "skeleton",
            "animation_clip",
            "navmesh",
            "behavior",
        ] {
            assert!(
                builder.asset_type_registry().get(asset_type).is_some(),
                "missing asset type extension {asset_type}"
            );
        }
        assert_eq!(builder.render_extension_registry().producer_count(), 1);
        assert!(builder.debug_draw_registry().provider_count() >= 3);
        assert_eq!(
            builder
                .animation_extension_handles()
                .skinned_extract
                .pending_count(),
            0
        );
    }

    #[cfg(feature = "runtime-subsystems")]
    #[test]
    fn runtime_subsystem_components_survive_strict_scene_loading() {
        let _guard = serial_ffi_world_test();
        let mut scene = engine_scene::sample_scene();
        for component in [
            "engine.canvas",
            "engine.audio_source",
            "engine.audio_listener",
            "engine.animation_player",
            "engine.skeleton",
            "engine.ik_target",
            "engine.nav_agent",
        ] {
            insert_empty_component(&mut scene, component);
        }

        let mut runtime = EngineRuntime::new(EngineConfig::default());
        runtime
            .load_scene_to_world(scene)
            .expect("registered runtime subsystem components should load strictly");

        runtime
            .with_world(|world| {
                assert_eq!(world.query::<engine_ui::Canvas>().count(), 1);
                assert_eq!(
                    world
                        .query::<engine_audio::components::AudioSourceComponent>()
                        .count(),
                    1
                );
                assert_eq!(
                    world.query::<engine_animation::AnimationPlayer>().count(),
                    1
                );
                assert_eq!(world.query::<engine_nav::AiAgent>().count(), 1);
            })
            .expect("strict load should install a World");
    }

    #[cfg(not(feature = "runtime-subsystems"))]
    #[test]
    fn minimal_runtime_does_not_install_optional_subsystems() {
        let builder = EngineRuntimeBuilder::default();
        assert!(!builder.component_registry().is_registered("engine.canvas"));
        assert!(!builder
            .component_registry()
            .is_registered("engine.audio_source"));
        assert!(!builder
            .component_registry()
            .is_registered("engine.animation_player"));
        assert!(!builder
            .component_registry()
            .is_registered("engine.nav_agent"));
        assert!(builder.asset_type_registry().get("audio_clip").is_none());
        assert_eq!(builder.render_extension_registry().producer_count(), 0);
    }

    #[test]
    fn runtime_invokes_registered_render_extensions_before_drawing() {
        let _guard = serial_ffi_world_test();
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut builder = EngineRuntimeBuilder::default();
        builder
            .render_extension_registry_mut()
            .register(Box::new(CountingRenderExtension {
                calls: std::sync::Arc::clone(&calls),
            }));
        let mut runtime = builder.build();
        runtime
            .renderer_mut()
            .set_backend(Box::new(RecordingBackend {
                uploads: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
                rendered_ui_batch_counts: None,
            }));
        runtime
            .load_scene_to_world(engine_scene::sample_scene())
            .expect("sample scene should load");

        runtime.render_frame(17).expect("frame should render");

        assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 1);
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
    fn runtime_submits_host_ui_batches_with_the_scene() {
        let _guard = serial_ffi_world_test();
        let ui_counts = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut runtime = EngineRuntime::new(EngineConfig::default());
        runtime
            .renderer_mut()
            .set_backend(Box::new(RecordingBackend {
                uploads: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
                rendered_ui_batch_counts: Some(std::sync::Arc::clone(&ui_counts)),
            }));
        runtime
            .load_scene_to_world(engine_scene::sample_scene())
            .expect("sample scene should load");

        let batch = engine_renderer::UiBatch {
            canvas_id: "editor".into(),
            z_order: 0,
            clip_rect: engine_renderer::Rect {
                min: [0.0, 0.0],
                max: [800.0, 600.0],
            },
            texture: None,
            vertices: vec![
                engine_renderer::UiVertex {
                    position: [0.0, 0.0],
                    uv: [0.0, 0.0],
                    color: [255; 4],
                },
                engine_renderer::UiVertex {
                    position: [10.0, 0.0],
                    uv: [1.0, 0.0],
                    color: [255; 4],
                },
                engine_renderer::UiVertex {
                    position: [10.0, 10.0],
                    uv: [1.0, 1.0],
                    color: [255; 4],
                },
                engine_renderer::UiVertex {
                    position: [0.0, 10.0],
                    uv: [0.0, 1.0],
                    color: [255; 4],
                },
            ],
            indices: vec![0, 1, 2, 0, 2, 3],
            material: AssetId::new("ui/default"),
        };

        runtime
            .render_frame_with_ui(7, vec![batch])
            .expect("scene and host UI should render together");
        let ui_counts = ui_counts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(!ui_counts.is_empty());
        assert!(ui_counts.iter().all(|count| *count == 1));
    }

    #[cfg(feature = "runtime-subsystems")]
    #[test]
    fn game_loop_submits_retained_scene_canvas_batches_automatically() {
        let _guard = serial_ffi_world_test();
        let ui_counts = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut game_loop = game_loop::GameLoop::new(EngineConfig::default());
        game_loop
            .runtime
            .renderer_mut()
            .set_backend(Box::new(RecordingBackend {
                uploads: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
                rendered_ui_batch_counts: Some(std::sync::Arc::clone(&ui_counts)),
            }));
        game_loop
            .load_scene(engine_scene::sample_scene())
            .expect("sample scene should load");
        game_loop
            .runtime
            .with_world_mut(|world| {
                let entity = world.entity_by_persistent_id("camera-main").unwrap();
                let mut canvas = engine_ui::Canvas::new(320.0, 180.0);
                canvas.add_element(engine_ui::UiElement::new(
                    engine_ui::UiElementKind::Panel {
                        color: engine_ui::Color::new(40, 80, 120, 255),
                    },
                    engine_ui::Layout::FILL,
                ));
                world.add_component(entity, canvas);
            })
            .expect("runtime world should be available");

        game_loop
            .render(7)
            .expect("retained scene Canvas should render automatically");

        let ui_counts = ui_counts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(!ui_counts.is_empty());
        assert!(ui_counts.iter().all(|count| *count == 1));
    }

    #[test]
    fn runtime_uploads_and_deduplicates_ui_only_textures() {
        let _guard = serial_ffi_world_test();
        let uploads = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut runtime = EngineRuntime::new(EngineConfig::default());
        runtime
            .renderer_mut()
            .set_backend(Box::new(RecordingBackend {
                uploads: std::sync::Arc::clone(&uploads),
                rendered_ui_batch_counts: None,
            }));

        let texture_id = AssetId::new("texture-ui-atlas");
        runtime.register_texture_asset(TextureUpload {
            texture_id: texture_id.clone(),
            width: 1,
            height: 1,
            format: engine_renderer::TextureUploadFormat::Rgba8,
            color_space: engine_renderer::ColorSpace::Srgb,
            mip_levels: vec![engine_renderer::TextureMipLevel {
                width: 1,
                height: 1,
                bytes: vec![255, 255, 255, 255],
            }],
            sampler: engine_renderer::SamplerDescriptor::default(),
            content_hash: [9; 32],
        });
        runtime
            .load_scene_to_world(engine_scene::sample_scene())
            .expect("sample scene should load");
        let batch = engine_renderer::UiBatch {
            canvas_id: "hud".into(),
            z_order: 0,
            clip_rect: engine_renderer::Rect {
                min: [0.0, 0.0],
                max: [128.0, 128.0],
            },
            texture: Some(texture_id),
            vertices: vec![
                engine_renderer::UiVertex {
                    position: [0.0, 0.0],
                    uv: [0.0, 0.0],
                    color: [255; 4],
                },
                engine_renderer::UiVertex {
                    position: [1.0, 0.0],
                    uv: [1.0, 0.0],
                    color: [255; 4],
                },
                engine_renderer::UiVertex {
                    position: [0.0, 1.0],
                    uv: [0.0, 1.0],
                    color: [255; 4],
                },
            ],
            indices: vec![0, 1, 2],
            material: AssetId::new("ui/default"),
        };

        runtime
            .render_frame_with_ui(8, vec![batch.clone(), batch])
            .expect("UI texture should be synchronised before rendering");

        let uploads = uploads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(
            uploads
                .iter()
                .filter(|upload| upload.as_str() == "texture:texture-ui-atlas")
                .count(),
            1
        );
    }

    #[test]
    fn runtime_uploads_registered_scene_resources_in_dependency_order() {
        let _guard = serial_ffi_world_test();
        let uploads = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut runtime = EngineRuntime::new(EngineConfig::default());
        runtime
            .renderer_mut()
            .set_backend(Box::new(RecordingBackend {
                uploads: std::sync::Arc::clone(&uploads),
                rendered_ui_batch_counts: None,
            }));

        let texture_id = AssetId::new("texture-auto");
        runtime.register_texture_asset(TextureUpload {
            texture_id: texture_id.clone(),
            width: 1,
            height: 1,
            format: engine_renderer::TextureUploadFormat::Rgba8,
            color_space: engine_renderer::ColorSpace::Srgb,
            mip_levels: vec![engine_renderer::TextureMipLevel {
                width: 1,
                height: 1,
                bytes: vec![255, 255, 255, 255],
            }],
            sampler: engine_renderer::SamplerDescriptor::default(),
            content_hash: [1; 32],
        });
        let material_id = AssetId::new("material-auto");
        runtime.register_material_asset(MaterialUpload {
            material_id: material_id.clone(),
            base_color: [1.0; 4],
            metallic: 0.0,
            roughness: 1.0,
            ambient_occlusion: 1.0,
            base_color_texture: Some(texture_id),
            transparency: engine_renderer::Transparency::Opaque,
            double_sided: false,
            content_hash: [2; 32],
        });

        let mut scene = engine_scene::sample_scene();
        let renderable = scene
            .entities
            .iter_mut()
            .find_map(|entity| entity.components.get_mut("engine.renderable"))
            .expect("sample renderable");
        renderable.fields.insert(
            "material".to_string(),
            engine_serialize::Value::Asset(material_id),
        );
        runtime.load_scene(scene).expect("scene load");

        runtime.render_frame(0).expect("render");

        assert_eq!(
            *uploads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            vec![
                "texture:texture-auto".to_string(),
                "material:material-auto".to_string(),
                "mesh:mesh-cube".to_string(),
            ]
        );
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
    fn script_engine_replacement_is_atomic_and_does_not_accumulate_hosts() {
        use engine_script::{MockHost, ScriptEngine};

        let mut runtime = EngineRuntime::new(EngineConfig::default());
        runtime.register_script_host(Box::new(MockHost::new()));
        runtime
            .load_script_assembly("old", "mock", b"old")
            .expect("old runtime assembly");

        let invalid_candidate = ScriptEngine::new();
        let error = runtime
            .replace_script_engine(invalid_candidate, "mock")
            .expect_err("candidate without the selected host must be rejected");
        assert!(error.to_string().contains("exactly one host"));
        assert_eq!(runtime.script_engine().host_count(), 1);
        assert_eq!(runtime.script_engine().managers()[0].assembly_count(), 1);

        let mut duplicate_candidate = ScriptEngine::new();
        duplicate_candidate.register_host(Box::new(MockHost::new()));
        duplicate_candidate.register_host(Box::new(MockHost::new()));
        runtime
            .replace_script_engine(duplicate_candidate, "mock")
            .expect_err("duplicate selected hosts must be rejected");
        assert_eq!(runtime.script_engine().host_count(), 1);
        assert_eq!(runtime.script_engine().managers()[0].assembly_count(), 1);

        let mut candidate = ScriptEngine::new();
        candidate.register_host(Box::new(MockHost::new()));
        candidate
            .load_script("new-dependency", "mock", b"dependency")
            .expect("candidate dependency");
        candidate
            .load_script("new-game", "mock", b"game")
            .expect("candidate game assembly");

        runtime
            .replace_script_engine(candidate, "mock")
            .expect("valid candidate should replace the runtime");
        assert_eq!(runtime.script_engine().host_count(), 1);
        assert_eq!(runtime.script_engine().managers()[0].assembly_count(), 2);
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
    fn script_create_entity_is_transactional_first_wins_and_enters_next_snapshot() {
        let _guard = serial_ffi_world_test();
        let mut runtime = EngineRuntime::new(EngineConfig::default());
        runtime.set_world(World::from_scene(&engine_scene::sample_scene()));
        let first_transform = ScriptTransform {
            translation: [7.0, 8.0, 9.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [2.0, 3.0, 4.0],
        };
        let commands = vec![
            engine_script::OwnedGameplayCommand {
                entity_id: "cube-01".into(),
                command: GameplayCommand::CreateEntity {
                    entity_id: "spawned-01".into(),
                    transform: first_transform.clone(),
                },
            },
            engine_script::OwnedGameplayCommand {
                entity_id: "cube-01".into(),
                command: GameplayCommand::CreateEntity {
                    entity_id: "spawned-01".into(),
                    transform: ScriptTransform {
                        translation: [100.0, 100.0, 100.0],
                        rotation: [0.0, 0.0, 0.0, 1.0],
                        scale: [1.0; 3],
                    },
                },
            },
        ];

        let diagnostics = runtime.apply_script_gameplay_commands(commands);

        assert_eq!(
            diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "SCRIPT_ENTITY_CREATE_CONFLICT")
                .count(),
            1
        );
        runtime
            .with_world(|world| {
                assert_eq!(world.alive_count(), 3);
                let entity = world
                    .entity_by_persistent_id("spawned-01")
                    .expect("first creation must persist");
                let transform = world
                    .get::<engine_scene::components::Transform>(entity)
                    .expect("created entity must have Transform");
                assert_eq!(
                    transform.translation.to_array(),
                    first_transform.translation
                );
                assert_eq!(transform.rotation.to_array(), first_transform.rotation);
                assert_eq!(transform.scale.to_array(), first_transform.scale);
            })
            .expect("runtime must keep an active World");
        let snapshots = runtime.script_gameplay_entity_snapshots();
        assert_eq!(
            snapshots["spawned-01"].transform,
            Some(first_transform),
            "the next script context must include the newly-created entity"
        );
    }

    #[cfg(feature = "subsystem-scripting-csharp")]
    #[test]
    fn script_create_entity_validation_and_missing_owner_never_partially_create() {
        let _guard = serial_ffi_world_test();
        let mut runtime = EngineRuntime::new(EngineConfig::default());
        runtime.set_world(World::from_scene(&engine_scene::sample_scene()));
        let valid_transform = ScriptTransform {
            translation: [0.0; 3],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0; 3],
        };
        let commands = vec![
            engine_script::OwnedGameplayCommand {
                entity_id: "cube-01".into(),
                command: GameplayCommand::CreateEntity {
                    entity_id: "../invalid".into(),
                    transform: valid_transform.clone(),
                },
            },
            engine_script::OwnedGameplayCommand {
                entity_id: "cube-01".into(),
                command: GameplayCommand::CreateEntity {
                    entity_id: "invalid-transform".into(),
                    transform: ScriptTransform {
                        rotation: [0.0; 4],
                        ..valid_transform.clone()
                    },
                },
            },
            engine_script::OwnedGameplayCommand {
                entity_id: "missing-owner".into(),
                command: GameplayCommand::CreateEntity {
                    entity_id: "orphan".into(),
                    transform: valid_transform,
                },
            },
        ];

        let diagnostics = runtime.apply_script_gameplay_commands(commands);

        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SCRIPT_ENTITY_CREATE_ID_INVALID"));
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SCRIPT_ENTITY_CREATE_TRANSFORM_INVALID"));
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SCRIPT_COMMAND_OWNER_MISSING"));
        runtime
            .with_world(|world| {
                assert_eq!(world.alive_count(), 2);
                assert!(world.entity_by_persistent_id("invalid-transform").is_none());
                assert!(world.entity_by_persistent_id("orphan").is_none());
            })
            .expect("runtime must keep an active World");
    }

    #[cfg(feature = "subsystem-scripting-csharp")]
    #[test]
    fn script_create_entity_rechecks_owner_after_prior_same_frame_destroy() {
        let _guard = serial_ffi_world_test();
        let mut runtime = EngineRuntime::new(EngineConfig::default());
        runtime.set_world(World::from_scene(&engine_scene::sample_scene()));
        let commands = vec![
            engine_script::OwnedGameplayCommand {
                entity_id: "cube-01".into(),
                command: GameplayCommand::DestroySelf,
            },
            engine_script::OwnedGameplayCommand {
                entity_id: "cube-01".into(),
                command: GameplayCommand::CreateEntity {
                    entity_id: "after-destroy".into(),
                    transform: ScriptTransform {
                        translation: [0.0; 3],
                        rotation: [0.0, 0.0, 0.0, 1.0],
                        scale: [1.0; 3],
                    },
                },
            },
        ];

        let diagnostics = runtime.apply_script_gameplay_commands(commands);

        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SCRIPT_COMMAND_OWNER_MISSING"));
        runtime
            .with_world(|world| {
                assert!(world.entity_by_persistent_id("cube-01").is_none());
                assert!(world.entity_by_persistent_id("after-destroy").is_none());
            })
            .expect("runtime must keep an active World");
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
            .load_scene_to_world(scene.clone())
            .expect("engine.script metadata should be allowed");

        // After load_scene, the script engine should have an instance
        assert_eq!(runtime.script_engine.host_count(), 1);
        let after = runtime.script_engine.managers()[0].instance_count();
        assert_eq!(after, 1, "script instance should have been created");

        runtime
            .load_scene_to_world(scene)
            .expect("reloading a scripted scene should replace its instances");
        assert_eq!(
            runtime.script_engine.managers()[0].instance_count(),
            1,
            "scene reload must not accumulate duplicate script instances"
        );

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
