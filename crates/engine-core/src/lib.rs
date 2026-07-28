#![forbid(unsafe_code)]

pub mod diagnostics;
pub use diagnostics::*;
pub mod component_audit;
pub mod cooked_assets;
pub use cooked_assets::*;
pub mod runtime_mesh;
pub use runtime_mesh::*;
pub mod asset_stream;
mod runtime;
pub use asset_stream::*;
pub mod cell_stream;
pub use cell_stream::{CellStreamingConfig, CellStreamingDriver};
pub mod savegame;
pub use savegame::*;
#[cfg(feature = "subsystem-terrain")]
pub mod terrain;
#[cfg(feature = "subsystem-terrain")]
pub use terrain::{TerrainBindingStats, TerrainSystem};

use engine_asset::{AssetHandle, AssetRegistry};
use engine_renderer::{
    AssetId, DebugDrawRegistry, MaterialUpload, MeshUpload, MeshVertexFormat,
    RenderExtensionRegistry, Renderer, TextureUpload,
};
use engine_scene::{
    validate_scene, AssetTypeRegistry, ComponentRegistry, Scene, SceneLoadDiagnostic, World,
    WorldSlot,
};
use engine_serialize::{Diagnostic, DiagnosticSeverity};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

#[cfg(test)]
use engine_renderer::FrameStats;

pub mod ffi_init;
pub mod game_loop;
#[cfg(all(feature = "subsystem-animation", feature = "subsystem-physics"))]
mod ragdoll_runtime;
#[cfg(feature = "subsystem-ui")]
pub use game_loop::{RuntimeUiEvent, RuntimeUiValue};

// ── Optional script subsystem ─────────────────────────────────────────────

#[cfg(feature = "subsystem-scripting-csharp")]
pub mod script;
#[cfg(feature = "subsystem-scripting-csharp")]
mod script_commands;
#[cfg(feature = "subsystem-scripting-csharp")]
mod script_components;
#[cfg(all(feature = "subsystem-scripting-csharp", feature = "subsystem-physics"))]
use engine_script::GameplayDamageEvent;
#[cfg(all(
    feature = "subsystem-scripting-csharp",
    feature = "subsystem-physics",
    feature = "subsystem-animation"
))]
use engine_script::GameplayRagdollEvent;
#[cfg(feature = "subsystem-scripting-csharp")]
use engine_script::{
    GameplayCameraSnapshot, GameplayCommand, GameplayContext, GameplayEntitySnapshot,
    GameplayInputTransitions, GameplayInputValue, GameplayPhysicsEvent, GameplayPointerSnapshot,
    GameplaySaveEvent, GameplayUiEvent, ScriptEngine, ScriptError, ScriptHost, ScriptTransform,
};
#[cfg(feature = "subsystem-scripting-csharp")]
use script::{collect_scene_scripts, script_engine_state_summary};
#[cfg(feature = "subsystem-scripting-csharp")]
use script_commands::animation::{
    apply_script_animation_command, apply_script_morph_weights, ScriptAnimationCommand,
};
#[cfg(all(feature = "subsystem-scripting-csharp", feature = "subsystem-ui"))]
use script_commands::ui::apply_script_ui_command;

// ── Engine config ─────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineConfig {
    pub application_name: String,
    /// Master switch for GPU timestamp profiling (ENG-04). When `true`
    /// (default) a capable backend records per-pass GPU timestamps; when
    /// `false` the backend reports GPU timing as disabled and measures
    /// nothing. CPU stage timing is unaffected.
    pub gpu_timestamps: bool,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            application_name: "engine".to_string(),
            gpu_timestamps: true,
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
/// Character-controller and VFX components are always registered. Optional
/// subsystem features install their own components, assets, render producers,
/// and debug providers. Hosts can add further extensions before calling
/// [`build`](Self::build).
pub struct EngineRuntimeBuilder {
    config: EngineConfig,
    component_registry: ComponentRegistry,
    asset_type_registry: AssetTypeRegistry,
    render_extension_registry: RenderExtensionRegistry,
    debug_draw_registry: DebugDrawRegistry,
    #[cfg(feature = "subsystem-animation")]
    animation_extensions: engine_animation::AnimationExtensionHandles,
}

