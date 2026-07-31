use crate::{EngineConfig, EngineRuntime};
use engine_character::{CharacterController, CharacterMovement};
use engine_renderer::FrameStats;
use engine_scene::{RenderViewportContext, Scene};
use engine_serialize::{Diagnostic, DiagnosticSeverity};
use glam::Vec3;

#[cfg(any(feature = "subsystem-ui", feature = "runtime-audio-output"))]
use std::collections::BTreeMap;

#[cfg(feature = "runtime-audio-output")]
use std::collections::BTreeSet;

#[cfg(feature = "subsystem-gameplay")]
use engine_gameplay::{GameStateManager, InputActionMap};

#[cfg(feature = "subsystem-physics")]
use engine_physics::{PhysicsEvents, PhysicsWorld};

#[cfg(feature = "subsystem-animation")]
mod animation;
#[cfg(feature = "runtime-audio-output")]
mod audio;
mod character;
mod frame;
#[cfg(feature = "subsystem-navigation")]
mod navigation;
mod physics;
#[cfg(all(feature = "subsystem-navigation", feature = "subsystem-terrain"))]
mod planet_navigation;
#[cfg(feature = "subsystem-terrain")]
mod planet_scene_transition;
mod save;
#[cfg(feature = "subsystem-scripting-csharp")]
mod script_input;
#[cfg(feature = "subsystem-ui")]
mod ui;
mod world_origin;

#[cfg(feature = "runtime-audio-output")]
use audio::*;
#[cfg(feature = "subsystem-terrain")]
pub use planet_scene_transition::PlanetSceneTransitionTicket;
#[cfg(all(
    test,
    feature = "subsystem-scripting-csharp",
    feature = "subsystem-gameplay"
))]
use script_input::script_input_value_is_active;
#[cfg(feature = "subsystem-ui")]
use ui::embed_scene_ui_batches;
#[cfg(feature = "subsystem-ui")]
pub use ui::{RuntimeUiEvent, RuntimeUiValue};

/// Record of one executed world-origin shift (ENG-01 Phase 2).
///
/// Emitted by [`GameLoop::shift_world_origin`] and surfaced through
/// [`GameLoop::last_world_origin_shift`] and the headless run report. All
/// counts describe how much world-space state moved by `-delta` in the
/// atomic sweep.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldOriginShift {
    /// The applied origin delta. Logical positions are unchanged:
    /// `new_origin = old_origin + delta` and every stored world-space value
    /// moved by `-delta`.
    pub delta: [f64; 3],
    /// World origin after the shift.
    pub origin: [f64; 3],
    /// Root `Transform`s translated (children follow via the hierarchy).
    pub transforms: usize,
    /// Physics bodies teleported with simulation state preserved.
    pub physics_bodies: usize,
    /// Character controllers translated (components and the primary mirror).
    pub character_controllers: usize,
    /// Navigation agents whose target and path were translated.
    pub nav_agents: usize,
    /// Point gravity source centers translated.
    pub gravity_sources: usize,
    /// Live world-space CPU particles translated.
    pub vfx_particles: usize,
}

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

    #[cfg(feature = "subsystem-terrain")]
    pub terrain: crate::TerrainSystem,
    #[cfg(feature = "subsystem-terrain")]
    planet_scene_transitions: planet_scene_transition::PlanetSceneTransitionRuntime,

    /// Cumulative count and details of executed world-origin shifts.
    world_origin_shift_count: u64,
    last_world_origin_shift: Option<WorldOriginShift>,

    #[cfg(feature = "subsystem-physics")]
    pub physics: Option<PhysicsWorld>,

    /// Collision and trigger events produced by the most recent update.
    ///
    /// The loop drains the physics backend after every frame so events cannot
    /// accumulate indefinitely when a game does not explicitly consume them.
    #[cfg(feature = "subsystem-physics")]
    physics_events: PhysicsEvents,

    #[cfg(feature = "subsystem-gameplay")]
    pub state_manager: GameStateManager,

    #[cfg(feature = "subsystem-gameplay")]
    pub input_map: InputActionMap,

    #[cfg(all(feature = "subsystem-scripting-csharp", feature = "subsystem-gameplay"))]
    previous_script_input_actions:
        std::collections::BTreeMap<String, engine_script::GameplayInputValue>,

    #[cfg(feature = "subsystem-scripting-csharp")]
    script_pointer: engine_script::GameplayPointerSnapshot,
    #[cfg(feature = "subsystem-scripting-csharp")]
    script_save_directory: Option<std::path::PathBuf>,

    /// Physics query results computed after the previous update's script
    /// drain. They are delivered to scripts with exactly one frame snapshot
    /// and then discarded, mirroring the frame-local physics events.
    #[cfg(all(feature = "subsystem-scripting-csharp", feature = "subsystem-physics"))]
    script_physics_query_results:
        std::collections::BTreeMap<String, Vec<engine_script::GameplayPhysicsQueryResult>>,

    /// Kinematic character controller driven by `update_character`.
    pub character: Option<CharacterController>,

    /// Entity whose Transform is synced from the character controller's position.
    pub character_entity: Option<engine_scene::Entity>,

    #[cfg(feature = "subsystem-ui")]
    runtime_ui_input_states: BTreeMap<String, engine_ui::UiInputState>,
    #[cfg(feature = "subsystem-ui")]
    runtime_ui_pointer: [f32; 2],
    #[cfg(feature = "subsystem-ui")]
    runtime_ui_viewport: [f32; 2],
    #[cfg(feature = "subsystem-ui")]
    runtime_ui_captured_canvas: Option<String>,
    #[cfg(feature = "subsystem-ui")]
    runtime_ui_events: Vec<RuntimeUiEvent>,

    #[cfg(feature = "runtime-audio-output")]
    audio_output: RuntimeAudioOutput,
}

