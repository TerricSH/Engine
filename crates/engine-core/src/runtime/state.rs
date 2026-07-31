use crate::*;

// ── Engine runtime ────────────────────────────────────────────────────────

/// Registry-backed GPU resources known to have reached the active backend.
///
/// This is reconciliation state, not a second owner: the typed
/// [`AssetRegistry`] entry remains authoritative for both upload and removal.
#[derive(Default)]
pub(crate) struct SyncedRenderResources {
    pub(crate) meshes: BTreeMap<AssetId, HashDigest>,
    pub(crate) textures: BTreeMap<AssetId, HashDigest>,
    pub(crate) materials: BTreeMap<AssetId, HashDigest>,
    pub(crate) environment_maps: BTreeMap<AssetId, HashDigest>,
    pub(crate) morph_target_sets: BTreeMap<AssetId, HashDigest>,
}

pub struct EngineRuntime {
    pub(crate) config: EngineConfig,
    pub(crate) renderer: Renderer,
    pub(crate) asset_registry: AssetRegistry,
    pub(crate) render_environment: engine_renderer::EnvironmentSettings,
    pub(crate) loaded_cooked_asset_ids: BTreeSet<AssetId>,
    pub(crate) loaded_extension_asset_ids: BTreeMap<String, BTreeSet<AssetId>>,
    /// Handle table for runtime-registered dynamic meshes (ENG-20). The
    /// meshes themselves live as typed `MeshUpload` assets in
    /// `asset_registry`; the table owns handle generations, name lookup,
    /// and memory accounting.
    pub(crate) runtime_mesh_table: runtime_mesh::RuntimeMeshTable,
    /// Resources created from the registry and awaiting lifetime
    /// reconciliation by the canonical render sync.
    pub(crate) synced_render_resources: SyncedRenderResources,
    /// Exact registry allocations owned by the tooling-preview entry point.
    /// Allocation identity prevents a stale preview owner from unloading a
    /// persistent replacement that reused the same [`AssetId`].
    pub(crate) temporary_preview_textures: BTreeMap<AssetId, AssetHandle<TextureUpload>>,
    /// Last global UI atlas revision copied into the typed asset registry.
    #[cfg(feature = "subsystem-ui")]
    pub(crate) generated_font_atlas_revision: u64,
    /// Lazily created background cooked-asset decoder; see
    /// [`EngineRuntime::enqueue_cooked_asset_stream`]. `None` until the first
    /// streamed enqueue so runtimes that never stream never spawn a thread.
    pub(crate) stream_loader: Option<AssetStreamLoader>,
    /// Per-drain commit budget applied when the loader is created.
    pub(crate) stream_budget: usize,
    pub(crate) scene: Option<Scene>,
    pub(crate) world_slot: WorldSlot,
    /// Per-pass CPU/GPU frame timing recorder and rolling statistics (ENG-04).
    pub(crate) frame_timing: engine_renderer::FrameTimingTracker,
    pub(crate) component_registry: Arc<ComponentRegistry>,
    pub(crate) asset_type_registry: AssetTypeRegistry,
    pub(crate) render_extension_registry: RenderExtensionRegistry,
    pub(crate) debug_draw_registry: DebugDrawRegistry,
    #[cfg(feature = "subsystem-animation")]
    pub(crate) animation_extensions: engine_animation::AnimationExtensionHandles,
    pub(crate) collector: DiagnosticsCollector,
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub(crate) scripting: super::scripting::ScriptRuntimeState,
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

    pub(crate) fn from_parts(
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
            #[cfg(feature = "subsystem-ui")]
            generated_font_atlas_revision: 0,
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
            scripting: super::scripting::ScriptRuntimeState::default(),
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
            self.scripting.pending_scene_request = None;
            self.scripting.pending_physics_queries.clear();
            self.scripting.pending_physics_mutations.clear();
            self.scripting.pending_damage_requests.clear();
            self.scripting.damage_events.clear();
            self.scripting.pending_ragdoll_requests.clear();
            self.scripting.ragdoll_events.clear();
            self.scripting.pending_component_queries.clear();
            self.scripting.component_query_results.clear();
            self.scripting.pending_save_requests.clear();
            self.scripting.save_events.clear();
            self.scripting.logic_asset_results.clear();
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
            self.scripting.pending_scene_request = None;
            self.scripting.pending_physics_queries.clear();
            self.scripting.pending_physics_mutations.clear();
            self.scripting.pending_damage_requests.clear();
            self.scripting.damage_events.clear();
            self.scripting.pending_ragdoll_requests.clear();
            self.scripting.ragdoll_events.clear();
            self.scripting.pending_component_queries.clear();
            self.scripting.component_query_results.clear();
            self.scripting.pending_save_requests.clear();
            self.scripting.save_events.clear();
            self.scripting.logic_asset_results.clear();
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