impl EngineRuntimeBuilder {
    pub fn new(config: EngineConfig) -> Self {
        let mut component_registry = ComponentRegistry::new();
        let mut asset_type_registry = AssetTypeRegistry::new();
        #[cfg(feature = "subsystem-animation")]
        let mut render_extension_registry = RenderExtensionRegistry::new();
        #[cfg(not(feature = "subsystem-animation"))]
        let render_extension_registry = RenderExtensionRegistry::new();
        let mut debug_draw_registry = DebugDrawRegistry::new();
        component_registry.register_core();
        engine_scene::register_prefab_asset_type(&mut asset_type_registry);
        engine_asset::cook::register_logic_asset_type(&mut asset_type_registry);
        engine_character::register_character_extensions(
            &mut component_registry,
            Some(&mut debug_draw_registry),
        );
        engine_vfx::register_vfx_extensions(&mut component_registry);
        #[cfg(feature = "subsystem-terrain")]
        engine_terrain::register_terrain_extensions(&mut component_registry);
        #[cfg(feature = "subsystem-physics")]
        engine_physics::register_physics_extensions(
            &mut component_registry,
            Some(&mut debug_draw_registry),
        );

        #[cfg(feature = "subsystem-ui")]
        engine_ui::register_ui_extensions(&mut component_registry);
        #[cfg(feature = "subsystem-audio")]
        engine_audio::register_audio_extensions(&mut component_registry, &mut asset_type_registry);
        #[cfg(feature = "subsystem-animation")]
        let animation_extensions = engine_animation::register_animation_extensions(
            &mut component_registry,
            &mut asset_type_registry,
            &mut render_extension_registry,
            &mut debug_draw_registry,
        );
        #[cfg(feature = "subsystem-navigation")]
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
            #[cfg(feature = "subsystem-animation")]
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
    #[cfg(feature = "subsystem-animation")]
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
            #[cfg(feature = "subsystem-animation")]
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

/// Registry-backed GPU resources known to have reached the active backend.
///
/// This is reconciliation state, not a second owner: the typed
/// [`AssetRegistry`] entry remains authoritative for both upload and removal.
#[derive(Default)]
struct SyncedRenderResources {
    meshes: BTreeSet<AssetId>,
    textures: BTreeSet<AssetId>,
    materials: BTreeSet<AssetId>,
    environment_maps: BTreeSet<AssetId>,
    morph_target_sets: BTreeSet<AssetId>,
}

pub struct EngineRuntime {
    config: EngineConfig,
    renderer: Renderer,
    asset_registry: AssetRegistry,
    render_environment: engine_renderer::EnvironmentSettings,
    loaded_cooked_asset_ids: BTreeSet<AssetId>,
    loaded_extension_asset_ids: BTreeMap<String, BTreeSet<AssetId>>,
    /// Handle table for runtime-registered dynamic meshes (ENG-20). The
    /// meshes themselves live as typed `MeshUpload` assets in
    /// `asset_registry`; the table owns handle generations, name lookup,
    /// and memory accounting.
    runtime_mesh_table: runtime_mesh::RuntimeMeshTable,
    /// Resources created from the registry and awaiting lifetime
    /// reconciliation by the canonical render sync.
    synced_render_resources: SyncedRenderResources,
    /// Exact registry allocations owned by the tooling-preview entry point.
    /// Allocation identity prevents a stale preview owner from unloading a
    /// persistent replacement that reused the same [`AssetId`].
    temporary_preview_textures: BTreeMap<AssetId, AssetHandle<TextureUpload>>,
    /// Lazily created background cooked-asset decoder; see
    /// [`EngineRuntime::enqueue_cooked_asset_stream`]. `None` until the first
    /// streamed enqueue so runtimes that never stream never spawn a thread.
    stream_loader: Option<AssetStreamLoader>,
    /// Per-drain commit budget applied when the loader is created.
    stream_budget: usize,
    scene: Option<Scene>,
    world_slot: WorldSlot,
    /// Per-pass CPU/GPU frame timing recorder and rolling statistics (ENG-04).
    frame_timing: engine_renderer::FrameTimingTracker,
    component_registry: Arc<ComponentRegistry>,
    asset_type_registry: AssetTypeRegistry,
    render_extension_registry: RenderExtensionRegistry,
    debug_draw_registry: DebugDrawRegistry,
    #[cfg(feature = "subsystem-animation")]
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
    script_pointer: GameplayPointerSnapshot,
    #[cfg(feature = "subsystem-scripting-csharp")]
    script_camera: Option<GameplayCameraSnapshot>,
    #[cfg(feature = "subsystem-scripting-csharp")]
    pending_save_requests: Vec<engine_script::OwnedGameplaySaveRequest>,
    #[cfg(feature = "subsystem-scripting-csharp")]
    script_save_events: std::collections::BTreeMap<String, Vec<GameplaySaveEvent>>,
    #[cfg(feature = "subsystem-scripting-csharp")]
    script_logic_asset_results:
        std::collections::BTreeMap<String, Vec<engine_script::GameplayLogicAssetResult>>,
    #[cfg(feature = "subsystem-scripting-csharp")]
    pending_scene_request: Option<SceneLoadRequest>,
    /// Validated physics queries drained from scripts during the current
    /// update. The owning [`crate::game_loop::GameLoop`] executes them
    /// against its physics world at the frame boundary and delivers results
    /// in the next frame snapshot.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pending_physics_queries: Vec<engine_script::OwnedGameplayPhysicsQuery>,
    /// Validated forces and impulses drained from scripts during the current
    /// update. The owning GameLoop resolves them to physics commands.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pending_physics_mutations: Vec<engine_script::OwnedGameplayPhysicsMutation>,
    #[cfg(feature = "subsystem-scripting-csharp")]
    pending_damage_requests: Vec<engine_script::OwnedGameplayDamageRequest>,
    #[cfg(feature = "subsystem-scripting-csharp")]
    script_damage_events:
        std::collections::BTreeMap<String, Vec<engine_script::GameplayDamageEvent>>,
    #[cfg(feature = "subsystem-scripting-csharp")]
    pending_ragdoll_requests: Vec<engine_script::OwnedGameplayRagdollRequest>,
    #[cfg(feature = "subsystem-scripting-csharp")]
    script_ragdoll_events:
        std::collections::BTreeMap<String, Vec<engine_script::GameplayRagdollEvent>>,
    /// Validated component queries drained from scripts during the current
    /// update. The runtime executes them against the active World right after
    /// the frame's commands apply and stages results for the next snapshot.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pending_component_queries: Vec<engine_script::OwnedGameplayComponentQuery>,
    /// Component query results computed after the latest script update. They
    /// are delivered to scripts with exactly one frame snapshot and then
    /// replaced, mirroring the frame-local physics query results.
    #[cfg(feature = "subsystem-scripting-csharp")]
    script_component_query_results:
        std::collections::BTreeMap<String, Vec<engine_script::GameplayComponentQueryResult>>,
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
        #[cfg(feature = "subsystem-animation")]
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
            render_environment: engine_renderer::EnvironmentSettings::default(),
            loaded_cooked_asset_ids: BTreeSet::new(),
            loaded_extension_asset_ids: BTreeMap::new(),
            runtime_mesh_table: runtime_mesh::RuntimeMeshTable::default(),
            synced_render_resources: SyncedRenderResources::default(),
            temporary_preview_textures: BTreeMap::new(),
            stream_loader: None,
            stream_budget: asset_stream::DEFAULT_STREAM_COMMIT_BUDGET,
            scene: None,
            world_slot: WorldSlot::new(),
            frame_timing: engine_renderer::FrameTimingTracker::new(),
            component_registry,
            asset_type_registry,
            render_extension_registry,
            debug_draw_registry,
            #[cfg(feature = "subsystem-animation")]
            animation_extensions,
            collector: DiagnosticsCollector::new(),
            #[cfg(feature = "subsystem-scripting-csharp")]
            script_engine: ScriptEngine::new(),
            #[cfg(feature = "subsystem-scripting-csharp")]
            script_host_name: "dotnet".to_string(),
            #[cfg(feature = "subsystem-scripting-csharp")]
            script_input_actions: std::collections::BTreeMap::new(),
            #[cfg(feature = "subsystem-scripting-csharp")]
            script_pointer: GameplayPointerSnapshot::default(),
            #[cfg(feature = "subsystem-scripting-csharp")]
            script_camera: None,
            #[cfg(feature = "subsystem-scripting-csharp")]
            pending_save_requests: Vec::new(),
            #[cfg(feature = "subsystem-scripting-csharp")]
            script_save_events: std::collections::BTreeMap::new(),
            #[cfg(feature = "subsystem-scripting-csharp")]
            script_logic_asset_results: std::collections::BTreeMap::new(),
            #[cfg(feature = "subsystem-scripting-csharp")]
            pending_scene_request: None,
            #[cfg(feature = "subsystem-scripting-csharp")]
            pending_physics_queries: Vec::new(),
            #[cfg(feature = "subsystem-scripting-csharp")]
            pending_physics_mutations: Vec::new(),
            #[cfg(feature = "subsystem-scripting-csharp")]
            pending_damage_requests: Vec::new(),
            #[cfg(feature = "subsystem-scripting-csharp")]
            script_damage_events: std::collections::BTreeMap::new(),
            #[cfg(feature = "subsystem-scripting-csharp")]
            pending_ragdoll_requests: Vec::new(),
            #[cfg(feature = "subsystem-scripting-csharp")]
            script_ragdoll_events: std::collections::BTreeMap::new(),
            #[cfg(feature = "subsystem-scripting-csharp")]
            pending_component_queries: Vec::new(),
            #[cfg(feature = "subsystem-scripting-csharp")]
            script_component_query_results: std::collections::BTreeMap::new(),
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
    #[cfg(feature = "subsystem-animation")]
    pub fn animation_extension_handles(&self) -> &engine_animation::AnimationExtensionHandles {
        &self.animation_extensions
    }

    /// Transactionally validate and load a scene into the canonical ECS World.
    pub fn load_scene(&mut self, scene: Scene) -> Result<(), Vec<Diagnostic>> {
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

        let world_scene = {
            // `engine.script` is scene-only metadata consumed by the script
            // subsystem. Keep it in the retained Scene, but do not ask the ECS
            // component registry to materialise it, even when the optional
            // scripting runtime is not compiled in. No other type is ignored.
            let mut world_scene = scene.clone();
            for entity in &mut world_scene.entities {
                entity.components.remove("engine.script");
            }
            world_scene
        };
        let world_scene = &world_scene;

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
            self.pending_physics_queries.clear();
            self.pending_physics_mutations.clear();
            self.pending_damage_requests.clear();
            self.script_damage_events.clear();
            self.pending_ragdoll_requests.clear();
            self.script_ragdoll_events.clear();
            self.pending_component_queries.clear();
            self.script_component_query_results.clear();
            self.pending_save_requests.clear();
            self.script_save_events.clear();
            self.script_logic_asset_results.clear();
            self.attach_scene_scripts(&scene);
        }

        self.scene = Some(scene);
        Ok(())
    }

    /// Directly set an existing ECS World as the runtime's active world.
    ///
    /// This is the preferred entry point when building a World manually
    /// via `World::new()` + `create_entity()` + `add_component()`.
    /// Unlike `load_scene` it avoids the `to_scene()/from_scene()`
    /// serialisation round-trip.
    ///
    /// The world must contain at least one enabled [`Camera`] component
    /// and at least one enabled [`Renderable`] component for extraction
    /// to produce a valid frame.
    pub fn set_world(&mut self, mut world: World) {
        // A caller-provided registry is authoritative. Otherwise install the
        // runtime registry before serialising so external components survive
        // the Scene snapshot used by inspection and script attachment.
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
            self.pending_physics_queries.clear();
            self.pending_physics_mutations.clear();
            self.pending_damage_requests.clear();
            self.script_damage_events.clear();
            self.pending_ragdoll_requests.clear();
            self.script_ragdoll_events.clear();
            self.pending_component_queries.clear();
            self.script_component_query_results.clear();
            self.pending_save_requests.clear();
            self.script_save_events.clear();
            self.script_logic_asset_results.clear();
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

    /// Immutable access to the loaded scene (if any).
    pub fn scene_ref(&self) -> Option<&Scene> {
        self.scene.as_ref()
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

    /// Return the concrete managed behaviour classes verified by the active
    /// script hosts when their assemblies were loaded.
    ///
    /// This is the sole editor-facing discovery path: callers receive no
    /// source-derived or conventional default class names.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn verified_script_classes(&self) -> Vec<engine_script::VerifiedScriptClass> {
        self.script_engine.verified_classes()
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
        self.pending_physics_queries.clear();
        self.pending_physics_mutations.clear();
        self.pending_damage_requests.clear();
        self.script_damage_events.clear();
        self.pending_ragdoll_requests.clear();
        self.script_ragdoll_events.clear();
        self.pending_component_queries.clear();
        self.script_component_query_results.clear();
        self.pending_save_requests.clear();
        self.script_save_events.clear();
        self.script_logic_asset_results.clear();
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

    /// Set renderer-consistent pointer and camera data for the next script
    /// lifecycle call.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn set_script_view_context(
        &mut self,
        pointer: GameplayPointerSnapshot,
        camera: Option<GameplayCameraSnapshot>,
    ) {
        self.script_pointer = pointer;
        self.script_camera = camera;
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

    /// Take the validated physics queries drained from scripts during the
    /// current update, leaving the queue empty.
    ///
    /// The owning game loop executes these against its physics world at the
    /// frame boundary and delivers results in the next frame snapshot.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn take_pending_physics_queries(
        &mut self,
    ) -> Vec<engine_script::OwnedGameplayPhysicsQuery> {
        std::mem::take(&mut self.pending_physics_queries)
    }

    /// Take the validated forces and impulses drained from scripts during the
    /// current update, leaving the queue empty.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn take_pending_physics_mutations(
        &mut self,
    ) -> Vec<engine_script::OwnedGameplayPhysicsMutation> {
        std::mem::take(&mut self.pending_physics_mutations)
    }

    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn take_pending_damage_requests(
        &mut self,
    ) -> Vec<engine_script::OwnedGameplayDamageRequest> {
        std::mem::take(&mut self.pending_damage_requests)
    }

    #[cfg(all(feature = "subsystem-scripting-csharp", feature = "subsystem-physics"))]
    pub(crate) fn push_script_damage_event(
        &mut self,
        entity_id: String,
        event: GameplayDamageEvent,
    ) {
        self.script_damage_events
            .entry(entity_id)
            .or_default()
            .push(event);
    }

    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn take_pending_ragdoll_requests(
        &mut self,
    ) -> Vec<engine_script::OwnedGameplayRagdollRequest> {
        std::mem::take(&mut self.pending_ragdoll_requests)
    }

    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn take_pending_save_requests(&mut self) -> Vec<engine_script::OwnedGameplaySaveRequest> {
        std::mem::take(&mut self.pending_save_requests)
    }

    #[cfg(feature = "subsystem-scripting-csharp")]
    pub(crate) fn push_script_save_event(&mut self, entity_id: String, event: GameplaySaveEvent) {
        self.script_save_events
            .entry(entity_id)
            .or_default()
            .push(event);
    }

    #[cfg(all(
        feature = "subsystem-scripting-csharp",
        feature = "subsystem-physics",
        feature = "subsystem-animation"
    ))]
    pub(crate) fn push_script_ragdoll_event(
        &mut self,
        entity_id: String,
        event: GameplayRagdollEvent,
    ) {
        self.script_ragdoll_events
            .entry(entity_id)
            .or_default()
            .push(event);
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
        self.tick_scripts_with_frame_input_ui_and_physics_queries(
            dt,
            input_actions,
            input_transitions,
            physics_events,
            ui_events,
            &std::collections::BTreeMap::new(),
        );
    }

    /// Tick scripts with the complete frame snapshot, including retained UI
    /// clicks and the physics query results computed by the owning
    /// [`GameLoop`](crate::game_loop::GameLoop) after the previous update.
    ///
    /// Query results are frame snapshots. Callers must pass an empty map on
    /// frames without freshly computed results so stale answers cannot be
    /// observed.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn tick_scripts_with_frame_input_ui_and_physics_queries(
        &mut self,
        dt: f32,
        input_actions: &std::collections::BTreeMap<String, GameplayInputValue>,
        input_transitions: &GameplayInputTransitions,
        physics_events: &std::collections::BTreeMap<String, Vec<GameplayPhysicsEvent>>,
        ui_events: &[GameplayUiEvent],
        physics_query_results: &std::collections::BTreeMap<
            String,
            Vec<engine_script::GameplayPhysicsQueryResult>,
        >,
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
            physics_query_results,
        );
        let mut diagnostics = self.script_engine.set_gameplay_contexts(&contexts);
        self.script_damage_events.clear();
        self.script_ragdoll_events.clear();
        self.script_save_events.clear();
        self.script_logic_asset_results.clear();
        diagnostics.extend(self.script_engine.update(dt));
        let (commands, command_diagnostics) = self.script_engine.drain_gameplay_commands();
        diagnostics.extend(command_diagnostics);
        diagnostics.extend(self.apply_script_gameplay_commands(commands));
        // Component queries issued during this update snapshot the world
        // after this frame's commands applied, so next frame's results
        // observe same-frame component writes.
        diagnostics.extend(self.execute_script_component_queries());
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
            &std::collections::BTreeMap::new(),
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
        // Component queries issued from OnCreate are answered with the first
        // OnUpdate snapshot instead of waiting for a full extra frame.
        command_diags.extend(self.execute_script_component_queries());
        self.collector.push_script_diags(command_diags);
    }

    #[cfg(feature = "subsystem-scripting-csharp")]
    fn script_gameplay_contexts(
        &self,
        input_actions: &std::collections::BTreeMap<String, GameplayInputValue>,
        input_transitions: &GameplayInputTransitions,
        physics_events: &std::collections::BTreeMap<String, Vec<GameplayPhysicsEvent>>,
        ui_events: &[GameplayUiEvent],
        physics_query_results: &std::collections::BTreeMap<
            String,
            Vec<engine_script::GameplayPhysicsQueryResult>,
        >,
    ) -> std::collections::BTreeMap<String, GameplayContext> {
        let entity_ids = self
            .script_engine
            .managers()
            .iter()
            .flat_map(|manager| manager.iter_instances().map(|(entity_id, _, _)| entity_id))
            .map(str::to_owned)
            .collect::<std::collections::BTreeSet<_>>();

        let entities = self.script_gameplay_entity_snapshots();
        let world_origin = self
            .world_slot
            .with_world(|world| world.world_origin())
            .unwrap_or([0.0; 3]);

        entity_ids
            .into_iter()
            .map(|entity_id| {
                let context = GameplayContext {
                    script_api: engine_script::GAMEPLAY_SCRIPT_API_SCHEMA.to_owned(),
                    transform: entities
                        .get(&entity_id)
                        .and_then(|snapshot| snapshot.transform.clone()),
                    entity_id: entity_id.clone(),
                    world_origin,
                    input_actions: input_actions.clone(),
                    input_transitions: input_transitions.clone(),
                    pointer: self.script_pointer.clone(),
                    camera: self.script_camera.clone(),
                    save_events: self
                        .script_save_events
                        .get(&entity_id)
                        .cloned()
                        .unwrap_or_default(),
                    logic_asset_results: self
                        .script_logic_asset_results
                        .get(&entity_id)
                        .cloned()
                        .unwrap_or_default(),
                    physics_events: physics_events.get(&entity_id).cloned().unwrap_or_default(),
                    damage_events: self
                        .script_damage_events
                        .get(&entity_id)
                        .cloned()
                        .unwrap_or_default(),
                    ragdoll_events: self
                        .script_ragdoll_events
                        .get(&entity_id)
                        .cloned()
                        .unwrap_or_default(),
                    physics_query_results: physics_query_results
                        .get(&entity_id)
                        .cloned()
                        .unwrap_or_default(),
                    component_query_results: self
                        .script_component_query_results
                        .get(&entity_id)
                        .cloned()
                        .unwrap_or_default(),
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
        self.apply_script_gameplay_commands_with_depth(commands, 0)
    }

    /// Apply validated script commands at the frame boundary.
    ///
    /// `depth` bounds the synchronous `Scene.Spawn` → `OnCreate` → command
    /// chain: prefabs spawned from a spawned script's `OnCreate` are applied
    /// recursively so each new instance completes its lifecycle within the
    /// same frame boundary, up to [`MAX_SCRIPT_SPAWN_DEPTH`].
    #[cfg(feature = "subsystem-scripting-csharp")]
    fn apply_script_gameplay_commands_with_depth(
        &mut self,
        commands: Vec<engine_script::OwnedGameplayCommand>,
        depth: usize,
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
                GameplayCommand::SpawnPrefab {
                    prefab_id,
                    translation,
                } => {
                    if !script_command_owner_exists(&self.world_slot, &entity_id) {
                        diagnostics.push(script_owner_missing_diagnostic(
                            &entity_id,
                            &format!("spawn prefab '{prefab_id}'"),
                        ));
                        continue;
                    }
                    if let Err(reason) = engine_script::validate_prefab_id(&prefab_id) {
                        let mut diagnostic = Diagnostic::new(
                            "SCRIPT_PREFAB_ID_INVALID",
                            DiagnosticSeverity::Error,
                            "script",
                            format!("script entity '{entity_id}' {reason}"),
                        );
                        diagnostic.entity = Some(entity_id);
                        diagnostics.push(diagnostic);
                        continue;
                    }
                    if translation.is_some_and(|translation| {
                        !translation.iter().all(|value| value.is_finite())
                    }) {
                        let mut diagnostic = Diagnostic::new(
                            "SCRIPT_PREFAB_TRANSFORM_INVALID",
                            DiagnosticSeverity::Error,
                            "script",
                            format!(
                                "script entity '{entity_id}' requested prefab '{prefab_id}' with a non-finite spawn translation"
                            ),
                        );
                        diagnostic.entity = Some(entity_id);
                        diagnostics.push(diagnostic);
                        continue;
                    }
                    self.spawn_script_prefab(
                        &entity_id,
                        &prefab_id,
                        translation,
                        &mut diagnostics,
                        depth,
                    );
                }
                GameplayCommand::ApplyDamage {
                    entity_id: target_id,
                    amount,
                    damage_kind,
                    hit_position,
                    impulse,
                } => {
                    if !script_command_owner_exists(&self.world_slot, &entity_id) {
                        diagnostics
                            .push(script_owner_missing_diagnostic(&entity_id, "apply damage"));
                        continue;
                    }
                    let command = GameplayCommand::ApplyDamage {
                        entity_id: target_id.clone(),
                        amount,
                        damage_kind,
                        hit_position,
                        impulse,
                    };
                    if let Err(reason) = command.validate() {
                        diagnostics.push(script_component_diagnostic(
                            "SCRIPT_DAMAGE_INVALID",
                            &entity_id,
                            format!(
                                "script entity '{entity_id}' produced an invalid damage request: {reason}"
                            ),
                        ));
                        continue;
                    }
                    let target_exists = self
                        .world_slot
                        .with_world(|world| world.entity_by_persistent_id(&target_id).is_some())
                        .unwrap_or(false);
                    if !target_exists {
                        diagnostics.push(script_component_diagnostic(
                            "SCRIPT_DAMAGE_TARGET_MISSING",
                            &entity_id,
                            format!(
                                "script entity '{entity_id}' requested damage for unknown entity '{target_id}'"
                            ),
                        ));
                        continue;
                    }
                    if self.pending_damage_requests.len()
                        >= engine_script::MAX_PENDING_DAMAGE_REQUESTS
                    {
                        diagnostics.push(script_component_diagnostic(
                            "SCRIPT_DAMAGE_OVERFLOW",
                            &entity_id,
                            format!(
                                "script entity '{entity_id}' exceeded the pending damage budget of {} per frame",
                                engine_script::MAX_PENDING_DAMAGE_REQUESTS
                            ),
                        ));
                        continue;
                    }
                    self.pending_damage_requests
                        .push(engine_script::OwnedGameplayDamageRequest {
                            owner_entity_id: entity_id,
                            target_entity_id: target_id,
                            amount,
                            damage_kind,
                            hit_position,
                            impulse,
                        });
                }
                GameplayCommand::SetRagdoll {
                    entity_id: target_id,
                    active,
                    recovery_duration,
                    impulse,
                } => {
                    if !script_command_owner_exists(&self.world_slot, &entity_id) {
                        diagnostics.push(script_owner_missing_diagnostic(
                            &entity_id,
                            "change ragdoll ownership",
                        ));
                        continue;
                    }
                    let command = GameplayCommand::SetRagdoll {
                        entity_id: target_id.clone(),
                        active,
                        recovery_duration,
                        impulse,
                    };
                    if let Err(reason) = command.validate() {
                        diagnostics.push(script_component_diagnostic(
                            "SCRIPT_RAGDOLL_INVALID",
                            &entity_id,
                            format!(
                                "script entity '{entity_id}' produced an invalid ragdoll request: {reason}"
                            ),
                        ));
                        continue;
                    }
                    let target_exists = self
                        .world_slot
                        .with_world(|world| world.entity_by_persistent_id(&target_id).is_some())
                        .unwrap_or(false);
                    if !target_exists {
                        diagnostics.push(script_component_diagnostic(
                            "SCRIPT_RAGDOLL_TARGET_MISSING",
                            &entity_id,
                            format!(
                                "script entity '{entity_id}' requested ragdoll ownership for unknown entity '{target_id}'"
                            ),
                        ));
                        continue;
                    }
                    if self.pending_ragdoll_requests.len()
                        >= engine_script::MAX_PENDING_RAGDOLL_REQUESTS
                    {
                        diagnostics.push(script_component_diagnostic(
                            "SCRIPT_RAGDOLL_OVERFLOW",
                            &entity_id,
                            format!(
                                "script entity '{entity_id}' exceeded the pending ragdoll budget of {} per frame",
                                engine_script::MAX_PENDING_RAGDOLL_REQUESTS
                            ),
                        ));
                        continue;
                    }
                    self.pending_ragdoll_requests.push(
                        engine_script::OwnedGameplayRagdollRequest {
                            owner_entity_id: entity_id,
                            target_entity_id: target_id,
                            active,
                            recovery_duration,
                            impulse,
                        },
                    );
                }
                GameplayCommand::CharacterControl {
                    entity_id: target_id,
                    direction,
                    jump,
                    speed,
                } => {
                    if !script_command_owner_exists(&self.world_slot, &entity_id) {
                        diagnostics.push(script_owner_missing_diagnostic(
                            &entity_id,
                            "control a character",
                        ));
                        continue;
                    }
                    let command = GameplayCommand::CharacterControl {
                        entity_id: target_id.clone(),
                        direction,
                        jump,
                        speed,
                    };
                    if let Err(reason) = command.validate() {
                        diagnostics.push(script_component_diagnostic(
                            "SCRIPT_CHARACTER_CONTROL_INVALID",
                            &entity_id,
                            format!(
                                "script entity '{entity_id}' produced invalid character control: {reason}"
                            ),
                        ));
                        continue;
                    }
                    let applied = self
                        .world_slot
                        .with_world_mut(|world| {
                            let Some(target) = world.entity_by_persistent_id(&target_id) else {
                                return false;
                            };
                            let Some(controller) =
                                world.get_mut::<engine_character::CharacterController>(target)
                            else {
                                return false;
                            };
                            controller.push_command(engine_character::CharacterCommand {
                                direction: glam::Vec3::from(direction),
                                desired_speed: speed.unwrap_or(0.0),
                                jump_requested: jump,
                            });
                            true
                        })
                        .unwrap_or(false);
                    if !applied {
                        diagnostics.push(script_component_diagnostic(
                            "SCRIPT_CHARACTER_CONTROL_TARGET_MISSING",
                            &entity_id,
                            format!(
                                "script entity '{entity_id}' requested character control for '{target_id}', but it has no CharacterController"
                            ),
                        ));
                    }
                }
                GameplayCommand::Ui { command } => {
                    if !script_command_owner_exists(&self.world_slot, &entity_id) {
                        diagnostics.push(script_owner_missing_diagnostic(
                            &entity_id,
                            "mutate runtime UI",
                        ));
                        continue;
                    }
                    #[cfg(feature = "subsystem-ui")]
                    apply_script_ui_command(
                        &self.world_slot,
                        &entity_id,
                        command,
                        &mut diagnostics,
                    );
                    #[cfg(not(feature = "subsystem-ui"))]
                    {
                        let _ = command;
                        let mut diagnostic = Diagnostic::new(
                            "SCRIPT_UI_UNAVAILABLE",
                            DiagnosticSeverity::Error,
                            "script",
                            format!(
                                "script entity '{entity_id}' requested runtime UI, but engine-core was built without subsystem-ui"
                            ),
                        );
                        diagnostic.entity = Some(entity_id);
                        diagnostics.push(diagnostic);
                    }
                }
                GameplayCommand::PhysicsQuery { query } => {
                    if !script_command_owner_exists(&self.world_slot, &entity_id) {
                        diagnostics.push(script_owner_missing_diagnostic(
                            &entity_id,
                            "request a physics query",
                        ));
                        continue;
                    }
                    if let Err(reason) = query.validate() {
                        let mut diagnostic = Diagnostic::new(
                            "SCRIPT_PHYSICS_QUERY_INVALID",
                            DiagnosticSeverity::Error,
                            "script",
                            format!(
                                "script entity '{entity_id}' produced an invalid physics query: {reason}"
                            ),
                        );
                        diagnostic.entity = Some(entity_id);
                        diagnostics.push(diagnostic);
                        continue;
                    }
                    // `GameplayPhysicsQuery::validate` cannot know the world;
                    // reject exclusion targets that name no existing entity.
                    if let Some(excluded) = query
                        .filter()
                        .and_then(|filter| filter.exclude_entity.as_deref())
                    {
                        let excluded_exists = self
                            .world_slot
                            .with_world(|world| world.entity_by_persistent_id(excluded).is_some())
                            .unwrap_or(false);
                        if !excluded_exists {
                            let mut diagnostic = Diagnostic::new(
                                "SCRIPT_PHYSICS_QUERY_INVALID",
                                DiagnosticSeverity::Error,
                                "script",
                                format!(
                                    "script entity '{entity_id}' produced an invalid physics query: unknown exclude_entity id '{excluded}'"
                                ),
                            );
                            diagnostic.entity = Some(entity_id);
                            diagnostics.push(diagnostic);
                            continue;
                        }
                    }
                    if self.pending_physics_queries.len()
                        >= engine_script::MAX_PENDING_PHYSICS_QUERIES
                    {
                        let mut diagnostic = Diagnostic::new(
                            "SCRIPT_PHYSICS_QUERY_OVERFLOW",
                            DiagnosticSeverity::Error,
                            "script",
                            format!(
                                "script entity '{entity_id}' exceeded the pending physics query budget of {} per frame",
                                engine_script::MAX_PENDING_PHYSICS_QUERIES
                            ),
                        );
                        diagnostic.entity = Some(entity_id);
                        diagnostics.push(diagnostic);
                        continue;
                    }
                    self.pending_physics_queries
                        .push(engine_script::OwnedGameplayPhysicsQuery { entity_id, query });
                }
                GameplayCommand::PhysicsMutation { mutation } => {
                    if !script_command_owner_exists(&self.world_slot, &entity_id) {
                        diagnostics.push(script_owner_missing_diagnostic(
                            &entity_id,
                            "mutate a rigid body",
                        ));
                        continue;
                    }
                    if let Err(reason) = mutation.validate() {
                        let mut diagnostic = Diagnostic::new(
                            "SCRIPT_PHYSICS_MUTATION_INVALID",
                            DiagnosticSeverity::Error,
                            "script",
                            format!(
                                "script entity '{entity_id}' produced an invalid physics mutation: {reason}"
                            ),
                        );
                        diagnostic.entity = Some(entity_id);
                        diagnostics.push(diagnostic);
                        continue;
                    }
                    let missing_target = self
                        .world_slot
                        .with_world(|world| {
                            mutation
                                .required_existing_entity_ids()
                                .into_iter()
                                .find(|target_id| {
                                    world.entity_by_persistent_id(target_id).is_none()
                                })
                                .map(str::to_owned)
                        })
                        .flatten();
                    if let Some(target_id) = missing_target {
                        let mut diagnostic = Diagnostic::new(
                            "SCRIPT_PHYSICS_MUTATION_TARGET_MISSING",
                            DiagnosticSeverity::Error,
                            "script",
                            format!(
                                "script entity '{entity_id}' requested a physics mutation for unknown entity '{target_id}'"
                            ),
                        );
                        diagnostic.entity = Some(entity_id);
                        diagnostics.push(diagnostic);
                        continue;
                    }
                    if self.pending_physics_mutations.len()
                        >= engine_script::MAX_PENDING_PHYSICS_MUTATIONS
                    {
                        let mut diagnostic = Diagnostic::new(
                            "SCRIPT_PHYSICS_MUTATION_OVERFLOW",
                            DiagnosticSeverity::Error,
                            "script",
                            format!(
                                "script entity '{entity_id}' exceeded the pending physics mutation budget of {} per frame",
                                engine_script::MAX_PENDING_PHYSICS_MUTATIONS
                            ),
                        );
                        diagnostic.entity = Some(entity_id);
                        diagnostics.push(diagnostic);
                        continue;
                    }
                    self.pending_physics_mutations.push(
                        engine_script::OwnedGameplayPhysicsMutation {
                            owner_entity_id: entity_id,
                            mutation,
                        },
                    );
                }
                GameplayCommand::ComponentQuery { query } => {
                    if !script_command_owner_exists(&self.world_slot, &entity_id) {
                        diagnostics.push(script_owner_missing_diagnostic(
                            &entity_id,
                            "query a component",
                        ));
                        continue;
                    }
                    if let Err(reason) = query.validate() {
                        diagnostics.push(script_component_diagnostic(
                            "SCRIPT_COMPONENT_QUERY_INVALID",
                            &entity_id,
                            format!(
                                "script entity '{entity_id}' produced an invalid component query: {reason}"
                            ),
                        ));
                        continue;
                    }
                    // Registry-driven access check. `None` means no World is
                    // active; the query is still queued and the executor
                    // reports SCRIPT_WORLD_MISSING instead.
                    let resolution = self.world_slot.with_world(|world| {
                        (
                            script_components::resolve_script_component(
                                world,
                                &query.component_type,
                            ),
                            script_components::supported_script_component_types(world),
                        )
                    });
                    if let Some((
                        script_components::ScriptComponentResolution::Unsupported,
                        supported,
                    )) = resolution
                    {
                        diagnostics.push(script_component_diagnostic(
                            "SCRIPT_COMPONENT_UNKNOWN",
                            &entity_id,
                            format!(
                                "script entity '{entity_id}' queried component '{}' on entity '{}', but that type is not script-accessible; {}",
                                query.component_type,
                                query.entity_id,
                                supported_script_component_description(&supported)
                            ),
                        ));
                        continue;
                    }
                    if self.pending_component_queries.len()
                        >= engine_script::MAX_PENDING_COMPONENT_QUERIES
                    {
                        diagnostics.push(script_component_diagnostic(
                            "SCRIPT_COMPONENT_QUERY_OVERFLOW",
                            &entity_id,
                            format!(
                                "script entity '{entity_id}' exceeded the pending component query budget of {} per frame",
                                engine_script::MAX_PENDING_COMPONENT_QUERIES
                            ),
                        ));
                        continue;
                    }
                    self.pending_component_queries
                        .push(engine_script::OwnedGameplayComponentQuery { entity_id, query });
                }
                GameplayCommand::SetComponent {
                    entity_id: target_id,
                    component_type,
                    fields,
                } => {
                    if !script_command_owner_exists(&self.world_slot, &entity_id) {
                        diagnostics.push(script_owner_missing_diagnostic(
                            &entity_id,
                            "write a component",
                        ));
                        continue;
                    }
                    let wire_validation = engine_script::validate_entity_id(&target_id)
                        .and_then(|_| engine_script::validate_component_type_key(&component_type))
                        .and_then(|_| engine_script::validate_component_fields(&fields));
                    if let Err(reason) = wire_validation {
                        diagnostics.push(script_component_diagnostic(
                            "SCRIPT_COMPONENT_INVALID",
                            &entity_id,
                            format!(
                                "script entity '{entity_id}' produced an invalid set_component for entity '{target_id}': {reason}"
                            ),
                        ));
                        continue;
                    }
                    let resolution = self.world_slot.with_world(|world| {
                        (
                            script_components::resolve_script_component(world, &component_type),
                            script_components::supported_script_component_types(world),
                        )
                    });
                    match resolution {
                        Some((script_components::ScriptComponentResolution::ReadWrite, _)) => {}
                        Some((script_components::ScriptComponentResolution::ReadOnly, _)) => {
                            diagnostics.push(script_component_diagnostic(
                                "SCRIPT_COMPONENT_READ_ONLY",
                                &entity_id,
                                format!(
                                    "script entity '{entity_id}' tried to write component '{component_type}' on entity '{target_id}', but that component is read-only for scripts; query it with Components.Query instead"
                                ),
                            ));
                            continue;
                        }
                        Some((
                            script_components::ScriptComponentResolution::Unsupported,
                            supported,
                        )) => {
                            diagnostics.push(script_component_diagnostic(
                                "SCRIPT_COMPONENT_UNKNOWN",
                                &entity_id,
                                format!(
                                    "script entity '{entity_id}' tried to write component '{component_type}' on entity '{target_id}', but that type is not script-accessible; {}",
                                    supported_script_component_description(&supported)
                                ),
                            ));
                            continue;
                        }
                        None => {
                            diagnostics.push(script_component_diagnostic(
                                "SCRIPT_WORLD_MISSING",
                                &entity_id,
                                format!(
                                    "script entity '{entity_id}' component write for entity '{target_id}' could not be applied because no World is active"
                                ),
                            ));
                            continue;
                        }
                    }
                    let outcome = self.world_slot.with_world_mut(|world| {
                        script_components::apply_script_component_write(
                            world,
                            &target_id,
                            &component_type,
                            &fields,
                        )
                    });
                    match outcome {
                        Some(Ok(())) => {}
                        Some(Err(script_components::ScriptComponentWriteError::UnknownEntity)) => {
                            diagnostics.push(script_component_diagnostic(
                                "SCRIPT_COMPONENT_TARGET_MISSING",
                                &entity_id,
                                format!(
                                    "script entity '{entity_id}' wrote component '{component_type}' for entity '{target_id}', but that entity does not exist"
                                ),
                            ));
                        }
                        Some(Err(script_components::ScriptComponentWriteError::ReadOnly)) => {
                            diagnostics.push(script_component_diagnostic(
                                "SCRIPT_COMPONENT_READ_ONLY",
                                &entity_id,
                                format!(
                                    "script entity '{entity_id}' tried to write component '{component_type}' on entity '{target_id}', but that component is read-only for scripts; query it with Components.Query instead"
                                ),
                            ));
                        }
                        Some(Err(
                            script_components::ScriptComponentWriteError::PayloadRejected {
                                rejected,
                                known,
                            },
                        )) => {
                            diagnostics.push(script_component_diagnostic(
                                "SCRIPT_COMPONENT_PAYLOAD_INVALID",
                                &entity_id,
                                format!(
                                    "script entity '{entity_id}' wrote component '{component_type}' on entity '{target_id}' with fields the component rejected: {}; known fields: {}",
                                    rejected.join(", "),
                                    known.join(", ")
                                ),
                            ));
                        }
                        Some(Err(
                            script_components::ScriptComponentWriteError::ValidationFailed {
                                message,
                            },
                        )) => {
                            diagnostics.push(script_component_diagnostic(
                                "SCRIPT_COMPONENT_VALIDATION_FAILED",
                                &entity_id,
                                format!(
                                    "script entity '{entity_id}' wrote invalid component '{component_type}' parameters on entity '{target_id}': {message}"
                                ),
                            ));
                        }
                        Some(Err(script_components::ScriptComponentWriteError::Unsupported)) => {
                            let supported = self
                                .world_slot
                                .with_world(script_components::supported_script_component_types)
                                .unwrap_or_default();
                            diagnostics.push(script_component_diagnostic(
                                "SCRIPT_COMPONENT_UNKNOWN",
                                &entity_id,
                                format!(
                                    "script entity '{entity_id}' tried to write component '{component_type}' on entity '{target_id}', but that type is not script-accessible; {}",
                                    supported_script_component_description(&supported)
                                ),
                            ));
                        }
                        Some(Err(script_components::ScriptComponentWriteError::ApplyFailed)) => {
                            diagnostics.push(script_component_diagnostic(
                                "SCRIPT_COMPONENT_APPLY_FAILED",
                                &entity_id,
                                format!(
                                    "script entity '{entity_id}' wrote component '{component_type}' on entity '{target_id}', but the validated component could not be committed to storage"
                                ),
                            ));
                        }
                        None => {
                            diagnostics.push(script_component_diagnostic(
                                "SCRIPT_WORLD_MISSING",
                                &entity_id,
                                format!(
                                    "script entity '{entity_id}' component write for entity '{target_id}' could not be applied because no World is active"
                                ),
                            ));
                        }
                    }
                }
                GameplayCommand::PlayAnimation {
                    entity_id: target_id,
                    clip_asset,
                    looping,
                    speed,
                    restart,
                } => {
                    if !script_command_owner_exists(&self.world_slot, &entity_id) {
                        diagnostics.push(script_owner_missing_diagnostic(
                            &entity_id,
                            "play an animation",
                        ));
                        continue;
                    }
                    let command = GameplayCommand::PlayAnimation {
                        entity_id: target_id.clone(),
                        clip_asset: clip_asset.clone(),
                        looping,
                        speed,
                        restart,
                    };
                    if let Err(reason) = command.validate() {
                        diagnostics.push(script_component_diagnostic(
                            "SCRIPT_ANIMATION_INVALID",
                            &entity_id,
                            format!(
                                "script entity '{entity_id}' produced an invalid animation request: {reason}"
                            ),
                        ));
                        continue;
                    }
                    apply_script_animation_command(
                        &self.world_slot,
                        &entity_id,
                        &target_id,
                        ScriptAnimationCommand::PlayClip {
                            clip_asset,
                            looping,
                            speed,
                            restart,
                        },
                        &mut diagnostics,
                    );
                }
                GameplayCommand::SetAnimationParameter {
                    entity_id: target_id,
                    name,
                    value,
                } => {
                    if !script_command_owner_exists(&self.world_slot, &entity_id) {
                        diagnostics.push(script_owner_missing_diagnostic(
                            &entity_id,
                            "set an animation parameter",
                        ));
                        continue;
                    }
                    let command = GameplayCommand::SetAnimationParameter {
                        entity_id: target_id.clone(),
                        name: name.clone(),
                        value: value.clone(),
                    };
                    if let Err(reason) = command.validate() {
                        diagnostics.push(script_component_diagnostic(
                            "SCRIPT_ANIMATION_INVALID",
                            &entity_id,
                            format!(
                                "script entity '{entity_id}' produced an invalid animation parameter: {reason}"
                            ),
                        ));
                        continue;
                    }
                    apply_script_animation_command(
                        &self.world_slot,
                        &entity_id,
                        &target_id,
                        ScriptAnimationCommand::SetParameter { name, value },
                        &mut diagnostics,
                    );
                }
                GameplayCommand::TransitionAnimationState {
                    entity_id: target_id,
                    state,
                } => {
                    if !script_command_owner_exists(&self.world_slot, &entity_id) {
                        diagnostics.push(script_owner_missing_diagnostic(
                            &entity_id,
                            "transition an animation state",
                        ));
                        continue;
                    }
                    let command = GameplayCommand::TransitionAnimationState {
                        entity_id: target_id.clone(),
                        state: state.clone(),
                    };
                    if let Err(reason) = command.validate() {
                        diagnostics.push(script_component_diagnostic(
                            "SCRIPT_ANIMATION_INVALID",
                            &entity_id,
                            format!(
                                "script entity '{entity_id}' produced an invalid animation transition: {reason}"
                            ),
                        ));
                        continue;
                    }
                    apply_script_animation_command(
                        &self.world_slot,
                        &entity_id,
                        &target_id,
                        ScriptAnimationCommand::Transition { state },
                        &mut diagnostics,
                    );
                }
                GameplayCommand::SetAnimationPlaying {
                    entity_id: target_id,
                    playing,
                } => {
                    if !script_command_owner_exists(&self.world_slot, &entity_id) {
                        diagnostics.push(script_owner_missing_diagnostic(
                            &entity_id,
                            "change animation playback",
                        ));
                        continue;
                    }
                    if let Err(reason) = (GameplayCommand::SetAnimationPlaying {
                        entity_id: target_id.clone(),
                        playing,
                    })
                    .validate()
                    {
                        diagnostics.push(script_component_diagnostic(
                            "SCRIPT_ANIMATION_INVALID",
                            &entity_id,
                            format!(
                                "script entity '{entity_id}' produced an invalid animation request: {reason}"
                            ),
                        ));
                        continue;
                    }
                    apply_script_animation_command(
                        &self.world_slot,
                        &entity_id,
                        &target_id,
                        ScriptAnimationCommand::SetPlaying { playing },
                        &mut diagnostics,
                    );
                }
                GameplayCommand::SetMorphWeights {
                    entity_id: target_id,
                    weights,
                } => {
                    if !script_command_owner_exists(&self.world_slot, &entity_id) {
                        diagnostics.push(script_owner_missing_diagnostic(
                            &entity_id,
                            "change morph weights",
                        ));
                        continue;
                    }
                    let command = GameplayCommand::SetMorphWeights {
                        entity_id: target_id.clone(),
                        weights: weights.clone(),
                    };
                    if let Err(reason) = command.validate() {
                        diagnostics.push(script_component_diagnostic(
                            "SCRIPT_ANIMATION_INVALID",
                            &entity_id,
                            format!(
                                "script entity '{entity_id}' produced invalid morph weights: {reason}"
                            ),
                        ));
                        continue;
                    }
                    apply_script_morph_weights(
                        &self.world_slot,
                        &entity_id,
                        &target_id,
                        weights,
                        &mut diagnostics,
                    );
                }
                GameplayCommand::SaveCheckpoint { slot, state_json } => {
                    if !script_command_owner_exists(&self.world_slot, &entity_id) {
                        diagnostics.push(script_owner_missing_diagnostic(
                            &entity_id,
                            "save a checkpoint",
                        ));
                        continue;
                    }
                    let command = GameplayCommand::SaveCheckpoint {
                        slot: slot.clone(),
                        state_json: state_json.clone(),
                    };
                    if let Err(reason) = command.validate() {
                        diagnostics.push(script_component_diagnostic(
                            "SCRIPT_SAVE_INVALID",
                            &entity_id,
                            format!(
                                "script entity '{entity_id}' produced an invalid save request: {reason}"
                            ),
                        ));
                        continue;
                    }
                    if self.pending_save_requests.len() >= engine_script::MAX_PENDING_SAVE_REQUESTS
                    {
                        diagnostics.push(script_component_diagnostic(
                            "SCRIPT_SAVE_OVERFLOW",
                            &entity_id,
                            format!(
                                "script entity '{entity_id}' exceeded the pending save request budget of {} per frame",
                                engine_script::MAX_PENDING_SAVE_REQUESTS
                            ),
                        ));
                        continue;
                    }
                    self.pending_save_requests
                        .push(engine_script::OwnedGameplaySaveRequest {
                            owner_entity_id: entity_id,
                            slot,
                            operation: engine_script::GameplaySaveOperation::Save { state_json },
                        });
                }
                GameplayCommand::LoadCheckpoint { slot } => {
                    if !script_command_owner_exists(&self.world_slot, &entity_id) {
                        diagnostics.push(script_owner_missing_diagnostic(
                            &entity_id,
                            "load a checkpoint",
                        ));
                        continue;
                    }
                    if let Err(reason) = engine_script::validate_save_slot(&slot) {
                        diagnostics.push(script_component_diagnostic(
                            "SCRIPT_SAVE_INVALID",
                            &entity_id,
                            format!(
                                "script entity '{entity_id}' produced an invalid load request: {reason}"
                            ),
                        ));
                        continue;
                    }
                    if self.pending_save_requests.len() >= engine_script::MAX_PENDING_SAVE_REQUESTS
                    {
                        diagnostics.push(script_component_diagnostic(
                            "SCRIPT_SAVE_OVERFLOW",
                            &entity_id,
                            format!(
                                "script entity '{entity_id}' exceeded the pending save request budget of {} per frame",
                                engine_script::MAX_PENDING_SAVE_REQUESTS
                            ),
                        ));
                        continue;
                    }
                    self.pending_save_requests
                        .push(engine_script::OwnedGameplaySaveRequest {
                            owner_entity_id: entity_id,
                            slot,
                            operation: engine_script::GameplaySaveOperation::Load,
                        });
                }
                GameplayCommand::QueryLogicAsset { query_id, asset_id } => {
                    if !script_command_owner_exists(&self.world_slot, &entity_id) {
                        diagnostics.push(script_owner_missing_diagnostic(
                            &entity_id,
                            "query a logic asset",
                        ));
                        continue;
                    }
                    let query_count = self
                        .script_logic_asset_results
                        .values()
                        .map(Vec::len)
                        .sum::<usize>();
                    if query_count >= engine_script::MAX_PENDING_LOGIC_ASSET_QUERIES {
                        diagnostics.push(script_component_diagnostic(
                            "SCRIPT_LOGIC_ASSET_QUERY_OVERFLOW",
                            &entity_id,
                            format!(
                                "script entity '{entity_id}' exceeded the logic asset query budget of {} per frame",
                                engine_script::MAX_PENDING_LOGIC_ASSET_QUERIES
                            ),
                        ));
                        continue;
                    }
                    let result = if let Err(error) = engine_script::validate_entity_id(&asset_id) {
                        engine_script::GameplayLogicAssetResult {
                            query_id,
                            asset_id,
                            json: None,
                            error: Some(error),
                        }
                    } else {
                        let id = AssetId::new(asset_id.clone());
                        match self
                            .asset_registry
                            .get::<engine_asset::cook::LogicAsset>(&id)
                        {
                            Some(asset) => match serde_json::to_string(asset.get()) {
                                Ok(json)
                                    if json.len()
                                        <= engine_script::MAX_SCRIPT_LOGIC_ASSET_JSON_BYTES =>
                                {
                                    engine_script::GameplayLogicAssetResult {
                                        query_id,
                                        asset_id,
                                        json: Some(json),
                                        error: None,
                                    }
                                }
                                Ok(_) => engine_script::GameplayLogicAssetResult {
                                    query_id,
                                    asset_id,
                                    json: None,
                                    error: Some(format!(
                                        "logic asset JSON exceeds the {}-byte script limit",
                                        engine_script::MAX_SCRIPT_LOGIC_ASSET_JSON_BYTES
                                    )),
                                },
                                Err(error) => engine_script::GameplayLogicAssetResult {
                                    query_id,
                                    asset_id,
                                    json: None,
                                    error: Some(format!(
                                        "logic asset could not be serialized: {error}"
                                    )),
                                },
                            },
                            None => engine_script::GameplayLogicAssetResult {
                                query_id,
                                asset_id,
                                json: None,
                                error: Some("logic asset is not loaded".into()),
                            },
                        }
                    };
                    self.script_logic_asset_results
                        .entry(entity_id)
                        .or_default()
                        .push(result);
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

    /// Execute the component queries drained from the latest script update
    /// and stage the results for the next frame snapshot.
    ///
    /// Queries run against the active World after this frame's commands
    /// apply, so answers observe same-frame component writes. Results are
    /// frame-local: they replace the previous staging map and are consumed by
    /// exactly one following script tick.
    #[cfg(feature = "subsystem-scripting-csharp")]
    fn execute_script_component_queries(&mut self) -> Vec<Diagnostic> {
        let pending = std::mem::take(&mut self.pending_component_queries);
        if pending.is_empty() {
            self.script_component_query_results.clear();
            return Vec::new();
        }
        let mut diagnostics = Vec::new();
        let mut results: std::collections::BTreeMap<
            String,
            Vec<engine_script::GameplayComponentQueryResult>,
        > = std::collections::BTreeMap::new();
        for engine_script::OwnedGameplayComponentQuery { entity_id, query } in pending {
            let outcome = self.world_slot.with_world(|world| {
                (
                    script_components::read_script_component(
                        world,
                        &query.entity_id,
                        &query.component_type,
                    ),
                    script_components::supported_script_component_types(world),
                )
            });
            use engine_script::GameplayComponentQueryResult as QueryResult;
            use script_components::ScriptComponentRead as Read;
            let result = match outcome {
                Some((Read::Snapshot(fields), _)) => QueryResult::Snapshot {
                    query_id: query.query_id,
                    entity_id: query.entity_id,
                    component_type: query.component_type,
                    fields,
                },
                Some((Read::Missing, _)) => QueryResult::Missing {
                    query_id: query.query_id,
                    entity_id: query.entity_id,
                    component_type: query.component_type,
                },
                Some((Read::Unsupported, supported)) => {
                    diagnostics.push(script_component_diagnostic(
                        "SCRIPT_COMPONENT_UNKNOWN",
                        &entity_id,
                        format!(
                            "script entity '{entity_id}' queried component '{}' on entity '{}', but that type is not script-accessible; {}",
                            query.component_type,
                            query.entity_id,
                            supported_script_component_description(&supported)
                        ),
                    ));
                    continue;
                }
                None => {
                    diagnostics.push(script_component_diagnostic(
                        "SCRIPT_WORLD_MISSING",
                        &entity_id,
                        format!(
                            "script entity '{entity_id}' component query on entity '{}' could not run because no World is active",
                            query.entity_id
                        ),
                    ));
                    continue;
                }
            };
            results.entry(entity_id).or_default().push(result);
        }
        self.script_component_query_results = results;
        diagnostics
    }

    /// Instantiate one cooked prefab for a `Scene.Spawn` command.
    ///
    /// The prefab is resolved from the runtime's cooked `prefab` extension
    /// assets (never from script-supplied paths), instantiated transactionally
    /// through the same component restoration path scenes use, and assigned
    /// deterministic persistent IDs. `engine.script` records ride along as
    /// scene-only metadata and are attached to the script engine with the same
    /// lifecycle as scene-authored scripts: `OnCreate` runs immediately, and
    /// its commands are applied recursively within this frame boundary.
    #[cfg(feature = "subsystem-scripting-csharp")]
    fn spawn_script_prefab(
        &mut self,
        requested_by: &str,
        prefab_id: &str,
        translation: Option<[f32; 3]>,
        diagnostics: &mut Vec<Diagnostic>,
        depth: usize,
    ) {
        let asset_id = AssetId::new(prefab_id);
        let Some(root_handle) = self.extension_asset::<engine_scene::Prefab>("prefab", &asset_id)
        else {
            let mut diagnostic = Diagnostic::new(
                "SCRIPT_PREFAB_UNKNOWN",
                DiagnosticSeverity::Error,
                "script",
                format!(
                    "script entity '{requested_by}' requested unknown prefab '{prefab_id}'; {}",
                    self.available_prefab_description()
                ),
            );
            diagnostic.entity = Some(requested_by.to_owned());
            diagnostics.push(diagnostic);
            return;
        };
        let root_prefab = root_handle.get().clone();

        // Nested prefab references resolve against the same cooked batch.
        let mut resolver = engine_scene::PrefabRegistry::new();
        let mut visiting = std::collections::BTreeSet::new();
        if let Err(missing) =
            self.collect_prefab_graph(&asset_id, &root_prefab, &mut resolver, &mut visiting)
        {
            let mut diagnostic = Diagnostic::new(
                "SCRIPT_PREFAB_GRAPH_INCOMPLETE",
                DiagnosticSeverity::Error,
                "script",
                format!(
                    "script entity '{requested_by}' requested prefab '{prefab_id}', but its nested prefab '{missing}' is not loaded; declare and cook every referenced prefab asset"
                ),
            );
            diagnostic.entity = Some(requested_by.to_owned());
            diagnostics.push(diagnostic);
            return;
        }

        let outcome = self.world_slot.with_world_mut(|world| {
            match engine_scene::instantiate_prefab(world, &root_prefab, Some(&resolver)) {
                Ok(result) => {
                    match assign_spawned_persistent_ids(world, prefab_id, &result) {
                        Ok(assigned) => {
                            if let Some(translation) = translation {
                                apply_spawn_translation(world, result.root_entity, translation);
                            }
                            Ok((result, assigned))
                        }
                        Err(reason) => {
                            // Roll the whole instance back so a failed spawn
                            // cannot leave anonymous or partially named
                            // entities behind.
                            for entity in result.all_entities.iter().rev() {
                                let _ = world.destroy_entity(*entity);
                            }
                            Err(reason)
                        }
                    }
                }
                Err(error) => Err(error.to_string()),
            }
        });

        let (result, assigned) = match outcome {
            Some(Ok(spawned)) => spawned,
            Some(Err(reason)) => {
                let mut diagnostic = Diagnostic::new(
                    "SCRIPT_PREFAB_SPAWN_FAILED",
                    DiagnosticSeverity::Error,
                    "script",
                    format!(
                        "script entity '{requested_by}' could not spawn prefab '{prefab_id}': {reason}"
                    ),
                );
                diagnostic.entity = Some(requested_by.to_owned());
                diagnostics.push(diagnostic);
                return;
            }
            None => {
                diagnostics.push(Diagnostic::new(
                    "SCRIPT_WORLD_MISSING",
                    DiagnosticSeverity::Error,
                    "script",
                    format!(
                        "script entity '{requested_by}' could not spawn prefab '{prefab_id}' because no World is active"
                    ),
                ));
                return;
            }
        };

        // Attach scene-only `engine.script` records with the same lifecycle
        // scene-authored scripts receive.
        let id_by_entity: std::collections::HashMap<engine_scene::Entity, String> =
            assigned.into_iter().collect();
        let mut attached_any = false;
        for (entity, component_type_id, record) in &result.scene_only_components {
            if component_type_id != script::SCRIPT_COMPONENT_TYPE {
                continue;
            }
            let Some(entity_id) = id_by_entity.get(entity) else {
                continue;
            };
            let Some(component) = script::extract_script_component_from_record(record) else {
                let mut diagnostic = Diagnostic::new(
                    "SCRIPT_SPAWN_ATTACH_FAILED",
                    DiagnosticSeverity::Error,
                    "script",
                    format!(
                        "spawned entity '{entity_id}' has an invalid engine.script record: 'assembly_id' and 'class_name' strings are required"
                    ),
                );
                diagnostic.entity = Some(entity_id.clone());
                diagnostics.push(diagnostic);
                continue;
            };
            match self.script_engine.attach_script(
                entity_id,
                &self.script_host_name.clone(),
                &component,
            ) {
                Ok(()) => attached_any = true,
                Err(error) => {
                    let mut diagnostic = Diagnostic::new(
                        "SCRIPT_SPAWN_ATTACH_FAILED",
                        DiagnosticSeverity::Error,
                        "script",
                        format!(
                            "failed to attach script '{}' to spawned entity '{entity_id}': {error}",
                            component.class_name
                        ),
                    );
                    diagnostic.entity = Some(entity_id.clone());
                    diagnostics.push(diagnostic);
                }
            }
        }
        if !attached_any {
            return;
        }

        // Run OnCreate for the newly attached instances and apply the
        // commands they enqueue (including further spawns) at this same frame
        // boundary, bounded by MAX_SCRIPT_SPAWN_DEPTH.
        let contexts = self.script_gameplay_contexts(
            &self.script_input_actions.clone(),
            &GameplayInputTransitions::default(),
            &std::collections::BTreeMap::new(),
            &[],
            &std::collections::BTreeMap::new(),
        );
        diagnostics.extend(self.script_engine.set_gameplay_contexts(&contexts));
        diagnostics.extend(self.script_engine.create_instances());
        let (commands, command_diagnostics) = self.script_engine.drain_gameplay_commands();
        diagnostics.extend(command_diagnostics);
        if commands.is_empty() {
            return;
        }
        if depth >= MAX_SCRIPT_SPAWN_DEPTH {
            diagnostics.push(Diagnostic::new(
                "SCRIPT_SPAWN_DEPTH_EXCEEDED",
                DiagnosticSeverity::Error,
                "script",
                format!(
                    "prefab spawn chains from OnCreate callbacks exceeded the depth budget of {MAX_SCRIPT_SPAWN_DEPTH}; remaining commands were deferred"
                ),
            ));
            return;
        }
        diagnostics.extend(self.apply_script_gameplay_commands_with_depth(commands, depth + 1));
    }

    /// Human-readable list of the loaded cooked prefab assets for actionable
    /// `Scene.Spawn` error messages.
    #[cfg(feature = "subsystem-scripting-csharp")]
    fn available_prefab_description(&self) -> String {
        let available = self
            .loaded_extension_asset_ids
            .get("prefab")
            .map(|ids| {
                ids.iter()
                    .map(|id| id.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        if available.is_empty() {
            "no prefab assets are loaded; declare .prefab.ron sources in the project's source manifest and cook the project".to_string()
        } else {
            format!("loaded prefabs: {available}")
        }
    }

    /// Register the reachable nested-prefab graph of `root` with the resolver.
    ///
    /// Cycles are left to the instantiation validator, which reports them as
    /// structured errors; this walk only proves that every referenced child is
    /// present in the cooked batch.
    #[cfg(feature = "subsystem-scripting-csharp")]
    fn collect_prefab_graph(
        &self,
        asset_id: &AssetId,
        prefab: &engine_scene::Prefab,
        resolver: &mut engine_scene::PrefabRegistry,
        visiting: &mut std::collections::BTreeSet<String>,
    ) -> Result<(), String> {
        if !visiting.insert(asset_id.id.clone()) {
            return Ok(());
        }
        for child_ref in &prefab.child_prefab_refs {
            let child_id = &child_ref.prefab_asset;
            let child = self
                .extension_asset::<engine_scene::Prefab>("prefab", child_id)
                .ok_or_else(|| child_id.id.clone())?;
            let child = child.get().clone();
            self.collect_prefab_graph(child_id, &child, resolver, visiting)?;
            resolver.register(child_id.id.clone(), child);
        }
        Ok(())
    }
}

/// Maximum synchronous `Scene.Spawn` → `OnCreate` recursion depth per frame
/// boundary. A script whose `OnCreate` spawns another scripted prefab cannot
/// recurse without bound inside one command drain.
#[cfg(feature = "subsystem-scripting-csharp")]
const MAX_SCRIPT_SPAWN_DEPTH: usize = 8;

/// Assign deterministic persistent IDs to a freshly instantiated prefab.
///
/// The root entity receives the first free id from `<prefabId>`,
/// `<prefabId>-2`, `<prefabId>-3`, …; every other spawned entity receives
/// `<rootId>.<prefab-local id>` with the same `-N` conflict suffix. The
/// result pairs each entity with its assigned id in spawn order, root first.
#[cfg(feature = "subsystem-scripting-csharp")]
fn assign_spawned_persistent_ids(
    world: &mut World,
    prefab_id: &str,
    result: &engine_scene::PrefabInstantiateResult,
) -> Result<Vec<(engine_scene::Entity, String)>, String> {
    let mut assigned: Vec<(engine_scene::Entity, String)> =
        Vec::with_capacity(result.all_entities.len());
    for entity in &result.all_entities {
        let base = if *entity == result.root_entity {
            prefab_id.to_string()
        } else {
            let local_id = world
                .get::<engine_scene::PrefabInstanceRef>(*entity)
                .map(|reference| reference.entity_persistent_id.clone())
                .unwrap_or_else(|| format!("entity-{}", entity.index()));
            let root_id = &assigned
                .first()
                .expect("the root entity is always assigned first")
                .1;
            format!("{root_id}.{local_id}")
        };
        let candidate = first_free_persistent_id(world, &base).ok_or_else(|| {
            format!("could not allocate a unique persistent entity id below '{base}'")
        })?;
        engine_script::validate_entity_id(&candidate).map_err(|reason| {
            format!("prefab '{prefab_id}' produced an unusable spawned entity id: {reason}")
        })?;
        world
            .assign_persistent_id(*entity, candidate.clone())
            .map_err(|error| error.to_string())?;
        assigned.push((*entity, candidate));
    }
    Ok(assigned)
}

/// First unused persistent id from `base`, `base-2`, `base-3`, … within the
/// 128-byte persistent-id budget.
#[cfg(feature = "subsystem-scripting-csharp")]
fn first_free_persistent_id(world: &World, base: &str) -> Option<String> {
    if world.entity_by_persistent_id(base).is_none() {
        return Some(base.to_string());
    }
    for suffix in 2_u64.. {
        let candidate = format!("{base}-{suffix}");
        if candidate.len() > 128 {
            return None;
        }
        if world.entity_by_persistent_id(&candidate).is_none() {
            return Some(candidate);
        }
    }
    unreachable!("the u64 suffix space cannot be exhausted in memory")
}

/// Apply the optional `Scene.Spawn` translation override to the spawned root.
/// A prefab root without a Transform gains one so the spawn position is never
/// silently dropped; rotation and scale from the prefab are preserved.
#[cfg(feature = "subsystem-scripting-csharp")]
fn apply_spawn_translation(world: &mut World, root: engine_scene::Entity, translation: [f32; 3]) {
    let translation = glam::Vec3::from_array(translation);
    if let Some(transform) = world.get_mut::<engine_scene::components::Transform>(root) {
        transform.translation = translation;
    } else {
        world.add_component(
            root,
            engine_scene::components::Transform {
                translation,
                ..Default::default()
            },
        );
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

/// Build an entity-scoped script diagnostic for the typed component bridge.
#[cfg(feature = "subsystem-scripting-csharp")]
fn script_component_diagnostic(code: &str, entity_id: &str, message: String) -> Diagnostic {
    let mut diagnostic = Diagnostic::new(code, DiagnosticSeverity::Error, "script", message);
    diagnostic.entity = Some(entity_id.to_owned());
    diagnostic
}

/// Human-readable list of the component type keys scripts may access, used
/// by `SCRIPT_COMPONENT_UNKNOWN` diagnostics.
#[cfg(feature = "subsystem-scripting-csharp")]
fn supported_script_component_description(supported: &[&'static str]) -> String {
    if supported.is_empty() {
        return "no component types are script-accessible in this build".to_string();
    }
    format!("supported component types: {}", supported.join(", "))
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
        SceneLoadDiagnostic::InvalidComponentFields {
            entity_id,
            component_type_id,
            ..
        } => ("SC0034", entity_id, component_type_id, None),
        SceneLoadDiagnostic::DuplicateSingletonComponent {
            entity_id,
            component_type_id,
            ..
        } => ("SC0035", entity_id, component_type_id, None),
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

fn missing_registered_render_asset(kind: &str, requested: &AssetId) -> Vec<Diagnostic> {
    let mut diagnostic = Diagnostic::new(
        "AS0002",
        DiagnosticSeverity::Error,
        "engine-core.assets",
        format!(
            "{kind} asset '{}' is referenced by the frame but is not registered in AssetRegistry",
            requested.id
        ),
    );
    diagnostic.asset = Some(requested.clone());
    vec![diagnostic]
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
            emissive: [0.0; 3],
            base_color_texture: None,
            normal_texture: None,
            metallic_roughness_texture: None,
            occlusion_texture: None,
            emissive_texture: None,
            advanced: engine_renderer::AdvancedMaterialParameters::default(),
            transparency: engine_renderer::Transparency::Opaque,
            double_sided: false,
            content_hash: engine_asset::compute_content_hash(&[b"mat-default-v1"]),
        },
    );

    let quad = engine_asset::mesh::MeshData {
        positions: vec![
            glam::Vec3::new(-0.5, -0.5, 0.0),
            glam::Vec3::new(0.5, -0.5, 0.0),
            glam::Vec3::new(0.5, 0.5, 0.0),
            glam::Vec3::new(-0.5, 0.5, 0.0),
        ],
        normals: vec![glam::Vec3::Z; 4],
        uvs: vec![
            glam::Vec2::new(0.0, 1.0),
            glam::Vec2::new(1.0, 1.0),
            glam::Vec2::new(1.0, 0.0),
            glam::Vec2::new(0.0, 0.0),
        ],
        indices: vec![0, 1, 2, 0, 2, 3],
        bounds: (
            glam::Vec3::new(-0.5, -0.5, 0.0),
            glam::Vec3::new(0.5, 0.5, 0.0),
        ),
        joints: Vec::new(),
        weights: Vec::new(),
    };
    let (vertex_bytes, index_bytes, index_count, _) =
        engine_asset::mesh::mesh_data_to_upload_bytes(&quad);
    let content_hash =
        engine_asset::compute_content_hash(&[vertex_bytes.as_slice(), index_bytes.as_slice()]);
    let mesh_id = AssetId::new(engine_vfx::BUILTIN_VFX_QUAD_MESH_ID);
    registry.insert_typed(
        mesh_id.clone(),
        MeshUpload {
            mesh_id,
            vertex_format: MeshVertexFormat::Pbr32,
            vertex_count: 4,
            vertex_bytes,
            index_format: engine_renderer::IndexFormat::U32,
            index_count,
            index_bytes,
            bounds: engine_renderer::AxisAlignedBox {
                min: quad.bounds.0.to_array(),
                max: quad.bounds.1.to_array(),
            },
            content_hash,
        },
    );

    let material_id = AssetId::new(engine_vfx::BUILTIN_VFX_MATERIAL_ID);
    registry.insert_typed(
        material_id.clone(),
        MaterialUpload {
            material_id,
            base_color: [1.0, 1.0, 1.0, 0.75],
            metallic: 0.0,
            roughness: 1.0,
            ambient_occlusion: 1.0,
            emissive: [0.0; 3],
            base_color_texture: None,
            normal_texture: None,
            metallic_roughness_texture: None,
            occlusion_texture: None,
            emissive_texture: None,
            advanced: engine_renderer::AdvancedMaterialParameters::default(),
            transparency: engine_renderer::Transparency::Blend,
            double_sided: true,
            content_hash: engine_asset::compute_content_hash(&[b"mat-vfx-default-v1"]),
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
/// during startup and pass the returned [`engine_renderer::BackendRenderer`] to
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
mod tests;