impl GameLoop {
    pub fn new(config: EngineConfig) -> Self {
        Self {
            runtime: EngineRuntime::new(config),
            #[cfg(feature = "subsystem-terrain")]
            terrain: crate::TerrainSystem::default(),
            #[cfg(feature = "subsystem-terrain")]
            planet_scene_transitions:
                planet_scene_transition::PlanetSceneTransitionRuntime::default(),
            world_origin_shift_count: 0,
            last_world_origin_shift: None,
            #[cfg(feature = "subsystem-physics")]
            physics: None,
            #[cfg(feature = "subsystem-physics")]
            physics_events: PhysicsEvents::default(),
            #[cfg(feature = "subsystem-gameplay")]
            state_manager: GameStateManager::with_default_transitions(
                engine_gameplay::GameState::Boot,
            ),
            #[cfg(feature = "subsystem-gameplay")]
            input_map: InputActionMap::new("player".to_string(), "gameplay".to_string()),
            #[cfg(all(feature = "subsystem-scripting-csharp", feature = "subsystem-gameplay"))]
            previous_script_input_actions: std::collections::BTreeMap::new(),
            #[cfg(feature = "subsystem-scripting-csharp")]
            script_pointer: engine_script::GameplayPointerSnapshot {
                focused: true,
                ..engine_script::GameplayPointerSnapshot::default()
            },
            #[cfg(feature = "subsystem-scripting-csharp")]
            script_save_directory: None,
            #[cfg(all(feature = "subsystem-scripting-csharp", feature = "subsystem-physics"))]
            script_physics_query_results: std::collections::BTreeMap::new(),
            character: None,
            character_entity: None,
            #[cfg(feature = "subsystem-ui")]
            runtime_ui_input_states: BTreeMap::new(),
            #[cfg(feature = "subsystem-ui")]
            runtime_ui_pointer: [0.0, 0.0],
            #[cfg(feature = "subsystem-ui")]
            runtime_ui_viewport: [0.0, 0.0],
            #[cfg(feature = "subsystem-ui")]
            runtime_ui_captured_canvas: None,
            #[cfg(feature = "subsystem-ui")]
            runtime_ui_events: Vec::new(),
            #[cfg(feature = "runtime-audio-output")]
            audio_output: RuntimeAudioOutput::default(),
        }
    }

    /// Load a scene and build the ECS World from it.
    ///
    /// After this call:
    /// - `runtime.with_world(...)` accesses the populated World
    /// - `runtime.render_frame()` uses World-based extraction (transforms work)
    /// - character and physics bindings describe the newly-loaded World
    ///
    /// Loading is transactional from the caller's perspective: if strict ECS
    /// restoration fails, the previous World and its gameplay bindings remain
    /// active.
    pub fn load_scene(&mut self, scene: Scene) -> Result<(), Vec<Diagnostic>> {
        #[cfg(all(feature = "subsystem-scripting-csharp", feature = "subsystem-gameplay"))]
        {
            let input_actions = self.resolved_script_input_actions();
            self.runtime.set_script_input_actions(input_actions);
        }
        #[cfg(all(feature = "subsystem-scripting-csharp", feature = "subsystem-physics"))]
        {
            self.script_physics_query_results.clear();
        }
        self.runtime.load_scene(scene)?;
        #[cfg(feature = "subsystem-terrain")]
        self.terrain.reset(&mut self.runtime);
        #[cfg(feature = "subsystem-ui")]
        self.reset_runtime_ui_input();
        #[cfg(feature = "runtime-audio-output")]
        self.reset_runtime_audio_scene();
        self.character = None;
        self.character_entity = None;
        self.bind_scene_character();
        self.init_physics();
        Ok(())
    }
}

#[cfg(test)]
#[path = "game_loop/tests.rs"]
mod tests;
