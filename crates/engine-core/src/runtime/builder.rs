use crate::*;

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
