use crate::{EngineConfig, EngineRuntime};
use engine_renderer::FrameStats;
use engine_scene::Scene;
use engine_serialize::{Diagnostic, DiagnosticSeverity};

#[cfg(feature = "gameplay")]
use engine_gameplay::{GameStateManager, InputActionMap};

#[cfg(feature = "gameplay")]
use engine_physics::PhysicsWorld;

/// Standard game loop that wires together all engine subsystems.
///
/// ```text
/// input -> physics -> character -> ECS update -> extraction -> rendering
/// ```
///
/// The developer provides models (.gltf) and script/config code;
/// this struct handles the full tick pipeline through the engine's own
/// renderer (SceneRenderer / BackendRenderer).
pub struct GameLoop {
    pub runtime: EngineRuntime,

    #[cfg(feature = "gameplay")]
    pub physics: Option<PhysicsWorld>,

    #[cfg(feature = "gameplay")]
    pub state_manager: GameStateManager,

    #[cfg(feature = "gameplay")]
    pub input_map: InputActionMap,
}

impl GameLoop {
    pub fn new(config: EngineConfig) -> Self {
        Self {
            runtime: EngineRuntime::new(config),
            #[cfg(feature = "gameplay")]
            physics: None,
            #[cfg(feature = "gameplay")]
            state_manager: GameStateManager::with_default_transitions(
                engine_gameplay::GameState::Boot,
            ),
            #[cfg(feature = "gameplay")]
            input_map: InputActionMap::new("player".to_string(), "gameplay".to_string()),
        }
    }

    /// Load a scene and build the ECS World from it.
    ///
    /// After this call:
    /// - `runtime.world()` returns the populated World
    /// - `runtime.render_frame()` uses World-based extraction (transforms work)
    pub fn load_scene(&mut self, scene: Scene) {
        self.runtime.load_scene_to_world(scene);
    }

    /// Initialise the physics world using gravity from the scene settings
    /// (or a default of (0, -9.81, 0)) and sync any RigidBody/Collider
    /// components already in the ECS world.
    ///
    /// No-op when the `gameplay` feature is not enabled.
    pub fn init_physics(&mut self) {
        #[cfg(feature = "gameplay")]
        {
            let gravity = self
                .runtime
                .world()
                .map(|w| w.scene_settings().gravity)
                .flatten()
                .map(|g| glam::Vec3::new(g[0], g[1], g[2]))
                .unwrap_or(glam::Vec3::new(0.0, -9.81, 0.0));
            let mut pw = PhysicsWorld::new(gravity);
            if let Some(world) = self.runtime.world() {
                pw.sync_from_ecs(world);
            }
            self.physics = Some(pw);
        }
    }

    /// Advance the simulation by `dt` seconds.
    ///
    /// Handles physics stepping and ECS ↔ physics sync when the `gameplay`
    /// feature is enabled. Script ticking runs when the
    /// `subsystem-scripting-csharp` feature is active.
    ///
    /// Typical per-frame orchestration:
    /// 1. Resolve input events against `input_map`
    /// 2. Call `update(dt)` for physics + character + scripts
    /// 3. Call `render(frame_idx)` for extraction + draw
    pub fn update(&mut self, _dt: f32) {
        // Tick physics (ECS → physics → ECS sync) — gameplay feature
        #[cfg(feature = "gameplay")]
        if let Some(ref mut physics) = self.physics {
            if let Some(ref mut world) = self.runtime.world_mut() {
                physics.step(_dt, world);
            }
        }

        // Tick scripts (OnUpdate)
        #[cfg(feature = "subsystem-scripting-csharp")]
        self.runtime.tick_scripts(dt);
    }

    /// Produce a single rendered frame.
    pub fn render(&mut self, frame_index: u64) -> Result<FrameStats, Vec<Diagnostic>> {
        self.runtime.render_frame(frame_index)
    }

    /// Validate that the runtime has a loaded scene ready for rendering.
    pub fn validate_ready(&self) -> Result<(), Vec<Diagnostic>> {
        if self.runtime.world().is_none() && self.runtime.scene_ref().is_none() {
            return Err(vec![Diagnostic::new(
                "GL0001",
                DiagnosticSeverity::Error,
                "game_loop",
                "no scene loaded — call load_scene() first",
            )]);
        }
        Ok(())
    }
}
