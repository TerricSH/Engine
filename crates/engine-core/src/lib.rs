#![forbid(unsafe_code)]

pub mod diagnostics;
pub use diagnostics::*;
pub mod cooked_assets;
pub use cooked_assets::*;

use engine_asset::{AssetHandle, AssetRegistry};
use engine_renderer::{
    AssetId, BackendRenderer, DebugDrawRegistry, FrameStats, MaterialUpload, MeshUpload,
    MeshVertexFormat, RenderExtensionRegistry, Renderer, TextureUpload,
};
use engine_scene::{
    extract_renderer_input_from_world, extract_renderer_input_from_world_with_viewport,
    validate_scene, AssetTypeRegistry, ComponentRegistry, RenderViewportContext, Scene,
    SceneLoadDiagnostic, World, WorldSlot,
};
use engine_serialize::{Diagnostic, DiagnosticSeverity};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

pub mod ffi_init;
pub mod game_loop;
#[cfg(feature = "runtime-subsystems")]
pub use game_loop::{RuntimeUiEvent, RuntimeUiValue};

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
        let mut asset_type_registry = AssetTypeRegistry::new();
        #[cfg(feature = "runtime-subsystems")]
        let mut render_extension_registry = RenderExtensionRegistry::new();
        #[cfg(not(feature = "runtime-subsystems"))]
        let render_extension_registry = RenderExtensionRegistry::new();
        let mut debug_draw_registry = DebugDrawRegistry::new();
        component_registry.register_core();
        engine_scene::register_prefab_asset_type(&mut asset_type_registry);
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
    /// Validated physics queries drained from scripts during the current
    /// update. The owning [`crate::game_loop::GameLoop`] executes them
    /// against its physics world at the frame boundary and delivers results
    /// in the next frame snapshot.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pending_physics_queries: Vec<engine_script::OwnedGameplayPhysicsQuery>,
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
            #[cfg(feature = "subsystem-scripting-csharp")]
            pending_physics_queries: Vec::new(),
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

    /// Install the backend used by the runtime's sole render pipeline.
    pub fn set_renderer_backend(&mut self, backend: Box<dyn BackendRenderer>) {
        self.renderer.set_backend(backend);
    }

    /// Resize the active backend without exposing unrestricted renderer access.
    pub fn resize_renderer(&mut self, width: u32, height: u32) -> Result<(), Vec<Diagnostic>> {
        self.renderer.resize(width, height)
    }

    /// Upload an editor-owned, short-lived preview texture.
    ///
    /// Persistent scene resources must be registered in [`AssetRegistry`].
    /// This deliberately narrow entry point is reserved for generated tooling
    /// previews that do not participate in scene asset ownership.
    pub fn upload_temporary_preview_texture(
        &mut self,
        upload: TextureUpload,
    ) -> Result<engine_renderer::UploadReceipt, Vec<Diagnostic>> {
        let id = upload.texture_id.clone();
        self.asset_registry.insert_typed(id, upload);
        Ok(engine_renderer::UploadReceipt {
            revision: 1,
            warnings: Vec::new(),
        })
    }

    /// Release an editor-owned texture previously uploaded through
    /// [`Self::upload_temporary_preview_texture`].
    pub fn remove_temporary_preview_texture(
        &mut self,
        texture_id: engine_renderer::AssetId,
    ) -> Result<(), Vec<Diagnostic>> {
        self.asset_registry.unload(&texture_id);
        self.renderer
            .remove_resource(engine_renderer::ResourceRemoval {
                kind: engine_renderer::ResourceKind::Texture,
                resource_id: texture_id,
            })
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
        self.render_frame_submission(frame_index, ui_batches, None)
    }

    /// Render a scene into a normalized sub-region of a concrete surface and
    /// composite caller-produced UI over the complete surface.
    ///
    /// Camera-authored viewports are composed inside `viewport`; extraction,
    /// asset synchronization, render-graph execution, and presentation remain
    /// on the exact same EngineRuntime -> Renderer -> backend path as a
    /// full-screen frame.
    pub fn render_frame_with_ui_in_viewport(
        &mut self,
        frame_index: u64,
        ui_batches: Vec<engine_renderer::UiBatch>,
        viewport: RenderViewportContext,
    ) -> Result<FrameStats, Vec<Diagnostic>> {
        self.render_frame_submission(frame_index, ui_batches, Some(viewport))
    }

    fn render_frame_submission(
        &mut self,
        frame_index: u64,
        ui_batches: Vec<engine_renderer::UiBatch>,
        viewport: Option<RenderViewportContext>,
    ) -> Result<FrameStats, Vec<Diagnostic>> {
        let mut input = if let Some(result) = self.world_slot.with_world(|world| match viewport {
            Some(viewport) => {
                extract_renderer_input_from_world_with_viewport(world, frame_index, viewport)
            }
            None => extract_renderer_input_from_world(world, frame_index),
        }) {
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
        self.refresh_generated_ui_assets();
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
            let handle = self
                .asset_registry
                .get::<MaterialUpload>(id)
                .ok_or_else(|| missing_registered_render_asset("material", id))?;
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
            let handle = self
                .asset_registry
                .get::<TextureUpload>(id)
                .ok_or_else(|| missing_registered_render_asset("texture", id))?;
            let upload = handle.get().clone();
            validate_registered_asset_id("texture", id, &upload.texture_id)?;
            textures.push(upload);
        }

        let mut meshes = Vec::new();
        for id in mesh_ids.values() {
            let handle = self
                .asset_registry
                .get::<MeshUpload>(id)
                .ok_or_else(|| missing_registered_render_asset("mesh", id))?;
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

    fn refresh_generated_ui_assets(&mut self) {
        #[cfg(feature = "runtime-subsystems")]
        if let Some(upload) = engine_ui::font_atlas_texture_upload() {
            let changed = self
                .asset_registry
                .get::<TextureUpload>(&upload.texture_id)
                .is_none_or(|current| current.get().content_hash != upload.content_hash);
            if changed {
                self.asset_registry
                    .insert_typed(upload.texture_id.clone(), upload);
            }
        }
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

        entity_ids
            .into_iter()
            .map(|entity_id| {
                let context = GameplayContext {
                    script_api: engine_script::GAMEPLAY_SCRIPT_API_SCHEMA.to_owned(),
                    transform: entities
                        .get(&entity_id)
                        .and_then(|snapshot| snapshot.transform.clone()),
                    entity_id: entity_id.clone(),
                    input_actions: input_actions.clone(),
                    input_transitions: input_transitions.clone(),
                    physics_events: physics_events.get(&entity_id).cloned().unwrap_or_default(),
                    physics_query_results: physics_query_results
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
                GameplayCommand::Ui { command } => {
                    if !script_command_owner_exists(&self.world_slot, &entity_id) {
                        diagnostics.push(script_owner_missing_diagnostic(
                            &entity_id,
                            "mutate runtime UI",
                        ));
                        continue;
                    }
                    #[cfg(feature = "runtime-subsystems")]
                    apply_script_ui_command(
                        &self.world_slot,
                        &entity_id,
                        command,
                        &mut diagnostics,
                    );
                    #[cfg(not(feature = "runtime-subsystems"))]
                    {
                        let _ = command;
                        let mut diagnostic = Diagnostic::new(
                            "SCRIPT_UI_UNAVAILABLE",
                            DiagnosticSeverity::Error,
                            "script",
                            format!(
                                "script entity '{entity_id}' requested runtime UI, but engine-core was built without runtime-subsystems"
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

#[cfg(all(feature = "subsystem-scripting-csharp", feature = "runtime-subsystems"))]
fn apply_script_ui_command(
    world_slot: &WorldSlot,
    requested_by: &str,
    command: engine_script::GameplayUiCommand,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if let Err(reason) = command.validate() {
        let mut diagnostic = Diagnostic::new(
            "SCRIPT_UI_COMMAND_INVALID",
            DiagnosticSeverity::Error,
            "script",
            format!("script entity '{requested_by}' produced an invalid UI command: {reason}"),
        );
        diagnostic.entity = Some(requested_by.to_owned());
        diagnostics.push(diagnostic);
        return;
    }

    let canvas_id = match &command {
        engine_script::GameplayUiCommand::CreateCanvas { canvas_id, .. }
        | engine_script::GameplayUiCommand::RemoveCanvas { canvas_id }
        | engine_script::GameplayUiCommand::ResizeCanvas { canvas_id, .. }
        | engine_script::GameplayUiCommand::SetCanvasScaleMode { canvas_id, .. }
        | engine_script::GameplayUiCommand::ClearCanvas { canvas_id }
        | engine_script::GameplayUiCommand::AddElement { canvas_id, .. }
        | engine_script::GameplayUiCommand::RemoveElement { canvas_id, .. }
        | engine_script::GameplayUiCommand::SetElementEnabled { canvas_id, .. }
        | engine_script::GameplayUiCommand::SetText { canvas_id, .. }
        | engine_script::GameplayUiCommand::SetToggleValue { canvas_id, .. }
        | engine_script::GameplayUiCommand::SetCheckboxValue { canvas_id, .. }
        | engine_script::GameplayUiCommand::SetSliderValue { canvas_id, .. } => canvas_id.clone(),
    };

    let applied = world_slot.with_world_mut(|world| -> Result<(), String> {
        match command {
            engine_script::GameplayUiCommand::CreateCanvas {
                canvas_id,
                width,
                height,
            } => {
                if world.entity_by_persistent_id(&canvas_id).is_some() {
                    return Err(format!(
                        "canvas '{canvas_id}' cannot be created because that persistent entity already exists"
                    ));
                }
                let entity = world
                    .create_persistent_entity(canvas_id.clone())
                    .map_err(|error| format!("create canvas entity '{canvas_id}': {error}"))?;
                world.add_component(entity, engine_ui::Canvas::new(width, height));
                Ok(())
            }
            engine_script::GameplayUiCommand::RemoveCanvas { canvas_id } => {
                let entity = world
                    .entity_by_persistent_id(&canvas_id)
                    .ok_or_else(|| format!("canvas '{canvas_id}' does not exist"))?;
                world
                    .remove_component::<engine_ui::Canvas>(entity)
                    .map(|_| ())
                    .ok_or_else(|| format!("entity '{canvas_id}' has no UI Canvas"))
            }
            engine_script::GameplayUiCommand::ResizeCanvas {
                canvas_id,
                width,
                height,
            } => {
                let canvas = script_ui_canvas_mut(world, &canvas_id)?;
                canvas.resize(width, height);
                Ok(())
            }
            engine_script::GameplayUiCommand::SetCanvasScaleMode {
                canvas_id,
                scale_mode,
            } => {
                script_ui_canvas_mut(world, &canvas_id)?.scale_mode = match scale_mode {
                    engine_script::GameplayUiScaleMode::Fixed => engine_ui::ScaleMode::Fixed,
                    engine_script::GameplayUiScaleMode::FitWidth => {
                        engine_ui::ScaleMode::FitWidth
                    }
                    engine_script::GameplayUiScaleMode::FitHeight => {
                        engine_ui::ScaleMode::FitHeight
                    }
                };
                Ok(())
            }
            engine_script::GameplayUiCommand::ClearCanvas { canvas_id } => {
                script_ui_canvas_mut(world, &canvas_id)?.clear();
                Ok(())
            }
            engine_script::GameplayUiCommand::AddElement {
                canvas_id,
                element_id,
                element,
            } => {
                let canvas = script_ui_canvas_mut(world, &canvas_id)?;
                canvas
                    .insert_element(
                        engine_ui::ElementId(element_id),
                        script_ui_runtime_element(element),
                    )
                    .map(|_| ())
                    .map_err(|error| format!("canvas '{canvas_id}': {error}"))
            }
            engine_script::GameplayUiCommand::RemoveElement {
                canvas_id,
                element_id,
            } => {
                let canvas = script_ui_canvas_mut(world, &canvas_id)?;
                if canvas.remove_element(engine_ui::ElementId(element_id)) {
                    Ok(())
                } else {
                    Err(format!(
                        "canvas '{canvas_id}' has no element with id {element_id}"
                    ))
                }
            }
            engine_script::GameplayUiCommand::SetElementEnabled {
                canvas_id,
                element_id,
                enabled,
            } => {
                let canvas = script_ui_canvas_mut(world, &canvas_id)?;
                let element = canvas
                    .get_element_mut(engine_ui::ElementId(element_id))
                    .ok_or_else(|| {
                        format!("canvas '{canvas_id}' has no element with id {element_id}")
                    })?;
                element.enabled = enabled;
                Ok(())
            }
            engine_script::GameplayUiCommand::SetText {
                canvas_id,
                element_id,
                text,
            } => {
                let canvas = script_ui_canvas_mut(world, &canvas_id)?;
                let element = canvas
                    .get_element_mut(engine_ui::ElementId(element_id))
                    .ok_or_else(|| {
                        format!("canvas '{canvas_id}' has no element with id {element_id}")
                    })?;
                match &mut element.kind {
                    engine_ui::UiElementKind::Text { content, .. } => {
                        *content = text;
                        Ok(())
                    }
                    _ => Err(format!(
                        "canvas '{canvas_id}' element {element_id} is not a Text element"
                    )),
                }
            }
            engine_script::GameplayUiCommand::SetToggleValue {
                canvas_id,
                element_id,
                is_on,
            } => {
                let canvas = script_ui_canvas_mut(world, &canvas_id)?;
                let element = canvas
                    .get_element_mut(engine_ui::ElementId(element_id))
                    .ok_or_else(|| {
                        format!("canvas '{canvas_id}' has no element with id {element_id}")
                    })?;
                match &mut element.kind {
                    engine_ui::UiElementKind::Toggle {
                        is_on: current, ..
                    } => {
                        *current = is_on;
                        Ok(())
                    }
                    _ => Err(format!(
                        "canvas '{canvas_id}' element {element_id} is not a Toggle element"
                    )),
                }
            }
            engine_script::GameplayUiCommand::SetCheckboxValue {
                canvas_id,
                element_id,
                checked,
            } => {
                let canvas = script_ui_canvas_mut(world, &canvas_id)?;
                let element = canvas
                    .get_element_mut(engine_ui::ElementId(element_id))
                    .ok_or_else(|| {
                        format!("canvas '{canvas_id}' has no element with id {element_id}")
                    })?;
                match &mut element.kind {
                    engine_ui::UiElementKind::Checkbox {
                        checked: current, ..
                    } => {
                        *current = checked;
                        Ok(())
                    }
                    _ => Err(format!(
                        "canvas '{canvas_id}' element {element_id} is not a Checkbox element"
                    )),
                }
            }
            engine_script::GameplayUiCommand::SetSliderValue {
                canvas_id,
                element_id,
                value,
            } => {
                let canvas = script_ui_canvas_mut(world, &canvas_id)?;
                let element = canvas
                    .get_element_mut(engine_ui::ElementId(element_id))
                    .ok_or_else(|| {
                        format!("canvas '{canvas_id}' has no element with id {element_id}")
                    })?;
                match &mut element.kind {
                    engine_ui::UiElementKind::Slider {
                        value: current,
                        min,
                        max,
                        ..
                    } if value >= *min && value <= *max => {
                        *current = value;
                        Ok(())
                    }
                    engine_ui::UiElementKind::Slider { min, max, .. } => Err(format!(
                        "canvas '{canvas_id}' slider {element_id} value {value} is outside [{min}, {max}]"
                    )),
                    _ => Err(format!(
                        "canvas '{canvas_id}' element {element_id} is not a Slider element"
                    )),
                }
            }
        }
    });

    match applied {
        Some(Ok(())) => {}
        Some(Err(reason)) => {
            let mut diagnostic = Diagnostic::new(
                "SCRIPT_UI_COMMAND_FAILED",
                DiagnosticSeverity::Error,
                "script",
                format!(
                    "script entity '{requested_by}' could not mutate canvas '{canvas_id}': {reason}"
                ),
            );
            diagnostic.entity = Some(canvas_id);
            diagnostics.push(diagnostic);
        }
        None => diagnostics.push(Diagnostic::new(
            "SCRIPT_WORLD_MISSING",
            DiagnosticSeverity::Error,
            "script",
            format!(
                "script entity '{requested_by}' could not mutate canvas '{canvas_id}' because no World is active"
            ),
        )),
    }
}

#[cfg(all(feature = "subsystem-scripting-csharp", feature = "runtime-subsystems"))]
fn script_ui_canvas_mut<'a>(
    world: &'a mut World,
    canvas_id: &str,
) -> Result<&'a mut engine_ui::Canvas, String> {
    let entity = world
        .entity_by_persistent_id(canvas_id)
        .ok_or_else(|| format!("canvas '{canvas_id}' does not exist"))?;
    world
        .get_mut::<engine_ui::Canvas>(entity)
        .ok_or_else(|| format!("entity '{canvas_id}' has no UI Canvas"))
}

#[cfg(all(feature = "subsystem-scripting-csharp", feature = "runtime-subsystems"))]
fn script_ui_runtime_element(element: engine_script::GameplayUiElement) -> engine_ui::UiElement {
    use engine_script::GameplayUiElement as WireElement;
    use engine_ui::UiElementKind;

    let color = |value: engine_script::GameplayUiColor| {
        engine_ui::Color::new(value.r, value.g, value.b, value.a)
    };
    let layout = |value: engine_script::GameplayUiLayout| {
        engine_ui::Layout::new(
            glam::Vec2::from_array(value.anchor_min),
            glam::Vec2::from_array(value.anchor_max),
            glam::Vec2::from_array(value.offset_min),
            glam::Vec2::from_array(value.offset_max),
        )
    };

    let (kind, layout, z_order) = match element {
        WireElement::Panel {
            layout: element_layout,
            color: element_color,
            z_order,
        } => (
            UiElementKind::Panel {
                color: color(element_color),
            },
            layout(element_layout),
            z_order,
        ),
        WireElement::Image {
            layout: element_layout,
            texture_id,
            color: element_color,
            z_order,
        } => (
            UiElementKind::Image {
                texture_id,
                color: color(element_color),
            },
            layout(element_layout),
            z_order,
        ),
        WireElement::Text {
            layout: element_layout,
            text,
            font_size,
            color: element_color,
            z_order,
        } => (
            UiElementKind::Text {
                content: text,
                font_size,
                color: color(element_color),
            },
            layout(element_layout),
            z_order,
        ),
        WireElement::Button {
            layout: element_layout,
            label,
            normal_color,
            hover_color,
            pressed_color,
            callback_id,
            z_order,
        } => (
            UiElementKind::Button {
                label,
                normal_color: color(normal_color),
                hover_color: color(hover_color),
                pressed_color: color(pressed_color),
                callback_id,
            },
            layout(element_layout),
            z_order,
        ),
        WireElement::Toggle {
            layout: element_layout,
            label,
            is_on,
            color_on,
            color_off,
            callback_id,
            z_order,
        } => (
            UiElementKind::Toggle {
                label,
                is_on,
                color_on: color(color_on),
                color_off: color(color_off),
                callback_id,
            },
            layout(element_layout),
            z_order,
        ),
        WireElement::Checkbox {
            layout: element_layout,
            label,
            checked,
            color: element_color,
            callback_id,
            z_order,
        } => (
            UiElementKind::Checkbox {
                label,
                checked,
                color: color(element_color),
                callback_id,
            },
            layout(element_layout),
            z_order,
        ),
        WireElement::Slider {
            layout: element_layout,
            label,
            value,
            min,
            max,
            callback_id,
            z_order,
        } => (
            UiElementKind::Slider {
                label,
                value,
                min,
                max,
                callback_id,
            },
            layout(element_layout),
            z_order,
        ),
        WireElement::ScrollView {
            layout: element_layout,
            content_width,
            content_height,
            color: element_color,
            z_order,
        } => (
            UiElementKind::ScrollView {
                scroll_x: 0.0,
                scroll_y: 0.0,
                content_width,
                content_height,
                color: color(element_color),
            },
            layout(element_layout),
            z_order,
        ),
    };

    engine_ui::UiElement::new(kind, layout).with_z_order(z_order)
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
        fn begin_frame(
            &mut self,
            _input: &engine_renderer::RenderFrameInput,
        ) -> Result<(), Vec<Diagnostic>> {
            Ok(())
        }

        fn apply_pass_barriers(
            &mut self,
            _input: &engine_renderer::RenderFrameInput,
            _pass: &engine_renderer::render_graph2::PassNode,
            _barriers: &[engine_renderer::render_graph2::CompiledBarrier],
        ) -> Result<(), Vec<Diagnostic>> {
            Ok(())
        }

        fn execute_pass(
            &mut self,
            input: &engine_renderer::RenderFrameInput,
            _pass: &engine_renderer::render_graph2::PassNode,
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

        fn end_frame(&mut self, _stats: &mut FrameStats) -> Result<(), Vec<Diagnostic>> {
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
            .load_scene(scene)
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
        runtime.set_renderer_backend(Box::new(RecordingBackend {
            uploads: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            rendered_ui_batch_counts: None,
        }));
        runtime
            .load_scene(engine_scene::sample_scene())
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
        runtime.set_renderer_backend(Box::new(RecordingBackend {
            uploads: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            rendered_ui_batch_counts: Some(std::sync::Arc::clone(&ui_counts)),
        }));
        runtime
            .load_scene(engine_scene::sample_scene())
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
    fn runtime_refreshes_generated_font_atlas_after_ui_batch_build() {
        if engine_ui::font_atlas_texture_upload().is_none() {
            return;
        }
        let _guard = serial_ffi_world_test();
        let uploads = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut runtime = EngineRuntime::new(EngineConfig::default());
        runtime.set_renderer_backend(Box::new(RecordingBackend {
            uploads: std::sync::Arc::clone(&uploads),
            rendered_ui_batch_counts: None,
        }));
        runtime
            .load_scene(engine_scene::sample_scene())
            .expect("sample scene should load");

        let mut canvas = engine_ui::Canvas::new(320.0, 180.0);
        canvas.add_element(engine_ui::UiElement::new(
            engine_ui::UiElementKind::Text {
                content: "Editor text".into(),
                font_size: 18.0,
                color: engine_ui::Color::WHITE,
            },
            engine_ui::Layout::FILL,
        ));
        canvas.layout_all();
        let batches = canvas.build_batches();
        assert!(batches.iter().any(|batch| {
            batch
                .texture
                .as_ref()
                .is_some_and(|texture| texture.id == engine_ui::FONT_ATLAS_ASSET)
        }));

        runtime
            .render_frame_with_ui(0, batches)
            .expect("generated font atlas should be registered before rendering");

        let texture_id = AssetId::new(engine_ui::FONT_ATLAS_ASSET);
        let atlas = runtime
            .asset_registry()
            .get::<TextureUpload>(&texture_id)
            .expect("font atlas must be owned by AssetRegistry");
        assert!(atlas.get().mip_levels[0]
            .bytes
            .chunks_exact(4)
            .any(|pixel| pixel[3] != 0));
        assert!(uploads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .any(|upload| upload == "texture:engine/font-atlas"));
    }

    #[cfg(feature = "runtime-subsystems")]
    #[test]
    fn game_loop_submits_retained_scene_canvas_batches_automatically() {
        let _guard = serial_ffi_world_test();
        let ui_counts = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut game_loop = game_loop::GameLoop::new(EngineConfig::default());
        game_loop
            .runtime
            .set_renderer_backend(Box::new(RecordingBackend {
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
        runtime.set_renderer_backend(Box::new(RecordingBackend {
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
            .load_scene(engine_scene::sample_scene())
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
        runtime.set_renderer_backend(Box::new(RecordingBackend {
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
            .load_scene(engine_scene::sample_scene())
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
            .load_scene(invalid_scene)
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
            .load_scene(duplicate)
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
            .load_scene(missing_parent)
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
    fn canonical_scene_load_replaces_and_activates_the_world() {
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
    fn engine_runtime_exposes_only_host_verified_script_classes() {
        use engine_script::MockHost;

        let mut runtime = EngineRuntime::new(EngineConfig::default());
        runtime.register_script_host(Box::new(
            MockHost::new().with_verified_classes("game", ["Game.Player"]),
        ));
        runtime
            .load_script_assembly("game", "mock", b"managed")
            .unwrap();

        let classes = runtime.verified_script_classes();
        assert_eq!(classes.len(), 1);
        assert_eq!(classes[0].assembly_id, "game");
        assert_eq!(classes[0].class_name, "Game.Player");
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
    fn script_spawn_test_transform_record(translation: [f32; 3]) -> engine_scene::ComponentRecord {
        engine_scene::ComponentRecord {
            schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: std::collections::BTreeMap::from([
                (
                    "translation".to_string(),
                    engine_serialize::Value::Vec3(translation),
                ),
                (
                    "rotation".to_string(),
                    engine_serialize::Value::Quat([
                        0.0,
                        0.0,
                        std::f32::consts::FRAC_1_SQRT_2,
                        std::f32::consts::FRAC_1_SQRT_2,
                    ]),
                ),
                (
                    "scale".to_string(),
                    engine_serialize::Value::Vec3([2.0, 2.0, 2.0]),
                ),
            ]),
        }
    }

    /// Two-entity prefab: root `root` with a rotated/scaled Transform and a
    /// child `bolt`, so tests can assert deterministic id assignment,
    /// hierarchy parenting, and translation overrides.
    #[cfg(feature = "subsystem-scripting-csharp")]
    fn script_spawn_test_prefab(prefab_id: &str) -> engine_scene::Prefab {
        let mut prefab = engine_scene::Prefab::new(AssetId::new(prefab_id));
        prefab.add_entity(engine_scene::EntityRecord {
            persistent_id: "root".to_string(),
            parent: None,
            name: Some("Root".to_string()),
            enabled: true,
            components: std::collections::BTreeMap::from([(
                "engine.transform".to_string(),
                script_spawn_test_transform_record([1.0, 2.0, 3.0]),
            )]),
        });
        prefab.add_entity(engine_scene::EntityRecord {
            persistent_id: "bolt".to_string(),
            parent: Some("root".to_string()),
            name: Some("Bolt".to_string()),
            enabled: true,
            components: std::collections::BTreeMap::from([(
                "engine.transform".to_string(),
                script_spawn_test_transform_record([0.0, 1.0, 0.0]),
            )]),
        });
        prefab
    }

    /// Install a cooked prefab into the runtime exactly like the cooked-batch
    /// loader does: typed payload in the asset registry plus the extension
    /// type-id registration that `extension_asset::<Prefab>("prefab", ..)`
    /// consults.
    #[cfg(feature = "subsystem-scripting-csharp")]
    fn register_script_prefab(
        runtime: &mut EngineRuntime,
        prefab_id: &str,
        prefab: engine_scene::Prefab,
    ) {
        let asset_id = AssetId::new(prefab_id);
        runtime
            .asset_registry_mut()
            .insert_typed(asset_id.clone(), prefab);
        runtime
            .loaded_extension_asset_ids
            .entry("prefab".to_string())
            .or_default()
            .insert(asset_id);
    }

    #[cfg(feature = "subsystem-scripting-csharp")]
    fn spawn_prefab_command(
        owner: &str,
        prefab_id: &str,
        translation: Option<[f32; 3]>,
    ) -> engine_script::OwnedGameplayCommand {
        engine_script::OwnedGameplayCommand {
            entity_id: owner.to_string(),
            command: GameplayCommand::SpawnPrefab {
                prefab_id: prefab_id.to_string(),
                translation,
            },
        }
    }

    #[cfg(feature = "subsystem-scripting-csharp")]
    #[test]
    fn script_spawn_prefab_assigns_deterministic_ids_and_enters_next_snapshot() {
        let _guard = serial_ffi_world_test();
        let mut runtime = EngineRuntime::new(EngineConfig::default());
        runtime.set_world(World::from_scene(&engine_scene::sample_scene()));
        register_script_prefab(
            &mut runtime,
            "prefab-x",
            script_spawn_test_prefab("prefab-x"),
        );

        let diagnostics = runtime.apply_script_gameplay_commands(vec![
            spawn_prefab_command("cube-01", "prefab-x", None),
            spawn_prefab_command("cube-01", "prefab-x", None),
        ]);

        assert!(
            diagnostics.is_empty(),
            "unexpected spawn diagnostics: {diagnostics:?}"
        );
        runtime
            .with_world(|world| {
                assert_eq!(world.alive_count(), 6);
                for id in ["prefab-x", "prefab-x.bolt", "prefab-x-2", "prefab-x-2.bolt"] {
                    assert!(
                        world.entity_by_persistent_id(id).is_some(),
                        "missing spawned entity '{id}'"
                    );
                }
                let root = world
                    .entity_by_persistent_id("prefab-x")
                    .expect("first spawn keeps the bare prefab id");
                let root_transform = world
                    .get::<engine_scene::components::Transform>(root)
                    .expect("spawned root must keep its Transform");
                assert_eq!(root_transform.translation.to_array(), [1.0, 2.0, 3.0]);
                let child = world
                    .entity_by_persistent_id("prefab-x.bolt")
                    .expect("child id derives from the prefab-local id");
                let child_transform = world
                    .get::<engine_scene::components::Transform>(child)
                    .expect("spawned child must keep its Transform");
                assert_eq!(child_transform.parent, Some(root));
            })
            .expect("runtime must keep an active World");
        let snapshots = runtime.script_gameplay_entity_snapshots();
        for id in ["prefab-x", "prefab-x.bolt", "prefab-x-2", "prefab-x-2.bolt"] {
            assert!(
                snapshots.contains_key(id),
                "next script context must include spawned entity '{id}'"
            );
        }
    }

    #[cfg(feature = "subsystem-scripting-csharp")]
    #[test]
    fn script_spawn_prefab_unknown_id_reports_actionable_diagnostic() {
        let _guard = serial_ffi_world_test();
        let mut runtime = EngineRuntime::new(EngineConfig::default());
        runtime.set_world(World::from_scene(&engine_scene::sample_scene()));
        register_script_prefab(
            &mut runtime,
            "prefab-x",
            script_spawn_test_prefab("prefab-x"),
        );

        let diagnostics = runtime.apply_script_gameplay_commands(vec![spawn_prefab_command(
            "cube-01",
            "prefab-missing",
            None,
        )]);

        assert_eq!(diagnostics.len(), 1);
        let diagnostic = &diagnostics[0];
        assert_eq!(diagnostic.code, "SCRIPT_PREFAB_UNKNOWN");
        assert_eq!(diagnostic.entity.as_deref(), Some("cube-01"));
        runtime
            .with_world(|world| {
                assert_eq!(world.alive_count(), 2);
            })
            .expect("runtime must keep an active World");
    }

    #[cfg(feature = "subsystem-scripting-csharp")]
    #[test]
    fn script_spawn_prefab_invalid_requests_never_partially_spawn() {
        let _guard = serial_ffi_world_test();
        let mut runtime = EngineRuntime::new(EngineConfig::default());
        runtime.set_world(World::from_scene(&engine_scene::sample_scene()));
        register_script_prefab(
            &mut runtime,
            "prefab-x",
            script_spawn_test_prefab("prefab-x"),
        );

        let diagnostics = runtime.apply_script_gameplay_commands(vec![
            spawn_prefab_command("cube-01", "../invalid", None),
            spawn_prefab_command("cube-01", "prefab-x", Some([f32::NAN, 0.0, 0.0])),
            spawn_prefab_command("missing-owner", "prefab-x", None),
        ]);

        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SCRIPT_PREFAB_ID_INVALID"));
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SCRIPT_PREFAB_TRANSFORM_INVALID"));
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SCRIPT_COMMAND_OWNER_MISSING"));
        runtime
            .with_world(|world| {
                assert_eq!(world.alive_count(), 2);
                assert!(world.entity_by_persistent_id("prefab-x").is_none());
            })
            .expect("runtime must keep an active World");
    }

    #[cfg(feature = "subsystem-scripting-csharp")]
    #[test]
    fn script_spawn_prefab_translation_override_preserves_prefab_rotation_and_scale() {
        let _guard = serial_ffi_world_test();
        let mut runtime = EngineRuntime::new(EngineConfig::default());
        runtime.set_world(World::from_scene(&engine_scene::sample_scene()));
        register_script_prefab(
            &mut runtime,
            "prefab-x",
            script_spawn_test_prefab("prefab-x"),
        );

        let diagnostics = runtime.apply_script_gameplay_commands(vec![spawn_prefab_command(
            "cube-01",
            "prefab-x",
            Some([7.0, 8.0, 9.0]),
        )]);

        assert!(
            diagnostics.is_empty(),
            "unexpected spawn diagnostics: {diagnostics:?}"
        );
        runtime
            .with_world(|world| {
                let root = world
                    .entity_by_persistent_id("prefab-x")
                    .expect("spawned root must exist");
                let transform = world
                    .get::<engine_scene::components::Transform>(root)
                    .expect("spawned root must keep its Transform");
                assert_eq!(transform.translation.to_array(), [7.0, 8.0, 9.0]);
                assert_eq!(
                    transform.rotation.to_array(),
                    [
                        0.0,
                        0.0,
                        std::f32::consts::FRAC_1_SQRT_2,
                        std::f32::consts::FRAC_1_SQRT_2
                    ],
                    "the override must not reset the prefab rotation"
                );
                assert_eq!(
                    transform.scale.to_array(),
                    [2.0, 2.0, 2.0],
                    "the override must not reset the prefab scale"
                );
                let child = world
                    .entity_by_persistent_id("prefab-x.bolt")
                    .expect("spawned child must exist");
                let child_transform = world
                    .get::<engine_scene::components::Transform>(child)
                    .expect("spawned child must keep its Transform");
                assert_eq!(
                    child_transform.translation.to_array(),
                    [0.0, 1.0, 0.0],
                    "the override only applies to the root"
                );
            })
            .expect("runtime must keep an active World");
    }

    #[cfg(feature = "subsystem-scripting-csharp")]
    #[test]
    fn script_spawn_prefab_attaches_scene_only_scripts_and_creates_instances() {
        let _guard = serial_ffi_world_test();
        use engine_script::MockHost;

        let mut runtime = EngineRuntime::new(EngineConfig::default());
        runtime.register_script_host(Box::new(MockHost::new()));
        runtime.set_script_host_name("mock");
        runtime
            .load_script_assembly("game", "mock", b"managed")
            .expect("mock assembly should load");
        runtime.set_world(World::from_scene(&engine_scene::sample_scene()));

        let mut prefab = engine_scene::Prefab::new(AssetId::new("prefab-scripted"));
        prefab.add_entity(engine_scene::EntityRecord {
            persistent_id: "root".to_string(),
            parent: None,
            name: Some("Root".to_string()),
            enabled: true,
            components: std::collections::BTreeMap::from([
                (
                    "engine.transform".to_string(),
                    script_spawn_test_transform_record([0.0; 3]),
                ),
                (
                    "engine.script".to_string(),
                    engine_scene::ComponentRecord {
                        schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
                        enabled: true,
                        fields: std::collections::BTreeMap::from([
                            (
                                "assembly_id".to_string(),
                                engine_serialize::Value::Str("game".to_string()),
                            ),
                            (
                                "class_name".to_string(),
                                engine_serialize::Value::Str("Game.Spawned".to_string()),
                            ),
                        ]),
                    },
                ),
            ]),
        });
        register_script_prefab(&mut runtime, "prefab-scripted", prefab);

        assert_eq!(runtime.script_engine.managers()[0].instance_count(), 0);
        let diagnostics = runtime.apply_script_gameplay_commands(vec![spawn_prefab_command(
            "cube-01",
            "prefab-scripted",
            None,
        )]);

        assert!(
            diagnostics.is_empty(),
            "unexpected spawn diagnostics: {diagnostics:?}"
        );
        assert_eq!(
            runtime.script_engine.managers()[0].instance_count(),
            1,
            "the scene-only engine.script record must attach to the spawned entity"
        );

        let diagnostics = runtime.apply_script_gameplay_commands(vec![spawn_prefab_command(
            "cube-01",
            "prefab-scripted",
            None,
        )]);
        assert!(
            diagnostics.is_empty(),
            "unexpected spawn diagnostics: {diagnostics:?}"
        );
        assert_eq!(runtime.script_engine.managers()[0].instance_count(), 2);
        runtime
            .with_world(|world| {
                assert!(world.entity_by_persistent_id("prefab-scripted").is_some());
                assert!(world.entity_by_persistent_id("prefab-scripted-2").is_some());
            })
            .expect("runtime must keep an active World");
    }

    #[cfg(all(feature = "subsystem-scripting-csharp", feature = "runtime-subsystems"))]
    #[test]
    fn managed_ui_commands_create_and_mutate_retained_canvas_components() {
        let _guard = serial_ffi_world_test();
        let mut runtime = EngineRuntime::new(EngineConfig::default());
        runtime.set_world(World::from_scene(&engine_scene::sample_scene()));
        let layout = engine_script::GameplayUiLayout {
            anchor_min: [0.0, 0.0],
            anchor_max: [0.0, 0.0],
            offset_min: [24.0, 24.0],
            offset_max: [344.0, 56.0],
        };
        let commands = vec![
            engine_script::OwnedGameplayCommand {
                entity_id: "cube-01".into(),
                command: GameplayCommand::Ui {
                    command: engine_script::GameplayUiCommand::CreateCanvas {
                        canvas_id: "hud".into(),
                        width: 1280.0,
                        height: 720.0,
                    },
                },
            },
            engine_script::OwnedGameplayCommand {
                entity_id: "cube-01".into(),
                command: GameplayCommand::Ui {
                    command: engine_script::GameplayUiCommand::SetCanvasScaleMode {
                        canvas_id: "hud".into(),
                        scale_mode: engine_script::GameplayUiScaleMode::FitWidth,
                    },
                },
            },
            engine_script::OwnedGameplayCommand {
                entity_id: "cube-01".into(),
                command: GameplayCommand::Ui {
                    command: engine_script::GameplayUiCommand::AddElement {
                        canvas_id: "hud".into(),
                        element_id: 1,
                        element: engine_script::GameplayUiElement::Panel {
                            layout,
                            color: engine_script::GameplayUiColor {
                                r: 20,
                                g: 20,
                                b: 20,
                                a: 210,
                            },
                            z_order: 10,
                        },
                    },
                },
            },
            engine_script::OwnedGameplayCommand {
                entity_id: "cube-01".into(),
                command: GameplayCommand::Ui {
                    command: engine_script::GameplayUiCommand::AddElement {
                        canvas_id: "hud".into(),
                        element_id: 2,
                        element: engine_script::GameplayUiElement::Text {
                            layout,
                            text: "Score: 0".into(),
                            font_size: 24.0,
                            color: engine_script::GameplayUiColor {
                                r: 255,
                                g: 255,
                                b: 255,
                                a: 255,
                            },
                            z_order: 11,
                        },
                    },
                },
            },
            engine_script::OwnedGameplayCommand {
                entity_id: "cube-01".into(),
                command: GameplayCommand::Ui {
                    command: engine_script::GameplayUiCommand::AddElement {
                        canvas_id: "hud".into(),
                        element_id: 3,
                        element: engine_script::GameplayUiElement::Toggle {
                            layout,
                            label: "Music".into(),
                            is_on: false,
                            color_on: engine_script::GameplayUiColor {
                                r: 0,
                                g: 200,
                                b: 80,
                                a: 255,
                            },
                            color_off: engine_script::GameplayUiColor {
                                r: 80,
                                g: 80,
                                b: 80,
                                a: 255,
                            },
                            callback_id: Some("music".into()),
                            z_order: 12,
                        },
                    },
                },
            },
            engine_script::OwnedGameplayCommand {
                entity_id: "cube-01".into(),
                command: GameplayCommand::Ui {
                    command: engine_script::GameplayUiCommand::AddElement {
                        canvas_id: "hud".into(),
                        element_id: 4,
                        element: engine_script::GameplayUiElement::Checkbox {
                            layout,
                            label: "Hints".into(),
                            checked: false,
                            color: engine_script::GameplayUiColor {
                                r: 200,
                                g: 200,
                                b: 200,
                                a: 255,
                            },
                            callback_id: Some("hints".into()),
                            z_order: 12,
                        },
                    },
                },
            },
            engine_script::OwnedGameplayCommand {
                entity_id: "cube-01".into(),
                command: GameplayCommand::Ui {
                    command: engine_script::GameplayUiCommand::AddElement {
                        canvas_id: "hud".into(),
                        element_id: 5,
                        element: engine_script::GameplayUiElement::Slider {
                            layout,
                            label: "Volume".into(),
                            value: 0.2,
                            min: 0.0,
                            max: 1.0,
                            callback_id: Some("volume".into()),
                            z_order: 12,
                        },
                    },
                },
            },
            engine_script::OwnedGameplayCommand {
                entity_id: "cube-01".into(),
                command: GameplayCommand::Ui {
                    command: engine_script::GameplayUiCommand::SetText {
                        canvas_id: "hud".into(),
                        element_id: 2,
                        text: "Score: 10".into(),
                    },
                },
            },
            engine_script::OwnedGameplayCommand {
                entity_id: "cube-01".into(),
                command: GameplayCommand::Ui {
                    command: engine_script::GameplayUiCommand::SetElementEnabled {
                        canvas_id: "hud".into(),
                        element_id: 1,
                        enabled: false,
                    },
                },
            },
            engine_script::OwnedGameplayCommand {
                entity_id: "cube-01".into(),
                command: GameplayCommand::Ui {
                    command: engine_script::GameplayUiCommand::SetToggleValue {
                        canvas_id: "hud".into(),
                        element_id: 3,
                        is_on: true,
                    },
                },
            },
            engine_script::OwnedGameplayCommand {
                entity_id: "cube-01".into(),
                command: GameplayCommand::Ui {
                    command: engine_script::GameplayUiCommand::SetCheckboxValue {
                        canvas_id: "hud".into(),
                        element_id: 4,
                        checked: true,
                    },
                },
            },
            engine_script::OwnedGameplayCommand {
                entity_id: "cube-01".into(),
                command: GameplayCommand::Ui {
                    command: engine_script::GameplayUiCommand::SetSliderValue {
                        canvas_id: "hud".into(),
                        element_id: 5,
                        value: 0.8,
                    },
                },
            },
        ];

        let diagnostics = runtime.apply_script_gameplay_commands(commands);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        runtime
            .with_world(|world| {
                let hud = world.entity_by_persistent_id("hud").expect("HUD entity");
                let canvas = world
                    .get::<engine_ui::Canvas>(hud)
                    .expect("Canvas component");
                assert_eq!((canvas.width, canvas.height), (1280.0, 720.0));
                assert_eq!(canvas.scale_mode, engine_ui::ScaleMode::FitWidth);
                assert!(!canvas.get_element(engine_ui::ElementId(1)).unwrap().enabled);
                assert!(matches!(
                    &canvas.get_element(engine_ui::ElementId(2)).unwrap().kind,
                    engine_ui::UiElementKind::Text { content, .. } if content == "Score: 10"
                ));
                assert!(matches!(
                    &canvas.get_element(engine_ui::ElementId(3)).unwrap().kind,
                    engine_ui::UiElementKind::Toggle { is_on: true, .. }
                ));
                assert!(matches!(
                    &canvas.get_element(engine_ui::ElementId(4)).unwrap().kind,
                    engine_ui::UiElementKind::Checkbox { checked: true, .. }
                ));
                assert!(matches!(
                    &canvas.get_element(engine_ui::ElementId(5)).unwrap().kind,
                    engine_ui::UiElementKind::Slider { value, .. } if (*value - 0.8).abs() < f32::EPSILON
                ));
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
            .load_scene(scene.clone())
            .expect("engine.script metadata should be allowed");

        // After load_scene, the script engine should have an instance
        assert_eq!(runtime.script_engine.host_count(), 1);
        let after = runtime.script_engine.managers()[0].instance_count();
        assert_eq!(after, 1, "script instance should have been created");

        runtime
            .load_scene(scene)
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
            .load_scene(scene)
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
