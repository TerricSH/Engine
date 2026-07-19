use crate::{EngineConfig, EngineRuntime};
use engine_character::{CharacterController, CharacterMovement};
use engine_renderer::FrameStats;
use engine_scene::{RenderViewportContext, Scene};
use engine_serialize::{Diagnostic, DiagnosticSeverity};
use glam::Vec3;

#[cfg(feature = "runtime-subsystems")]
use std::collections::BTreeMap;

#[cfg(feature = "runtime-audio-output")]
use std::collections::BTreeSet;

#[cfg(feature = "gameplay")]
use engine_gameplay::{GameStateManager, InputActionMap};

#[cfg(feature = "gameplay")]
use engine_physics::{PhysicsEvents, PhysicsWorld};

#[cfg(feature = "runtime-subsystems")]
fn embed_scene_ui_batches(
    batches: &mut [engine_renderer::UiBatch],
    viewport: RenderViewportContext,
) {
    let surface_size = viewport.surface_size();
    let output = viewport.output_rect();
    let origin = [
        output.min[0] * surface_size[0] as f32,
        output.min[1] * surface_size[1] as f32,
    ];
    let extent = [
        output.width() * surface_size[0] as f32,
        output.height() * surface_size[1] as f32,
    ];
    for batch in batches {
        for vertex in &mut batch.vertices {
            vertex.position[0] += origin[0];
            vertex.position[1] += origin[1];
        }
        batch.clip_rect.min[0] =
            (batch.clip_rect.min[0] + origin[0]).clamp(origin[0], origin[0] + extent[0]);
        batch.clip_rect.min[1] =
            (batch.clip_rect.min[1] + origin[1]).clamp(origin[1], origin[1] + extent[1]);
        batch.clip_rect.max[0] =
            (batch.clip_rect.max[0] + origin[0]).clamp(origin[0], origin[0] + extent[0]);
        batch.clip_rect.max[1] =
            (batch.clip_rect.max[1] + origin[1]).clamp(origin[1], origin[1] + extent[1]);
    }
}

/// Platform-independent retained UI click produced by a scene Canvas.
///
/// This native event mirrors [`engine_script::GameplayUiEvent`] when the
/// scripting feature is enabled, while remaining available to non-scripted
/// runtime hosts through [`GameLoop::take_ui_events`].
#[cfg(feature = "runtime-subsystems")]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RuntimeUiValue {
    Bool(bool),
    Float(f32),
}

#[cfg(all(test, feature = "runtime-subsystems"))]
mod runtime_ui_tests {
    use super::*;

    #[test]
    fn retained_ui_geometry_is_embedded_and_clipped_to_the_scene_viewport() {
        let viewport = RenderViewportContext::new(
            1000,
            800,
            engine_renderer::Rect {
                min: [0.2, 0.125],
                max: [0.7, 0.75],
            },
        )
        .unwrap();
        let mut batches = vec![engine_renderer::UiBatch {
            canvas_id: "hud".into(),
            z_order: 0,
            clip_rect: engine_renderer::Rect {
                min: [-10.0, -20.0],
                max: [600.0, 700.0],
            },
            texture: None,
            vertices: vec![engine_renderer::UiVertex {
                position: [25.0, 40.0],
                uv: [0.0, 0.0],
                color: [255; 4],
            }],
            indices: Vec::new(),
            material: engine_renderer::AssetId::new("ui/default"),
        }];

        embed_scene_ui_batches(&mut batches, viewport);

        assert_eq!(batches[0].vertices[0].position, [225.0, 140.0]);
        assert_eq!(batches[0].clip_rect.min, [200.0, 100.0]);
        assert_eq!(batches[0].clip_rect.max, [700.0, 600.0]);
    }

    fn game_loop_with_canvas(mut canvas: engine_ui::Canvas) -> GameLoop {
        canvas.layout_all();
        let mut game_loop = GameLoop::new(EngineConfig::default());
        game_loop.load_scene(engine_scene::sample_scene()).unwrap();
        game_loop
            .runtime
            .with_world_mut(|world| {
                let entity = world.entity_by_persistent_id("camera-main").unwrap();
                world.add_component(entity, canvas);
            })
            .unwrap();
        game_loop
    }

    #[test]
    fn scaled_toggle_click_persists_value_and_reports_it_to_the_host() {
        let mut canvas = engine_ui::Canvas::new(100.0, 20.0);
        canvas.scale_mode = engine_ui::ScaleMode::FitWidth;
        let toggle_id = canvas.add_element(engine_ui::UiElement::new(
            engine_ui::UiElementKind::Toggle {
                label: "Music".into(),
                is_on: false,
                color_on: engine_ui::Color::new(0, 200, 80, 255),
                color_off: engine_ui::Color::new(80, 80, 80, 255),
                callback_id: Some("music".into()),
            },
            engine_ui::Layout::FILL,
        ));
        let mut game_loop = game_loop_with_canvas(canvas);
        game_loop.set_ui_viewport_size(200, 100);

        // Screen coordinates are converted back to the 100x20 logical Canvas.
        game_loop.ui_pointer_move(100.0, 20.0);
        game_loop.ui_pointer_left_press();
        game_loop.ui_pointer_left_release();

        assert_eq!(
            game_loop.take_ui_events(),
            vec![RuntimeUiEvent {
                canvas_id: "camera-main".into(),
                element_id: toggle_id.0,
                callback_id: Some("music".into()),
                value: Some(RuntimeUiValue::Bool(true)),
            }]
        );
        assert_eq!(
            game_loop.runtime.with_world(|world| {
                let entity = world.entity_by_persistent_id("camera-main").unwrap();
                let canvas = world.get::<engine_ui::Canvas>(entity).unwrap();
                match &canvas.get_element(toggle_id).unwrap().kind {
                    engine_ui::UiElementKind::Toggle { is_on, .. } => *is_on,
                    _ => false,
                }
            }),
            Some(true)
        );
    }

    #[test]
    fn slider_drag_reports_continuous_float_values() {
        let mut canvas = engine_ui::Canvas::new(100.0, 20.0);
        let slider_id = canvas.add_element(engine_ui::UiElement::new(
            engine_ui::UiElementKind::Slider {
                label: "Volume".into(),
                value: 0.0,
                min: 0.0,
                max: 1.0,
                callback_id: Some("volume".into()),
            },
            engine_ui::Layout::FILL,
        ));
        let mut game_loop = game_loop_with_canvas(canvas);

        game_loop.ui_pointer_move(10.0, 10.0);
        game_loop.ui_pointer_left_press();
        game_loop.ui_pointer_move(75.0, 10.0);

        assert_eq!(
            game_loop.take_ui_events(),
            vec![RuntimeUiEvent {
                canvas_id: "camera-main".into(),
                element_id: slider_id.0,
                callback_id: Some("volume".into()),
                value: Some(RuntimeUiValue::Float(0.75)),
            }]
        );
        assert_eq!(
            game_loop.runtime.with_world(|world| {
                let entity = world.entity_by_persistent_id("camera-main").unwrap();
                let canvas = world.get::<engine_ui::Canvas>(entity).unwrap();
                match &canvas.get_element(slider_id).unwrap().kind {
                    engine_ui::UiElementKind::Slider { value, .. } => *value,
                    _ => -1.0,
                }
            }),
            Some(0.75)
        );
    }

    #[test]
    fn runtime_batches_scale_to_viewport_and_reference_the_font_atlas() {
        if engine_ui::font_atlas_texture_upload().is_none() {
            return;
        }
        let mut canvas = engine_ui::Canvas::new(320.0, 180.0);
        canvas.scale_mode = engine_ui::ScaleMode::FitWidth;
        canvas.add_element(engine_ui::UiElement::new(
            engine_ui::UiElementKind::Text {
                content: "HUD".into(),
                font_size: 20.0,
                color: engine_ui::Color::WHITE,
            },
            engine_ui::Layout::new(
                glam::Vec2::ZERO,
                glam::Vec2::ZERO,
                glam::Vec2::new(10.0, 10.0),
                glam::Vec2::new(100.0, 40.0),
            ),
        ));
        let mut game_loop = game_loop_with_canvas(canvas);
        game_loop.set_ui_viewport_size(640, 480);

        let batches = game_loop.runtime_ui_batches();
        assert_eq!(batches[0].clip_rect.max, [640.0, 360.0]);
        assert_eq!(
            batches[0].texture,
            Some(engine_serialize::AssetId::new(engine_ui::FONT_ATLAS_ASSET))
        );
    }
}

#[cfg(feature = "runtime-subsystems")]
#[derive(Clone, Debug, PartialEq)]
pub struct RuntimeUiEvent {
    pub canvas_id: String,
    pub element_id: u32,
    pub callback_id: Option<String>,
    pub value: Option<RuntimeUiValue>,
}

#[cfg(feature = "runtime-audio-output")]
#[derive(Clone, Debug, PartialEq)]
struct RuntimeAudioEmitterSnapshot {
    position: Vec3,
    max_distance: f32,
    rolloff_factor: f32,
}

#[cfg(feature = "runtime-audio-output")]
#[derive(Clone, Debug, PartialEq)]
struct RuntimeAudioSourceSnapshot {
    entity_id: String,
    clip_asset: Option<String>,
    clip_loaded: bool,
    playing: bool,
    volume: f32,
    looping: bool,
    emitter: Option<RuntimeAudioEmitterSnapshot>,
}

#[cfg(feature = "runtime-audio-output")]
impl RuntimeAudioSourceSnapshot {
    fn playable_clip(&self) -> Option<&str> {
        self.playing
            .then_some(())
            .filter(|_| self.clip_loaded)
            .and(self.clip_asset.as_deref())
            .filter(|clip_asset| !clip_asset.is_empty())
    }
}

#[cfg(feature = "runtime-audio-output")]
#[derive(Clone, Debug, PartialEq)]
struct RuntimeAudioListenerSnapshot {
    position: Vec3,
    forward: Vec3,
    up: Vec3,
}

#[cfg(feature = "runtime-audio-output")]
#[derive(Default)]
struct RuntimeAudioFrame {
    sources: Vec<RuntimeAudioSourceSnapshot>,
    listener: Option<RuntimeAudioListenerSnapshot>,
}

#[cfg(feature = "runtime-audio-output")]
#[derive(Clone, Debug, PartialEq)]
enum RuntimeAudioSourceAction {
    Start(RuntimeAudioSourceSnapshot),
    Update(RuntimeAudioSourceSnapshot),
    Stop { entity_id: String },
}

#[cfg(feature = "runtime-audio-output")]
#[derive(Clone, Debug)]
struct RuntimeAudioVoiceState {
    clip_asset: String,
    looping: bool,
    completed_while_requested: bool,
}

/// Device-independent scene/audio state reconciliation.
///
/// Keeping this state machine separate from cpal makes scene playback
/// semantics testable on build agents and machines without an output device.
#[cfg(feature = "runtime-audio-output")]
#[derive(Default)]
struct SceneAudioReconciler {
    voices: BTreeMap<String, RuntimeAudioVoiceState>,
}

#[cfg(feature = "runtime-audio-output")]
impl SceneAudioReconciler {
    fn reconcile(
        &mut self,
        sources: &[RuntimeAudioSourceSnapshot],
        finished_entities: &BTreeSet<String>,
    ) -> Vec<RuntimeAudioSourceAction> {
        let desired = sources
            .iter()
            .map(|source| (source.entity_id.clone(), source))
            .collect::<BTreeMap<_, _>>();
        let mut actions = Vec::new();

        let removed = self
            .voices
            .keys()
            .filter(|entity_id| !desired.contains_key(*entity_id))
            .cloned()
            .collect::<Vec<_>>();
        for entity_id in removed {
            if self
                .voices
                .remove(&entity_id)
                .is_some_and(|voice| !voice.completed_while_requested)
            {
                actions.push(RuntimeAudioSourceAction::Stop { entity_id });
            }
        }

        for (entity_id, source) in desired {
            let Some(clip_asset) = source.playable_clip() else {
                if self
                    .voices
                    .remove(&entity_id)
                    .is_some_and(|voice| !voice.completed_while_requested)
                {
                    actions.push(RuntimeAudioSourceAction::Stop { entity_id });
                }
                continue;
            };

            let Some(voice) = self.voices.get_mut(&entity_id) else {
                self.voices.insert(
                    entity_id,
                    RuntimeAudioVoiceState {
                        clip_asset: clip_asset.to_owned(),
                        looping: source.looping,
                        completed_while_requested: false,
                    },
                );
                actions.push(RuntimeAudioSourceAction::Start(source.clone()));
                continue;
            };

            if voice.clip_asset != clip_asset {
                if !voice.completed_while_requested {
                    actions.push(RuntimeAudioSourceAction::Stop {
                        entity_id: entity_id.clone(),
                    });
                }
                *voice = RuntimeAudioVoiceState {
                    clip_asset: clip_asset.to_owned(),
                    looping: source.looping,
                    completed_while_requested: false,
                };
                actions.push(RuntimeAudioSourceAction::Start(source.clone()));
                continue;
            }

            if finished_entities.contains(&entity_id) {
                actions.push(RuntimeAudioSourceAction::Stop {
                    entity_id: entity_id.clone(),
                });
                if source.looping {
                    voice.completed_while_requested = false;
                    actions.push(RuntimeAudioSourceAction::Start(source.clone()));
                } else {
                    voice.completed_while_requested = true;
                }
                voice.looping = source.looping;
                continue;
            }

            if voice.completed_while_requested {
                // A one-shot stays completed while `playing` remains true.
                // Enabling looping is an explicit state change and rearms it.
                if source.looping && !voice.looping {
                    voice.completed_while_requested = false;
                    actions.push(RuntimeAudioSourceAction::Start(source.clone()));
                }
                voice.looping = source.looping;
                continue;
            }

            voice.looping = source.looping;
            actions.push(RuntimeAudioSourceAction::Update(source.clone()));
        }

        actions
    }

    fn reset(&mut self) -> Vec<RuntimeAudioSourceAction> {
        std::mem::take(&mut self.voices)
            .into_iter()
            .filter_map(|(entity_id, voice)| {
                (!voice.completed_while_requested)
                    .then_some(RuntimeAudioSourceAction::Stop { entity_id })
            })
            .collect()
    }
}

#[cfg(feature = "runtime-audio-output")]
#[derive(Default)]
struct RuntimeAudioOutput {
    reconciler: SceneAudioReconciler,
    engine: Option<engine_audio::AudioEngine>,
    handles: BTreeMap<String, engine_audio::AudioHandle>,
    initialization_failed: bool,
}

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

    /// Cumulative count and details of executed world-origin shifts.
    world_origin_shift_count: u64,
    last_world_origin_shift: Option<WorldOriginShift>,

    #[cfg(feature = "gameplay")]
    pub physics: Option<PhysicsWorld>,

    /// Collision and trigger events produced by the most recent update.
    ///
    /// The loop drains the physics backend after every frame so events cannot
    /// accumulate indefinitely when a game does not explicitly consume them.
    #[cfg(feature = "gameplay")]
    physics_events: PhysicsEvents,

    #[cfg(feature = "gameplay")]
    pub state_manager: GameStateManager,

    #[cfg(feature = "gameplay")]
    pub input_map: InputActionMap,

    #[cfg(all(feature = "subsystem-scripting-csharp", feature = "gameplay"))]
    previous_script_input_actions:
        std::collections::BTreeMap<String, engine_script::GameplayInputValue>,

    /// Physics query results computed after the previous update's script
    /// drain. They are delivered to scripts with exactly one frame snapshot
    /// and then discarded, mirroring the frame-local physics events.
    #[cfg(all(feature = "subsystem-scripting-csharp", feature = "gameplay"))]
    script_physics_query_results:
        std::collections::BTreeMap<String, Vec<engine_script::GameplayPhysicsQueryResult>>,

    /// Kinematic character controller driven by `update_character`.
    pub character: Option<CharacterController>,

    /// Entity whose Transform is synced from the character controller's position.
    pub character_entity: Option<engine_scene::Entity>,

    #[cfg(feature = "runtime-subsystems")]
    runtime_ui_input_states: BTreeMap<String, engine_ui::UiInputState>,
    #[cfg(feature = "runtime-subsystems")]
    runtime_ui_pointer: [f32; 2],
    #[cfg(feature = "runtime-subsystems")]
    runtime_ui_viewport: [f32; 2],
    #[cfg(feature = "runtime-subsystems")]
    runtime_ui_captured_canvas: Option<String>,
    #[cfg(feature = "runtime-subsystems")]
    runtime_ui_events: Vec<RuntimeUiEvent>,

    #[cfg(feature = "runtime-audio-output")]
    audio_output: RuntimeAudioOutput,
}

impl GameLoop {
    pub fn new(config: EngineConfig) -> Self {
        Self {
            runtime: EngineRuntime::new(config),
            world_origin_shift_count: 0,
            last_world_origin_shift: None,
            #[cfg(feature = "gameplay")]
            physics: None,
            #[cfg(feature = "gameplay")]
            physics_events: PhysicsEvents::default(),
            #[cfg(feature = "gameplay")]
            state_manager: GameStateManager::with_default_transitions(
                engine_gameplay::GameState::Boot,
            ),
            #[cfg(feature = "gameplay")]
            input_map: InputActionMap::new("player".to_string(), "gameplay".to_string()),
            #[cfg(all(feature = "subsystem-scripting-csharp", feature = "gameplay"))]
            previous_script_input_actions: std::collections::BTreeMap::new(),
            #[cfg(all(feature = "subsystem-scripting-csharp", feature = "gameplay"))]
            script_physics_query_results: std::collections::BTreeMap::new(),
            character: None,
            character_entity: None,
            #[cfg(feature = "runtime-subsystems")]
            runtime_ui_input_states: BTreeMap::new(),
            #[cfg(feature = "runtime-subsystems")]
            runtime_ui_pointer: [0.0, 0.0],
            #[cfg(feature = "runtime-subsystems")]
            runtime_ui_viewport: [0.0, 0.0],
            #[cfg(feature = "runtime-subsystems")]
            runtime_ui_captured_canvas: None,
            #[cfg(feature = "runtime-subsystems")]
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
        #[cfg(all(feature = "subsystem-scripting-csharp", feature = "gameplay"))]
        {
            let input_actions = self.resolved_script_input_actions();
            self.runtime.set_script_input_actions(input_actions);
            self.script_physics_query_results.clear();
        }
        self.runtime.load_scene(scene)?;
        #[cfg(feature = "runtime-subsystems")]
        self.reset_runtime_ui_input();
        #[cfg(feature = "runtime-audio-output")]
        self.reset_runtime_audio_scene();
        self.character = None;
        self.character_entity = None;
        self.bind_scene_character();
        self.init_physics();
        Ok(())
    }

    fn bind_scene_character(&mut self) {
        let bound = self
            .runtime
            .with_world_mut(|world| {
                let entities = world
                    .query::<CharacterController>()
                    .map(|(entity, _)| entity)
                    .collect::<Vec<_>>();
                let mut bound = None;
                for entity in entities {
                    let transform_position = world
                        .get::<engine_scene::components::Transform>(entity)
                        .map(|transform| transform.translation);
                    let Some(controller) = world.get_mut::<CharacterController>(entity) else {
                        continue;
                    };
                    if let Some(position) = transform_position {
                        controller.set_position(position);
                    }
                    if bound.is_none() {
                        bound = Some((entity, controller.clone()));
                    }
                }
                bound
            })
            .flatten();
        if let Some((entity, controller)) = bound {
            self.character = Some(controller);
            self.character_entity = Some(entity);
        }
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
                .with_world(|world| world.scene_settings().gravity)
                .flatten()
                .map(|g| glam::Vec3::new(g[0], g[1], g[2]))
                .unwrap_or(glam::Vec3::new(0.0, -9.81, 0.0));
            let mut pw = PhysicsWorld::new(gravity);
            self.runtime.with_world(|world| pw.sync_from_ecs(world));
            self.physics = Some(pw);
            self.physics_events.clear();
        }
    }

    /// Events produced by the most recent physics update.
    ///
    /// This snapshot is replaced on the next call to [`Self::update`]. Use
    /// [`Self::take_physics_events`] when the caller wants to take ownership.
    #[cfg(feature = "gameplay")]
    pub fn physics_events(&self) -> &PhysicsEvents {
        &self.physics_events
    }

    /// Re-synchronise the physics world after direct ECS world mutations
    /// that bypass [`load_scene`](Self::load_scene) — world-partition cell
    /// streaming merges/unloads commit at the frame boundary and call this.
    ///
    /// With the `gameplay` feature this runs the incremental
    /// `PhysicsWorld::sync_from_ecs`: bodies and colliders are created for
    /// newly merged entities and removed for unloaded ones, while every
    /// untouched entity keeps its exact simulation state. Scene-level
    /// physics settings (gravity) cannot change through cell merges because
    /// merges preserve world scene metadata, so no full rebuild is needed.
    /// Without the `gameplay` feature this is a no-op.
    pub fn resync_physics_from_world(&mut self) {
        #[cfg(feature = "gameplay")]
        if let Some(ref mut physics) = self.physics {
            self.runtime
                .with_world(|world| physics.sync_from_ecs(world));
        }
    }

    // ── World origin shifting (ENG-01 Phase 2) ──────────────────────────

    /// Current runtime world origin.
    ///
    /// Every `Transform.translation` (and every other f32 world-space
    /// runtime value) is stored **relative** to this origin; the logical
    /// position of an entity is `world_origin + world_position`. Zero until
    /// the first [`shift_world_origin`](Self::shift_world_origin) and reset
    /// by every scene load.
    pub fn world_origin(&self) -> [f64; 3] {
        self.runtime
            .with_world(|world| world.world_origin())
            .unwrap_or([0.0; 3])
    }

    /// Number of world-origin shifts performed since this loop was created.
    pub fn world_origin_shift_count(&self) -> u64 {
        self.world_origin_shift_count
    }

    /// Details of the most recent world-origin shift, if any.
    pub fn last_world_origin_shift(&self) -> Option<WorldOriginShift> {
        self.last_world_origin_shift
    }

    /// Evaluate the origin-shift trigger once and shift at most once.
    ///
    /// Intended call site: the frame boundary, after `update()` and
    /// scene-transition processing, alongside cell-streaming commits and
    /// before `render()` — never mid-frame. With
    /// [`SceneSettings::origin_shift`] disabled (the default) or without an
    /// active world this is a no-op.
    ///
    /// The reference position is the configured
    /// `reference_entity`'s world position, or the active camera's world
    /// position when unset. When its distance from the origin exceeds
    /// `threshold`, exactly one shift by the full reference position runs, so
    /// the reference lands back at the (relative) origin.
    pub fn tick_world_origin_shift(&mut self) -> Option<WorldOriginShift> {
        let settings = self
            .runtime
            .with_world(|world| world.scene_settings().origin_shift.clone())?;
        if !settings.enabled || !settings.threshold.is_finite() || settings.threshold <= 0.0 {
            return None;
        }
        let reference = self
            .runtime
            .with_world(|world| match settings.reference_entity.as_deref() {
                Some(id) => world
                    .entity_by_persistent_id(id)
                    .and_then(|entity| engine_scene::entity_world_position(world, entity)),
                None => engine_scene::active_camera_world_position(world),
            })
            .flatten()?;
        if !reference.is_finite() {
            return None;
        }
        if (reference.length() as f64) <= f64::from(settings.threshold) {
            return None;
        }
        self.shift_world_origin([
            f64::from(reference.x),
            f64::from(reference.y),
            f64::from(reference.z),
        ])
    }

    /// Shift the world origin by `delta`, preserving logical positions.
    ///
    /// This is the atomic consistency sweep behind
    /// [`tick_world_origin_shift`](Self::tick_world_origin_shift); hosts may
    /// also call it directly (e.g. from tests or a debug console) at a frame
    /// boundary. Every f32 world-space runtime value moves by `-delta` and
    /// [`World::world_origin`] advances by `delta`:
    ///
    /// - every root `Transform` in the ECS (children follow via the
    ///   hierarchy; disabled entities included),
    /// - every physics body, teleported in place with velocities, forces,
    ///   joints, and sleep state preserved (`gameplay` feature),
    /// - every `CharacterController` position, including the primary mirror
    ///   used by [`update_character`](Self::update_character),
    /// - every navigation agent's target and in-progress path
    ///   (`runtime-subsystems` feature),
    /// - every point `GravitySource` center (`gameplay` feature).
    ///
    /// Audio needs no sweep: emitter/listener snapshots are rebuilt from ECS
    /// transforms every `update()`, and emitters and listener shift together
    /// so relative audio geometry is seamless. Camera-relative rendering
    /// composes unchanged: extraction subtracts the *current* camera
    /// translation each frame, which is exactly what the shift rebased.
    ///
    /// Returns `None` when no world is active.
    pub fn shift_world_origin(&mut self, delta: [f64; 3]) -> Option<WorldOriginShift> {
        let offset = Vec3::new(delta[0] as f32, delta[1] as f32, delta[2] as f32);
        let (transforms, character_controllers, nav_agents, gravity_sources) =
            self.runtime.with_world_mut(|world| {
                let transforms = world.shift_world_origin(delta);

                let mut characters = 0usize;
                for (_, controller) in world.query_all_mut::<CharacterController>() {
                    let position = controller.position();
                    controller.set_position(position - offset);
                    characters += 1;
                }

                #[cfg(feature = "runtime-subsystems")]
                let nav_agents = {
                    let mut count = 0usize;
                    for (_, agent) in world.query_all_mut::<engine_nav::AiAgent>() {
                        agent.shift_world_positions(-offset);
                        count += 1;
                    }
                    count
                };
                #[cfg(not(feature = "runtime-subsystems"))]
                let nav_agents = 0usize;

                #[cfg(feature = "gameplay")]
                let gravity_sources = engine_physics::shift_gravity_source_centers(world, -offset);
                #[cfg(not(feature = "gameplay"))]
                let gravity_sources = 0usize;

                (transforms, characters, nav_agents, gravity_sources)
            })?;

        #[cfg(feature = "gameplay")]
        let physics_bodies = self
            .physics
            .as_mut()
            .map(|physics| physics.translate_bodies(-offset))
            .unwrap_or(0);
        #[cfg(not(feature = "gameplay"))]
        let physics_bodies = 0usize;

        // The primary character mirror is a clone of the component refreshed
        // every frame; keep it consistent between the shift and the next
        // update so a same-frame read cannot observe the pre-shift position.
        if let Some(controller) = self.character.as_mut() {
            let position = controller.position();
            controller.set_position(position - offset);
        }

        let shift = WorldOriginShift {
            delta,
            origin: self.world_origin(),
            transforms,
            physics_bodies,
            character_controllers,
            nav_agents,
            gravity_sources,
        };
        self.world_origin_shift_count += 1;
        self.last_world_origin_shift = Some(shift);
        tracing::info!(
            delta = ?shift.delta,
            origin = ?shift.origin,
            transforms = shift.transforms,
            physics_bodies = shift.physics_bodies,
            character_controllers = shift.character_controllers,
            nav_agents = shift.nav_agents,
            gravity_sources = shift.gravity_sources,
            count = self.world_origin_shift_count,
            "world origin shifted"
        );
        Some(shift)
    }

    /// Take the most recent physics event snapshot, leaving it empty.
    #[cfg(feature = "gameplay")]
    pub fn take_physics_events(&mut self) -> PhysicsEvents {
        std::mem::take(&mut self.physics_events)
    }

    /// Drive the kinematic character controller and sync its position back to
    /// the ECS world.  Call this each frame after processing player input.
    ///
    /// `direction` is a normalised horizontal movement vector.
    /// `wish_jump` is true when the player wants to jump this frame.
    /// `dt` is the frame delta time in seconds.
    pub fn update_character(&mut self, direction: Vec3, wish_jump: bool, dt: f32) {
        let Some(ref mut ctrl) = self.character else {
            return;
        };

        let input = CharacterMovement {
            direction,
            wish_jump,
            delta_time: dt.min(0.1),
        };

        // Drive the controller.  Physics world is optional — without it the
        // controller still moves but won't do ground collision.
        #[cfg(feature = "gameplay")]
        {
            let physics: Option<&PhysicsWorld> = self.physics.as_ref();
            ctrl.update(&input, physics);
        }
        #[cfg(not(feature = "gameplay"))]
        ctrl.update(&input, None);

        // Write controller position back to the ECS entity's Transform.
        if let Some(entity) = self.character_entity {
            let updated_controller = ctrl.clone();
            self.runtime.with_world_mut(|world| {
                use engine_scene::components::Transform;
                if let Some(t) = world.get_mut::<Transform>(entity) {
                    t.translation = ctrl.position();
                }
                if let Some(component) = world.get_mut::<CharacterController>(entity) {
                    *component = updated_controller;
                }
            });
        }
    }

    /// Advance the simulation by `dt` seconds.
    ///
    /// Handles physics stepping and ECS ↔ physics sync when the `gameplay`
    /// feature is enabled.  Script ticking runs when the
    /// `subsystem-scripting-csharp` feature is active.
    ///
    /// Typical per-frame orchestration:
    /// 1. Resolve input events against `input_map`
    /// 2. Call `update(dt)` for physics + character + scripts
    /// 3. Call `render(frame_idx)` for extraction + draw
    pub fn update(&mut self, dt: f32) {
        // Tick physics (ECS → physics → ECS sync) — gameplay feature
        #[cfg(feature = "gameplay")]
        {
            self.physics_events.clear();
            if let Some(ref mut physics) = self.physics {
                self.runtime.with_world_mut(|world| {
                    physics.step(dt, world);
                });
                self.physics_events = physics.drain_events();
            }
        }

        #[cfg(feature = "gameplay")]
        let (character_direction, character_jump) = self.resolved_character_input();
        #[cfg(not(feature = "gameplay"))]
        let (character_direction, character_jump) = (Vec3::ZERO, false);
        #[cfg(feature = "runtime-subsystems")]
        self.queue_runtime_navigation(dt);
        self.update_character(character_direction, character_jump, dt);
        self.update_additional_characters(dt);

        #[cfg(feature = "subsystem-scripting-csharp")]
        let script_ui_events = {
            #[cfg(feature = "runtime-subsystems")]
            {
                std::mem::take(&mut self.runtime_ui_events)
                    .into_iter()
                    .map(|event| engine_script::GameplayUiEvent {
                        canvas_id: event.canvas_id,
                        element_id: event.element_id,
                        callback_id: event.callback_id,
                        value: event.value.map(|value| match value {
                            RuntimeUiValue::Bool(value) => {
                                engine_script::GameplayUiValue::Bool(value)
                            }
                            RuntimeUiValue::Float(value) => {
                                engine_script::GameplayUiValue::Float(value)
                            }
                        }),
                    })
                    .collect::<Vec<_>>()
            }
            #[cfg(not(feature = "runtime-subsystems"))]
            {
                Vec::<engine_script::GameplayUiEvent>::new()
            }
        };

        // Tick scripts (OnUpdate) with the same resolved input snapshot used
        // by the player/editor GameLoop.
        #[cfg(all(feature = "subsystem-scripting-csharp", feature = "gameplay"))]
        {
            let input_actions = self.resolved_script_input_actions();
            let input_transitions = self.resolved_script_input_transitions(&input_actions);
            let physics_events = self.resolved_script_physics_events();
            let physics_query_results = std::mem::take(&mut self.script_physics_query_results);
            self.runtime
                .tick_scripts_with_frame_input_ui_and_physics_queries(
                    dt,
                    &input_actions,
                    &input_transitions,
                    &physics_events,
                    &script_ui_events,
                    &physics_query_results,
                );
            self.previous_script_input_actions = input_actions;
            // Queries drained from this tick execute against the freshly
            // stepped physics world; scripts observe the results with the
            // next frame snapshot.
            self.execute_script_physics_queries();
        }
        #[cfg(all(feature = "subsystem-scripting-csharp", not(feature = "gameplay")))]
        {
            self.runtime.tick_scripts_with_frame_input_and_ui(
                dt,
                &std::collections::BTreeMap::new(),
                &engine_script::GameplayInputTransitions::default(),
                &std::collections::BTreeMap::new(),
                &script_ui_events,
            );
            // Without a physics world no query can be answered; drop drained
            // queries so the runtime queue cannot accumulate.
            let _ = self.runtime.take_pending_physics_queries();
        }

        #[cfg(feature = "runtime-subsystems")]
        self.update_runtime_animation(dt);
        #[cfg(feature = "runtime-audio-output")]
        self.update_runtime_audio(dt);
    }

    #[cfg(feature = "gameplay")]
    fn resolved_character_input(&self) -> (Vec3, bool) {
        use engine_gameplay::InputValue;

        let digital = |name: &str| {
            self.input_map
                .action(name)
                .map_or(0.0, |action| match &action.current_value {
                    InputValue::Bool(true) => 1.0,
                    InputValue::Float(value) => value.clamp(-1.0, 1.0),
                    InputValue::Bool(false) | InputValue::Vec2(_) => 0.0,
                })
        };
        let analog = self
            .input_map
            .action("move")
            .and_then(|action| match &action.current_value {
                InputValue::Vec2(value) => Some(*value),
                _ => None,
            })
            .unwrap_or(glam::Vec2::ZERO);
        let direction = Vec3::new(
            analog.x + digital("move_right") - digital("move_left"),
            0.0,
            -analog.y - digital("move_forward") + digital("move_backward"),
        )
        .normalize_or_zero();
        let wish_jump = self.input_map.action("jump").is_some_and(|action| {
            matches!(&action.current_value, InputValue::Bool(true))
                || matches!(&action.current_value, InputValue::Float(value) if *value > 0.5)
        });
        (direction, wish_jump)
    }

    /// Produce a single rendered frame.
    pub fn render(&mut self, frame_index: u64) -> Result<FrameStats, Vec<Diagnostic>> {
        #[cfg(feature = "runtime-subsystems")]
        {
            let ui_batches = self.runtime_ui_batches();
            self.runtime.render_frame_with_ui(frame_index, ui_batches)
        }
        #[cfg(not(feature = "runtime-subsystems"))]
        {
            self.runtime.render_frame(frame_index)
        }
    }

    /// Render the scene, retained game UI and engine-native overlays inside an
    /// embedded viewport. The desktop editor shell is composed by the OS and
    /// never enters this render path.
    pub fn render_embedded_viewport(
        &mut self,
        frame_index: u64,
        engine_overlay_batches: Vec<engine_renderer::UiBatch>,
        viewport: RenderViewportContext,
    ) -> Result<FrameStats, Vec<Diagnostic>> {
        #[cfg(feature = "runtime-subsystems")]
        let ui_batches = {
            let surface_size = viewport.surface_size();
            let output = viewport.output_rect();
            let extent = [
                output.width() * surface_size[0] as f32,
                output.height() * surface_size[1] as f32,
            ];
            self.runtime_ui_viewport = extent;
            let mut scene_ui_batches = self.runtime_ui_batches();
            embed_scene_ui_batches(&mut scene_ui_batches, viewport);
            scene_ui_batches.extend(engine_overlay_batches);
            scene_ui_batches
        };
        #[cfg(not(feature = "runtime-subsystems"))]
        let ui_batches = engine_overlay_batches;

        self.runtime
            .render_frame_with_ui_in_viewport(frame_index, ui_batches, viewport)
    }

    /// Drain retained Canvas click events for a native host.
    ///
    /// Script-enabled [`update`](Self::update) consumes the same queue once
    /// when building gameplay contexts. Native hosts that want ownership must
    /// therefore call this before that update.
    #[cfg(feature = "runtime-subsystems")]
    pub fn take_ui_events(&mut self) -> Vec<RuntimeUiEvent> {
        std::mem::take(&mut self.runtime_ui_events)
    }

    /// Whether a scene Canvas currently owns the primary pointer gesture.
    #[cfg(feature = "runtime-subsystems")]
    pub fn ui_has_pointer_capture(&self) -> bool {
        self.runtime_ui_captured_canvas.is_some()
    }

    /// Update the screen viewport used by retained UI scaling and hit tests.
    #[cfg(feature = "runtime-subsystems")]
    pub fn set_ui_viewport_size(&mut self, width: u32, height: u32) {
        self.runtime_ui_viewport = [width.max(1) as f32, height.max(1) as f32];
    }

    /// Update the retained UI primary-pointer position in Canvas coordinates.
    ///
    /// While a Canvas owns capture, movement is delivered only to that
    /// Canvas. Otherwise the topmost interactive Canvas under the pointer is
    /// selected using the same persistent-ID order as UI rendering.
    #[cfg(feature = "runtime-subsystems")]
    pub fn ui_pointer_move(&mut self, x: f32, y: f32) {
        if !x.is_finite() || !y.is_finite() {
            self.cancel_ui_pointer();
            return;
        }
        self.runtime_ui_pointer = [x, y];

        let mut canvases = self.runtime_ui_canvases();
        if let Some(captured_canvas) = self.runtime_ui_captured_canvas.clone() {
            let Some((_, canvas)) = canvases
                .iter_mut()
                .find(|(canvas_id, _)| canvas_id == &captured_canvas)
            else {
                self.cancel_ui_pointer_state();
                return;
            };
            let [canvas_x, canvas_y] = self.runtime_ui_canvas_point(canvas, x, y);
            let value_change = self
                .runtime_ui_input_states
                .entry(captured_canvas.clone())
                .or_default()
                .process_event(
                    canvas,
                    engine_ui::UiPointerEvent::Move {
                        x: canvas_x,
                        y: canvas_y,
                    },
                );
            self.commit_runtime_ui_canvas(&captured_canvas, canvas.clone());
            if let Some(value_change) = value_change {
                self.runtime_ui_events.push(RuntimeUiEvent {
                    canvas_id: captured_canvas,
                    element_id: value_change.element_id.0,
                    callback_id: value_change.callback_id,
                    value: value_change.value.map(|value| match value {
                        engine_ui::UiValue::Bool(value) => RuntimeUiValue::Bool(value),
                        engine_ui::UiValue::Float(value) => RuntimeUiValue::Float(value),
                    }),
                });
            }
            return;
        }

        let hovered_canvas = canvases
            .iter()
            .rev()
            .find(|(_, canvas)| {
                let [canvas_x, canvas_y] = self.runtime_ui_canvas_point(canvas, x, y);
                engine_ui::hit_test_interactive(canvas, canvas_x, canvas_y).is_some()
            })
            .map(|(canvas_id, _)| canvas_id.clone());
        for (canvas_id, state) in &mut self.runtime_ui_input_states {
            if hovered_canvas.as_deref() != Some(canvas_id.as_str()) {
                state.reset();
            }
        }
        if let Some(canvas_id) = hovered_canvas {
            let canvas = canvases
                .iter_mut()
                .find_map(|(candidate, canvas)| (candidate == &canvas_id).then_some(canvas))
                .expect("hovered Canvas came from the same snapshot");
            let [canvas_x, canvas_y] = self.runtime_ui_canvas_point(canvas, x, y);
            self.runtime_ui_input_states
                .entry(canvas_id)
                .or_default()
                .process_event(
                    canvas,
                    engine_ui::UiPointerEvent::Move {
                        x: canvas_x,
                        y: canvas_y,
                    },
                );
        }
    }

    /// Press the primary pointer at its most recently supplied position.
    ///
    /// Exactly one topmost Canvas can capture a press. Presses outside all
    /// interactive elements leave the UI uncaptured.
    #[cfg(feature = "runtime-subsystems")]
    pub fn ui_pointer_left_press(&mut self) {
        self.cancel_ui_pointer_state();
        let [x, y] = self.runtime_ui_pointer;
        let mut canvases = self.runtime_ui_canvases();
        let pressed_canvas = canvases
            .iter()
            .rev()
            .find(|(_, canvas)| {
                let [canvas_x, canvas_y] = self.runtime_ui_canvas_point(canvas, x, y);
                engine_ui::hit_test_interactive(canvas, canvas_x, canvas_y).is_some()
            })
            .map(|(canvas_id, _)| canvas_id.clone());
        let Some(canvas_id) = pressed_canvas else {
            return;
        };
        let canvas = canvases
            .iter_mut()
            .find_map(|(candidate, canvas)| (candidate == &canvas_id).then_some(canvas))
            .expect("pressed Canvas came from the same snapshot");
        let [canvas_x, canvas_y] = self.runtime_ui_canvas_point(canvas, x, y);
        let captured = {
            let state = self
                .runtime_ui_input_states
                .entry(canvas_id.clone())
                .or_default();
            state.process_event(
                canvas,
                engine_ui::UiPointerEvent::Press {
                    x: canvas_x,
                    y: canvas_y,
                },
            );
            state.capture.is_some()
        };
        self.commit_runtime_ui_canvas(&canvas_id, canvas.clone());
        if captured {
            self.runtime_ui_captured_canvas = Some(canvas_id);
        }
    }

    /// Release the primary pointer at its most recently supplied position.
    ///
    /// A click is queued only when the Canvas and element captured by press
    /// still exist and the release remains inside that enabled element.
    #[cfg(feature = "runtime-subsystems")]
    pub fn ui_pointer_left_release(&mut self) {
        let [x, y] = self.runtime_ui_pointer;
        let Some(canvas_id) = self.runtime_ui_captured_canvas.take() else {
            return;
        };
        let mut canvases = self.runtime_ui_canvases();
        let Some(canvas) = canvases
            .iter_mut()
            .find_map(|(candidate, canvas)| (candidate == &canvas_id).then_some(canvas))
        else {
            self.runtime_ui_input_states.remove(&canvas_id);
            return;
        };
        let [canvas_x, canvas_y] = self.runtime_ui_canvas_point(canvas, x, y);
        let click = self
            .runtime_ui_input_states
            .entry(canvas_id.clone())
            .or_default()
            .process_event(
                canvas,
                engine_ui::UiPointerEvent::Release {
                    x: canvas_x,
                    y: canvas_y,
                },
            );
        self.commit_runtime_ui_canvas(&canvas_id, canvas.clone());
        if let Some(click) = click {
            self.runtime_ui_events.push(RuntimeUiEvent {
                canvas_id,
                element_id: click.element_id.0,
                callback_id: click.callback_id,
                value: click.value.map(|value| match value {
                    engine_ui::UiValue::Bool(value) => RuntimeUiValue::Bool(value),
                    engine_ui::UiValue::Float(value) => RuntimeUiValue::Float(value),
                }),
            });
        }
    }

    /// Cancel a retained UI gesture without producing a click.
    ///
    /// Window focus loss, suspension, or an editor release over chrome must
    /// use this path so a later release cannot resurrect an old press.
    #[cfg(feature = "runtime-subsystems")]
    pub fn cancel_ui_pointer(&mut self) {
        let mut canvases = self.runtime_ui_canvases();
        for (canvas_id, canvas) in &mut canvases {
            if let Some(state) = self.runtime_ui_input_states.get_mut(canvas_id) {
                state.process_event(canvas, engine_ui::UiPointerEvent::Cancel);
            }
        }
        self.cancel_ui_pointer_state();
    }

    #[cfg(feature = "runtime-subsystems")]
    fn cancel_ui_pointer_state(&mut self) {
        self.runtime_ui_input_states.clear();
        self.runtime_ui_captured_canvas = None;
    }

    #[cfg(feature = "runtime-subsystems")]
    fn reset_runtime_ui_input(&mut self) {
        self.cancel_ui_pointer_state();
        self.runtime_ui_events.clear();
    }

    /// Snapshot and lay out all retained scene canvases in renderer order.
    #[cfg(feature = "runtime-subsystems")]
    fn runtime_ui_canvases(&mut self) -> Vec<(String, engine_ui::Canvas)> {
        self.runtime
            .with_world_mut(|world| {
                let mut canvases = world
                    .query::<engine_ui::Canvas>()
                    .filter_map(|(entity, _)| {
                        world
                            .persistent_id(entity)
                            .map(|id| (id.to_owned(), entity))
                    })
                    .collect::<Vec<_>>();
                canvases.sort_by(|left, right| left.0.cmp(&right.0));
                canvases
                    .into_iter()
                    .filter_map(|(canvas_id, entity)| {
                        let canvas = world.get_mut::<engine_ui::Canvas>(entity)?;
                        canvas.layout_all();
                        Some((canvas_id, canvas.clone()))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    #[cfg(feature = "runtime-subsystems")]
    fn runtime_ui_canvas_point(&self, canvas: &engine_ui::Canvas, x: f32, y: f32) -> [f32; 2] {
        let viewport_width = if self.runtime_ui_viewport[0] > 0.0 {
            self.runtime_ui_viewport[0]
        } else {
            canvas.width
        };
        let viewport_height = if self.runtime_ui_viewport[1] > 0.0 {
            self.runtime_ui_viewport[1]
        } else {
            canvas.height
        };
        let scale = engine_ui::canvas_scale(canvas, viewport_width, viewport_height);
        if scale.is_finite() && scale > 0.0 {
            [x / scale, y / scale]
        } else {
            [x, y]
        }
    }

    #[cfg(feature = "runtime-subsystems")]
    fn commit_runtime_ui_canvas(&mut self, canvas_id: &str, canvas: engine_ui::Canvas) {
        self.runtime.with_world_mut(|world| {
            let Some(entity) = world.entity_by_persistent_id(canvas_id) else {
                return;
            };
            if let Some(target) = world.get_mut::<engine_ui::Canvas>(entity) {
                *target = canvas;
            }
        });
    }

    /// Resolve retained-mode scene canvases into renderer batches for the
    /// current frame. Canvas order is based on persistent entity IDs so the
    /// result is stable even when ECS storage order changes.
    #[cfg(feature = "runtime-subsystems")]
    fn runtime_ui_batches(&mut self) -> Vec<engine_renderer::UiBatch> {
        let input_states = self.runtime_ui_input_states.clone();
        let viewport = self.runtime_ui_viewport;
        let batches = self
            .runtime
            .with_world_mut(|world| {
                let mut canvases = world
                    .query::<engine_ui::Canvas>()
                    .filter_map(|(entity, _)| {
                        world
                            .persistent_id(entity)
                            .map(|id| (id.to_owned(), entity))
                    })
                    .collect::<Vec<_>>();
                canvases.sort_by(|left, right| left.0.cmp(&right.0));

                canvases
                    .into_iter()
                    .flat_map(|(canvas_id, entity)| {
                        let Some(canvas) = world.get_mut::<engine_ui::Canvas>(entity) else {
                            return Vec::new();
                        };
                        canvas.layout_all();
                        let viewport_width = if viewport[0] > 0.0 {
                            viewport[0]
                        } else {
                            canvas.width
                        };
                        let viewport_height = if viewport[1] > 0.0 {
                            viewport[1]
                        } else {
                            canvas.height
                        };
                        let mut batches = canvas.build_batches_for_viewport(
                            viewport_width,
                            viewport_height,
                            input_states.get(&canvas_id),
                        );
                        for batch in &mut batches {
                            batch.canvas_id.clone_from(&canvas_id);
                        }
                        batches
                    })
                    .collect()
            })
            .unwrap_or_default();

        batches
    }

    /// Advance scene animation players and replace their static renderer
    /// extraction with skinned items backed by the loaded extension assets.
    #[cfg(feature = "runtime-subsystems")]
    fn update_runtime_animation(&mut self, dt: f32) {
        let asset_ids = self.runtime.asset_registry().cached_ids();
        let skeletons = asset_ids
            .iter()
            .filter_map(|id| {
                self.runtime
                    .extension_asset::<engine_animation::Skeleton>("skeleton", id)
                    .map(|handle| (id.id.clone(), handle.get().clone()))
            })
            .collect::<std::collections::HashMap<_, _>>();
        let clips = asset_ids
            .iter()
            .filter_map(|id| {
                self.runtime
                    .extension_asset::<engine_animation::AnimationClip>("animation_clip", id)
                    .map(|handle| (id.id.clone(), handle.get().clone()))
            })
            .collect::<std::collections::HashMap<_, _>>();
        let producer = self
            .runtime
            .animation_extension_handles()
            .skinned_extract
            .clone();

        // Multiple fixed updates may run before one render. Only the latest
        // evaluated pose belongs in the next frame.
        producer.drain();
        let _ = self.runtime.with_world_mut(|world| {
            engine_animation::bridge_skinned_items(world, &skeletons, &clips, &producer, dt);
        });
    }

    /// Synchronise scene audio components with the lazily-created desktop
    /// output device. Device failures are recoverable: the rest of the game
    /// loop continues and another scene load rearms one initialization attempt.
    #[cfg(feature = "runtime-audio-output")]
    fn update_runtime_audio(&mut self, dt: f32) {
        let frame = self.runtime_audio_frame();
        self.audio_output.update(&self.runtime, frame, dt);
    }

    #[cfg(feature = "runtime-audio-output")]
    fn reset_runtime_audio_scene(&mut self) {
        self.audio_output.reset_scene();
    }

    #[cfg(feature = "runtime-audio-output")]
    fn runtime_audio_frame(&self) -> RuntimeAudioFrame {
        let Some((mut sources, listener)) = self.runtime.with_world(|world| {
            let mut source_entities = world
                .query::<engine_audio::AudioSourceComponent>()
                .map(|(entity, _)| entity)
                .collect::<Vec<_>>();
            source_entities.sort_by(|left, right| {
                world.persistent_id(*left).cmp(&world.persistent_id(*right))
            });

            let sources = source_entities
                .into_iter()
                .filter_map(|entity| {
                    let entity_id = world.persistent_id(entity)?.to_owned();
                    let component = world
                        .get::<engine_audio::AudioSourceComponent>(entity)?
                        .clone();
                    let pose = resolved_audio_pose(world, entity);
                    let emitter = component.spatial.then_some(RuntimeAudioEmitterSnapshot {
                        position: pose.position,
                        max_distance: component.max_distance.max(0.0),
                        rolloff_factor: component.rolloff_factor.max(0.0),
                    });
                    Some(RuntimeAudioSourceSnapshot {
                        entity_id,
                        clip_asset: component.clip_asset,
                        clip_loaded: false,
                        playing: component.playing,
                        volume: component.volume.clamp(0.0, 1.0),
                        looping: component.looping,
                        emitter,
                    })
                })
                .collect::<Vec<_>>();

            let mut listener_entities = world
                .query::<engine_audio::AudioListenerComponent>()
                .filter_map(|(entity, listener)| listener.enabled.then_some(entity))
                .collect::<Vec<_>>();
            listener_entities.sort_by(|left, right| {
                world.persistent_id(*left).cmp(&world.persistent_id(*right))
            });
            let listener = listener_entities.first().map(|entity| {
                let pose = resolved_audio_pose(world, *entity);
                RuntimeAudioListenerSnapshot {
                    position: pose.position,
                    forward: pose.forward,
                    up: pose.up,
                }
            });
            (sources, listener)
        }) else {
            return RuntimeAudioFrame::default();
        };

        for source in &mut sources {
            source.clip_loaded = source.clip_asset.as_ref().is_some_and(|clip_asset| {
                self.runtime
                    .extension_asset::<engine_audio::AudioClip>(
                        "audio_clip",
                        &engine_serialize::AssetId::new(clip_asset),
                    )
                    .is_some()
            });
        }

        RuntimeAudioFrame { sources, listener }
    }

    /// Evaluate navigation agents and queue their movement intent on the
    /// CharacterController attached to the same entity. The primary player
    /// mirror is refreshed so its normal update consumes the queued command.
    #[cfg(feature = "runtime-subsystems")]
    fn queue_runtime_navigation(&mut self, dt: f32) {
        let navmeshes = self
            .runtime
            .asset_registry()
            .cached_ids()
            .into_iter()
            .filter_map(|id| {
                self.runtime
                    .extension_asset::<engine_nav::NavMesh>("navmesh", &id)
                    .map(|handle| (id.id, handle.get().clone()))
            })
            .collect::<std::collections::HashMap<_, _>>();
        let primary = self.character_entity;
        let updated_primary = self
            .runtime
            .with_world_mut(|world| {
                // Path queries run in the navmesh's authored (logical)
                // space; agent and controller state stays origin-relative.
                let origin = {
                    let origin = world.world_origin();
                    Vec3::new(origin[0] as f32, origin[1] as f32, origin[2] as f32)
                };
                let entities = world
                    .query::<engine_nav::AiAgent>()
                    .map(|(entity, _)| entity)
                    .collect::<Vec<_>>();
                let mut updated_primary = None;
                for entity in entities {
                    let Some(mut agent) = world.get::<engine_nav::AiAgent>(entity).cloned() else {
                        continue;
                    };
                    // Persistent scene data cannot safely encode a raw ECS
                    // generation. Zero therefore means the supported and
                    // portable same-entity controller binding.
                    if agent.controller_entity_id != 0 {
                        continue;
                    }
                    let Some(navmesh_id) = agent.navmesh_ref.as_deref() else {
                        continue;
                    };
                    let Some(navmesh) = navmeshes.get(navmesh_id) else {
                        continue;
                    };
                    let Some(mut controller) = world.get::<CharacterController>(entity).cloned()
                    else {
                        continue;
                    };
                    engine_nav::update_ai_agent_with_world_origin(
                        &mut agent,
                        &mut controller,
                        navmesh,
                        dt,
                        origin,
                    );
                    if let Some(component) = world.get_mut::<engine_nav::AiAgent>(entity) {
                        *component = agent;
                    }
                    if let Some(component) = world.get_mut::<CharacterController>(entity) {
                        *component = controller.clone();
                    }
                    if Some(entity) == primary {
                        updated_primary = Some(controller);
                    }
                }
                updated_primary
            })
            .flatten();
        if let Some(controller) = updated_primary {
            self.character = Some(controller);
        }
    }

    /// Advance every non-primary CharacterController so AI characters and
    /// ambient pawns are not frozen merely because they are not player-bound.
    fn update_additional_characters(&mut self, dt: f32) {
        let primary = self.character_entity;
        #[cfg(feature = "gameplay")]
        let physics = self.physics.as_ref();
        let _ = self.runtime.with_world_mut(|world| {
            let entities = world
                .query::<CharacterController>()
                .map(|(entity, _)| entity)
                .filter(|entity| Some(*entity) != primary)
                .collect::<Vec<_>>();
            for entity in entities {
                let Some(mut controller) = world.get::<CharacterController>(entity).cloned() else {
                    continue;
                };
                let input = CharacterMovement {
                    direction: Vec3::ZERO,
                    wish_jump: false,
                    delta_time: dt.min(0.1),
                };
                #[cfg(feature = "gameplay")]
                controller.update(&input, physics);
                #[cfg(not(feature = "gameplay"))]
                controller.update(&input, None);
                if let Some(transform) =
                    world.get_mut::<engine_scene::components::Transform>(entity)
                {
                    transform.translation = controller.position();
                }
                if let Some(component) = world.get_mut::<CharacterController>(entity) {
                    *component = controller;
                }
            }
        });
    }

    #[cfg(all(feature = "subsystem-scripting-csharp", feature = "gameplay"))]
    fn resolved_script_input_actions(
        &self,
    ) -> std::collections::BTreeMap<String, engine_script::GameplayInputValue> {
        self.input_map
            .actions
            .iter()
            .map(|action| {
                let value = match &action.current_value {
                    engine_gameplay::InputValue::Bool(value) => {
                        engine_script::GameplayInputValue::Bool(*value)
                    }
                    engine_gameplay::InputValue::Float(value) => {
                        engine_script::GameplayInputValue::Float(*value)
                    }
                    engine_gameplay::InputValue::Vec2(value) => {
                        engine_script::GameplayInputValue::Vec2(value.to_array())
                    }
                };
                (action.name.clone(), value)
            })
            .collect()
    }

    #[cfg(all(feature = "subsystem-scripting-csharp", feature = "gameplay"))]
    fn resolved_script_physics_events(
        &self,
    ) -> std::collections::BTreeMap<String, Vec<engine_script::GameplayPhysicsEvent>> {
        use engine_physics::{CollisionEventKind, TriggerEventKind};
        use engine_script::{GameplayPhysicsEvent, GameplayPhysicsEventKind};

        self.runtime
            .with_world(|world| {
                let mut by_entity =
                    std::collections::BTreeMap::<String, Vec<GameplayPhysicsEvent>>::new();
                let mut record_pair =
                    |entity_a, entity_b, kind: GameplayPhysicsEventKind| {
                        let Some(entity_a) = world.persistent_id(entity_a) else {
                            return;
                        };
                        let Some(entity_b) = world.persistent_id(entity_b) else {
                            return;
                        };
                        by_entity.entry(entity_a.to_owned()).or_default().push(
                            GameplayPhysicsEvent {
                                kind,
                                other_entity_id: entity_b.to_owned(),
                            },
                        );
                        by_entity.entry(entity_b.to_owned()).or_default().push(
                            GameplayPhysicsEvent {
                                kind,
                                other_entity_id: entity_a.to_owned(),
                            },
                        );
                    };

                for event in &self.physics_events.collisions {
                    let kind = match event.kind {
                        CollisionEventKind::ContactStarted => {
                            GameplayPhysicsEventKind::CollisionEntered
                        }
                        CollisionEventKind::ContactStaying => {
                            GameplayPhysicsEventKind::CollisionStayed
                        }
                        CollisionEventKind::ContactStopped => {
                            GameplayPhysicsEventKind::CollisionExited
                        }
                    };
                    record_pair(event.entity_a, event.entity_b, kind);
                }
                for event in &self.physics_events.triggers {
                    let kind = match event.kind {
                        TriggerEventKind::Entered => GameplayPhysicsEventKind::TriggerEntered,
                        TriggerEventKind::Stay => GameplayPhysicsEventKind::TriggerStayed,
                        TriggerEventKind::Exited => GameplayPhysicsEventKind::TriggerExited,
                    };
                    record_pair(event.entity_a, event.entity_b, kind);
                }
                by_entity
            })
            .unwrap_or_default()
    }

    /// Execute the physics queries drained from the latest script update and
    /// stage the results for the next frame snapshot.
    ///
    /// Queries run against the physics world after this frame's step and ECS
    /// sync, so answers are consistent with the physics events delivered in
    /// the same update. Results are frame-local: they replace the previous
    /// staging map and are consumed by the next script tick.
    #[cfg(all(feature = "subsystem-scripting-csharp", feature = "gameplay"))]
    fn execute_script_physics_queries(&mut self) {
        let pending = self.runtime.take_pending_physics_queries();
        if pending.is_empty() {
            return;
        }
        let mut results = std::collections::BTreeMap::<
            String,
            Vec<engine_script::GameplayPhysicsQueryResult>,
        >::new();
        for engine_script::OwnedGameplayPhysicsQuery { entity_id, query } in pending {
            let result = self.execute_script_physics_query(&query);
            results.entry(entity_id).or_default().push(result);
        }
        self.script_physics_query_results = results;
    }

    /// Run one validated script physics query against the physics world,
    /// translating backend hits into persistent entity ids so scripts never
    /// observe raw ECS handles.
    #[cfg(all(feature = "subsystem-scripting-csharp", feature = "gameplay"))]
    fn execute_script_physics_query(
        &self,
        query: &engine_script::GameplayPhysicsQuery,
    ) -> engine_script::GameplayPhysicsQueryResult {
        use engine_script::{GameplayPhysicsQuery, GameplayPhysicsQueryResult};

        match *query {
            GameplayPhysicsQuery::Raycast {
                query_id,
                origin,
                direction,
                max_distance,
            } => {
                let miss = || GameplayPhysicsQueryResult::RaycastMiss { query_id };
                let Some(physics) = self.physics.as_ref() else {
                    return miss();
                };
                let direction = Vec3::from(direction).normalize_or_zero();
                if direction == Vec3::ZERO {
                    return miss();
                }
                let max_distance = max_distance.min(engine_script::MAX_PHYSICS_QUERY_DISTANCE);
                let Some(hit) = physics.raycast(Vec3::from(origin), direction, max_distance) else {
                    return miss();
                };
                let entity_id = self
                    .runtime
                    .with_world(|world| world.persistent_id(hit.entity).map(str::to_owned))
                    .flatten();
                match entity_id {
                    Some(entity_id) => GameplayPhysicsQueryResult::RaycastHit {
                        query_id,
                        entity_id,
                        point: hit.point.to_array(),
                        normal: hit.normal.to_array(),
                        distance: hit.distance,
                    },
                    // A collider without a persistent id cannot be named to
                    // scripts, so the query reports no usable hit.
                    None => miss(),
                }
            }
            GameplayPhysicsQuery::OverlapSphere {
                query_id,
                center,
                radius,
            } => {
                let mut entity_ids = Vec::new();
                if let Some(physics) = self.physics.as_ref() {
                    let radius = radius.min(engine_script::MAX_PHYSICS_QUERY_DISTANCE);
                    let hits = physics.query_proximity(
                        &engine_physics::ColliderShape::Ball { radius },
                        Vec3::from(center),
                    );
                    let persistent_ids = self
                        .runtime
                        .with_world(|world| {
                            hits.iter()
                                .filter_map(|entity| {
                                    world.persistent_id(*entity).map(str::to_owned)
                                })
                                .collect::<std::collections::BTreeSet<_>>()
                        })
                        .unwrap_or_default();
                    entity_ids.extend(
                        persistent_ids
                            .into_iter()
                            .take(engine_script::MAX_PHYSICS_OVERLAP_RESULTS),
                    );
                }
                GameplayPhysicsQueryResult::OverlapSphere {
                    query_id,
                    entity_ids,
                }
            }
        }
    }

    #[cfg(all(feature = "subsystem-scripting-csharp", feature = "gameplay"))]
    fn resolved_script_input_transitions(
        &self,
        current: &std::collections::BTreeMap<String, engine_script::GameplayInputValue>,
    ) -> engine_script::GameplayInputTransitions {
        let action_names = self
            .previous_script_input_actions
            .keys()
            .chain(current.keys())
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        let mut transitions = engine_script::GameplayInputTransitions::default();
        for action_name in action_names {
            let was_active = self
                .previous_script_input_actions
                .get(&action_name)
                .is_some_and(script_input_value_is_active);
            let is_active = current
                .get(&action_name)
                .is_some_and(script_input_value_is_active);
            if is_active && !was_active {
                transitions.pressed.insert(action_name);
            } else if was_active && !is_active {
                transitions.released.insert(action_name);
            }
        }
        transitions
    }

    /// Validate that the runtime has a loaded scene ready for rendering.
    pub fn validate_ready(&self) -> Result<(), Vec<Diagnostic>> {
        if !self.runtime.has_world() {
            return Err(vec![Diagnostic::new(
                "GL0001",
                DiagnosticSeverity::Error,
                "game_loop",
                "no active World is loaded; call load_scene() or set_world() first",
            )]);
        }
        Ok(())
    }
}

#[cfg(feature = "runtime-audio-output")]
#[derive(Clone, Copy)]
struct RuntimeAudioPose {
    position: Vec3,
    forward: Vec3,
    up: Vec3,
}

#[cfg(feature = "runtime-audio-output")]
fn resolved_audio_pose(
    world: &engine_scene::World,
    entity: engine_scene::Entity,
) -> RuntimeAudioPose {
    let mut chain = Vec::new();
    let mut visited = Vec::new();
    let mut cursor = Some(entity);
    while let Some(current) = cursor {
        if visited.contains(&current) {
            break;
        }
        visited.push(current);
        let Some(transform) = world
            .get::<engine_scene::components::Transform>(current)
            .cloned()
        else {
            break;
        };
        cursor = transform.parent;
        chain.push(transform);
    }

    let mut matrix = glam::Mat4::IDENTITY;
    for transform in chain.iter().rev() {
        matrix *= glam::Mat4::from_scale_rotation_translation(
            transform.scale,
            transform.rotation,
            transform.translation,
        );
    }
    let position = matrix.transform_point3(Vec3::ZERO);
    let forward = matrix.transform_vector3(-Vec3::Z).normalize_or_zero();
    let up = matrix.transform_vector3(Vec3::Y).normalize_or_zero();
    RuntimeAudioPose {
        position: if position.is_finite() {
            position
        } else {
            Vec3::ZERO
        },
        forward: if forward != Vec3::ZERO {
            forward
        } else {
            -Vec3::Z
        },
        up: if up != Vec3::ZERO { up } else { Vec3::Y },
    }
}

#[cfg(feature = "runtime-audio-output")]
fn runtime_audio_emitter(snapshot: &RuntimeAudioEmitterSnapshot) -> engine_audio::AudioEmitter {
    let mut emitter = engine_audio::AudioEmitter::new(snapshot.position);
    emitter.set_max_distance(snapshot.max_distance);
    emitter.set_rolloff_factor(snapshot.rolloff_factor);
    emitter
}

#[cfg(feature = "runtime-audio-output")]
fn runtime_audio_listener(
    snapshot: Option<&RuntimeAudioListenerSnapshot>,
) -> engine_audio::AudioListener {
    let mut listener = engine_audio::AudioListener::new();
    if let Some(snapshot) = snapshot {
        listener.set_position(snapshot.position);
        listener.set_orientation(snapshot.forward, snapshot.up);
    }
    listener
}

#[cfg(feature = "runtime-audio-output")]
impl RuntimeAudioOutput {
    fn update(&mut self, runtime: &EngineRuntime, frame: RuntimeAudioFrame, dt: f32) {
        let wants_output = frame
            .sources
            .iter()
            .any(|source| source.playable_clip().is_some());
        if !self.ensure_engine_with(wants_output, engine_audio::AudioEngine::new) {
            return;
        }

        let finished_entities = self
            .handles
            .iter()
            .filter_map(|(entity_id, handle)| handle.is_finished().then_some(entity_id.clone()))
            .collect::<BTreeSet<_>>();
        let actions = self
            .reconciler
            .reconcile(&frame.sources, &finished_entities);
        let engine = self
            .engine
            .as_mut()
            .expect("audio engine initialized above");
        engine.set_listener(runtime_audio_listener(frame.listener.as_ref()));

        let mut output_failed = false;
        let mut missing_starts = Vec::new();
        for action in actions {
            match action {
                RuntimeAudioSourceAction::Start(source) => {
                    let Some(clip_asset) = source.playable_clip() else {
                        continue;
                    };
                    let Some(clip) = runtime
                        .extension_asset::<engine_audio::AudioClip>(
                            "audio_clip",
                            &engine_serialize::AssetId::new(clip_asset),
                        )
                        .map(|handle| handle.shared())
                    else {
                        missing_starts.push(source.entity_id);
                        continue;
                    };
                    if let Some(previous) = self.handles.remove(&source.entity_id) {
                        let _ = engine.stop(previous.id());
                    }
                    let started = if let Some(emitter) = source.emitter.as_ref() {
                        engine.play_spatial(clip, runtime_audio_emitter(emitter))
                    } else {
                        engine.play(clip)
                    };
                    let Ok(mut handle) = started else {
                        output_failed = true;
                        break;
                    };
                    if handle.set_volume(source.volume).is_err()
                        || handle.set_loop(source.looping).is_err()
                    {
                        let _ = engine.stop(handle.id());
                        output_failed = true;
                        break;
                    }
                    self.handles.insert(source.entity_id, handle);
                }
                RuntimeAudioSourceAction::Update(source) => {
                    let Some(handle) = self.handles.get_mut(&source.entity_id) else {
                        missing_starts.push(source.entity_id);
                        continue;
                    };
                    if handle.set_volume(source.volume).is_err()
                        || handle.set_loop(source.looping).is_err()
                    {
                        output_failed = true;
                        break;
                    }
                    let _ = engine.set_emitter(
                        handle.id(),
                        source.emitter.as_ref().map(runtime_audio_emitter),
                    );
                }
                RuntimeAudioSourceAction::Stop { entity_id } => {
                    if let Some(handle) = self.handles.remove(&entity_id) {
                        let _ = engine.stop(handle.id());
                    }
                }
            }
        }

        for entity_id in missing_starts {
            self.reconciler.voices.remove(&entity_id);
        }

        if output_failed {
            engine.stop_all();
            self.handles.clear();
            self.reconciler = SceneAudioReconciler::default();
            self.engine = None;
            self.initialization_failed = true;
            tracing::warn!("audio command channel closed; continuing without sound");
            return;
        }

        engine.update(dt, None, &[]);
    }

    fn ensure_engine_with(
        &mut self,
        wants_output: bool,
        create_engine: impl FnOnce() -> Result<engine_audio::AudioEngine, engine_audio::AudioError>,
    ) -> bool {
        if self.engine.is_some() {
            return true;
        }
        if !wants_output || self.initialization_failed {
            return false;
        }
        match create_engine() {
            Ok(engine) => {
                self.engine = Some(engine);
                true
            }
            Err(error) => {
                self.initialization_failed = true;
                tracing::warn!(%error, "audio output is unavailable; continuing without sound");
                false
            }
        }
    }

    fn reset_scene(&mut self) {
        let _ = self.reconciler.reset();
        if let Some(engine) = self.engine.as_mut() {
            engine.stop_all();
        }
        self.handles.clear();
        self.initialization_failed = false;
    }
}

#[cfg(all(feature = "subsystem-scripting-csharp", feature = "gameplay"))]
fn script_input_value_is_active(value: &engine_script::GameplayInputValue) -> bool {
    match value {
        engine_script::GameplayInputValue::Bool(value) => *value,
        engine_script::GameplayInputValue::Float(value) => value.abs() > 0.5,
        engine_script::GameplayInputValue::Vec2(value) => {
            value[0] * value[0] + value[1] * value[1] > 0.25
        }
    }
}

#[cfg(all(test, feature = "runtime-audio-output"))]
mod runtime_audio_reconcile_tests {
    use super::*;

    fn source(clip_asset: &str) -> RuntimeAudioSourceSnapshot {
        RuntimeAudioSourceSnapshot {
            entity_id: "speaker".into(),
            clip_asset: Some(clip_asset.into()),
            clip_loaded: true,
            playing: true,
            volume: 1.0,
            looping: false,
            emitter: None,
        }
    }

    #[test]
    fn reconcile_starts_updates_and_stops_a_scene_voice() {
        let mut reconciler = SceneAudioReconciler::default();
        let initial = source("audio.intro");
        assert_eq!(
            reconciler.reconcile(std::slice::from_ref(&initial), &BTreeSet::new()),
            vec![RuntimeAudioSourceAction::Start(initial.clone())]
        );

        let mut changed = initial.clone();
        changed.volume = 0.25;
        changed.looping = true;
        changed.emitter = Some(RuntimeAudioEmitterSnapshot {
            position: Vec3::new(2.0, 3.0, 4.0),
            max_distance: 18.0,
            rolloff_factor: 0.75,
        });
        assert_eq!(
            reconciler.reconcile(std::slice::from_ref(&changed), &BTreeSet::new()),
            vec![RuntimeAudioSourceAction::Update(changed.clone())]
        );

        let mut stopped = changed;
        stopped.playing = false;
        assert_eq!(
            reconciler.reconcile(&[stopped], &BTreeSet::new()),
            vec![RuntimeAudioSourceAction::Stop {
                entity_id: "speaker".into()
            }]
        );
    }

    #[test]
    fn reconcile_replaces_clips_transactionally() {
        let mut reconciler = SceneAudioReconciler::default();
        let first = source("audio.first");
        let _ = reconciler.reconcile(std::slice::from_ref(&first), &BTreeSet::new());

        let second = source("audio.second");
        assert_eq!(
            reconciler.reconcile(std::slice::from_ref(&second), &BTreeSet::new()),
            vec![
                RuntimeAudioSourceAction::Stop {
                    entity_id: "speaker".into()
                },
                RuntimeAudioSourceAction::Start(second),
            ]
        );
    }

    #[test]
    fn missing_asset_stops_a_voice_and_starts_when_the_asset_arrives() {
        let mut reconciler = SceneAudioReconciler::default();
        let mut desired = source("audio.delayed");
        desired.clip_loaded = false;
        assert!(reconciler
            .reconcile(std::slice::from_ref(&desired), &BTreeSet::new())
            .is_empty());

        desired.clip_loaded = true;
        assert_eq!(
            reconciler.reconcile(std::slice::from_ref(&desired), &BTreeSet::new()),
            vec![RuntimeAudioSourceAction::Start(desired.clone())]
        );

        desired.clip_loaded = false;
        assert_eq!(
            reconciler.reconcile(&[desired], &BTreeSet::new()),
            vec![RuntimeAudioSourceAction::Stop {
                entity_id: "speaker".into()
            }]
        );
    }

    #[test]
    fn completed_one_shot_does_not_restart_until_rearmed() {
        let mut reconciler = SceneAudioReconciler::default();
        let desired = source("audio.once");
        let _ = reconciler.reconcile(std::slice::from_ref(&desired), &BTreeSet::new());

        assert_eq!(
            reconciler.reconcile(
                std::slice::from_ref(&desired),
                &BTreeSet::from(["speaker".into()]),
            ),
            vec![RuntimeAudioSourceAction::Stop {
                entity_id: "speaker".into()
            }]
        );
        assert!(reconciler
            .reconcile(std::slice::from_ref(&desired), &BTreeSet::new())
            .is_empty());

        let mut released = desired.clone();
        released.playing = false;
        assert!(reconciler
            .reconcile(&[released], &BTreeSet::new())
            .is_empty());
        assert_eq!(
            reconciler.reconcile(std::slice::from_ref(&desired), &BTreeSet::new()),
            vec![RuntimeAudioSourceAction::Start(desired)]
        );
    }

    #[test]
    fn scene_reset_stops_active_voices_and_rearms_device_initialization() {
        let mut reconciler = SceneAudioReconciler::default();
        let desired = source("audio.scene");
        let _ = reconciler.reconcile(std::slice::from_ref(&desired), &BTreeSet::new());
        assert_eq!(
            reconciler.reset(),
            vec![RuntimeAudioSourceAction::Stop {
                entity_id: "speaker".into()
            }]
        );
        assert_eq!(
            reconciler.reconcile(std::slice::from_ref(&desired), &BTreeSet::new()),
            vec![RuntimeAudioSourceAction::Start(desired)]
        );

        #[cfg(feature = "runtime-audio-output")]
        {
            let mut output = RuntimeAudioOutput {
                initialization_failed: true,
                ..RuntimeAudioOutput::default()
            };
            output.reset_scene();
            assert!(!output.initialization_failed);
        }
    }

    #[cfg(feature = "runtime-audio-output")]
    #[test]
    fn audio_device_creation_is_lazy_and_a_failure_is_non_fatal_and_not_retried() {
        use std::cell::Cell;

        let attempts = Cell::new(0);
        let mut output = RuntimeAudioOutput::default();
        assert!(!output.ensure_engine_with(false, || {
            attempts.set(attempts.get() + 1);
            Err(engine_audio::AudioError::NoDevice)
        }));
        assert_eq!(attempts.get(), 0);

        assert!(!output.ensure_engine_with(true, || {
            attempts.set(attempts.get() + 1);
            Err(engine_audio::AudioError::NoDevice)
        }));
        assert_eq!(attempts.get(), 1);
        assert!(output.initialization_failed);

        assert!(!output.ensure_engine_with(true, || {
            panic!("a failed scene must not hammer the device every frame")
        }));
        output.reset_scene();
        assert!(!output.initialization_failed);
    }

    #[cfg(feature = "runtime-audio-output")]
    #[test]
    fn scene_audio_frame_joins_persistent_components_transforms_and_typed_clips() {
        let mut game_loop = GameLoop::new(EngineConfig::default());
        game_loop.load_scene(engine_scene::sample_scene()).unwrap();
        game_loop
            .runtime
            .with_world_mut(|world| {
                let speaker = world.entity_by_persistent_id("cube-01").unwrap();
                world.add_component(
                    speaker,
                    engine_scene::components::Transform {
                        translation: Vec3::new(4.0, 5.0, 6.0),
                        ..engine_scene::components::Transform::default()
                    },
                );
                world.add_component(
                    speaker,
                    engine_audio::AudioSourceComponent {
                        clip_asset: Some("audio.ambient".into()),
                        volume: 0.4,
                        looping: true,
                        spatial: true,
                        max_distance: 30.0,
                        rolloff_factor: 0.6,
                        playing: true,
                    },
                );

                let listener = world.entity_by_persistent_id("camera-main").unwrap();
                world.add_component(
                    listener,
                    engine_scene::components::Transform {
                        translation: Vec3::new(1.0, 2.0, 3.0),
                        ..engine_scene::components::Transform::default()
                    },
                );
                world.add_component(listener, engine_audio::AudioListenerComponent::new());
            })
            .unwrap();

        let clip_id = engine_serialize::AssetId::new("audio.ambient");
        game_loop.runtime.asset_registry_mut().insert_erased(
            clip_id.clone(),
            Vec::new(),
            Box::new(engine_audio::AudioClip::new(vec![0.0; 32], 48_000, 1)),
        );
        game_loop
            .runtime
            .loaded_extension_asset_ids
            .entry("audio_clip".into())
            .or_default()
            .insert(clip_id);

        let frame = game_loop.runtime_audio_frame();
        assert_eq!(frame.sources.len(), 1);
        assert_eq!(frame.sources[0].entity_id, "cube-01");
        assert!(frame.sources[0].clip_loaded);
        assert_eq!(frame.sources[0].volume, 0.4);
        assert!(frame.sources[0].looping);
        let emitter = frame.sources[0].emitter.as_ref().unwrap();
        assert_eq!(emitter.position, Vec3::new(4.0, 5.0, 6.0));
        assert_eq!(emitter.max_distance, 30.0);
        assert_eq!(emitter.rolloff_factor, 0.6);
        assert_eq!(
            frame.listener.as_ref().unwrap().position,
            Vec3::new(1.0, 2.0, 3.0)
        );
    }
}

#[cfg(all(test, feature = "gameplay"))]
mod character_scene_tests {
    use std::collections::BTreeMap;

    use engine_gameplay::{InputAction, InputValue, InputValueType};
    use engine_serialize::{SchemaVersion, Value};

    use super::*;

    #[test]
    fn scene_character_component_binds_and_uses_standard_movement_actions() {
        let mut scene = engine_scene::sample_scene();
        let target = scene
            .entities
            .iter_mut()
            .find(|entity| entity.persistent_id == "cube-01")
            .unwrap();
        target.components.insert(
            "engine.transform".into(),
            engine_scene::ComponentRecord {
                schema_version: SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: BTreeMap::from([
                    ("translation".into(), Value::Vec3([3.0, 0.0, 2.0])),
                    ("rotation".into(), Value::Quat([0.0, 0.0, 0.0, 1.0])),
                    ("scale".into(), Value::Vec3([1.0; 3])),
                ]),
            },
        );
        target.components.insert(
            "engine.character_controller".into(),
            engine_scene::ComponentRecord {
                schema_version: SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: BTreeMap::from([
                    ("position".into(), Value::Vec3([0.0; 3])),
                    ("gravity_scale".into(), Value::Float32(0.0)),
                    ("air_acceleration".into(), Value::Float32(10.0)),
                    ("state".into(), Value::Enum("Falling".into())),
                ]),
            },
        );

        let mut game_loop = GameLoop::new(EngineConfig::default());
        game_loop.load_scene(scene).unwrap();
        assert!(game_loop.character.is_some());
        assert_eq!(
            game_loop.runtime.with_world(|world| game_loop
                .character_entity
                .and_then(|entity| world.persistent_id(entity).map(str::to_string))),
            Some(Some("cube-01".into()))
        );

        let mut forward = InputAction::new("move_forward", InputValueType::Digital);
        forward.current_value = InputValue::Bool(true);
        game_loop.input_map.add_action(forward);
        game_loop.update(0.1);

        let (transform_position, component_position) = game_loop
            .runtime
            .with_world(|world| {
                let entity = world.entity_by_persistent_id("cube-01").unwrap();
                let transform = world
                    .get::<engine_scene::components::Transform>(entity)
                    .unwrap();
                let controller = world.get::<CharacterController>(entity).unwrap();
                (transform.translation, controller.position())
            })
            .unwrap();
        assert!(transform_position.z < 2.0, "{transform_position:?}");
        assert_eq!(component_position, transform_position);
    }

    #[cfg(feature = "runtime-subsystems")]
    #[test]
    fn game_loop_extracts_scene_canvases_in_stable_order_for_rendering() {
        let mut game_loop = GameLoop::new(EngineConfig::default());
        game_loop.load_scene(engine_scene::sample_scene()).unwrap();
        game_loop
            .runtime
            .with_world_mut(|world| {
                for (entity_id, color) in [
                    ("cube-01", engine_ui::Color::new(255, 0, 0, 255)),
                    ("camera-main", engine_ui::Color::new(0, 255, 0, 255)),
                ] {
                    let entity = world.entity_by_persistent_id(entity_id).unwrap();
                    let mut canvas = engine_ui::Canvas::new(320.0, 180.0);
                    canvas.add_element(engine_ui::UiElement::new(
                        engine_ui::UiElementKind::Panel { color },
                        engine_ui::Layout::FILL,
                    ));
                    world.add_component(entity, canvas);
                }
            })
            .unwrap();

        let batches = game_loop.runtime_ui_batches();

        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].canvas_id, "camera-main");
        assert_eq!(batches[1].canvas_id, "cube-01");
        assert!(batches.iter().all(|batch| batch.vertices.len() == 4));
        assert!(batches.iter().all(|batch| batch.indices.len() == 6));
        assert!(batches
            .iter()
            .all(|batch| batch.clip_rect.max == [320.0, 180.0]));
    }

    #[cfg(feature = "runtime-subsystems")]
    #[test]
    fn game_loop_advances_loaded_animation_assets_and_keeps_only_the_latest_pose() {
        use engine_animation::{
            AnimationClip, AnimationPlayer, Joint, JointTransform, Skeleton, SkeletonComponent,
        };
        use engine_asset::cook::{registered_asset_type_id, AssetType};

        let cooked = std::env::temp_dir().join(format!(
            "engine_core_game_loop_animation_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&cooked);
        std::fs::create_dir_all(&cooked).unwrap();

        let mut game_loop = GameLoop::new(EngineConfig::default());
        let write_extension = |id: &str, kind: AssetType, source: &[u8]| {
            let type_id = registered_asset_type_id(&kind).unwrap();
            let extension = game_loop
                .runtime
                .asset_type_registry()
                .get(type_id)
                .unwrap();
            let mut payload = Vec::new();
            extension.cooker.unwrap()(source, &mut payload).unwrap();
            engine_asset::cook::write_cooked_artifact(
                &cooked.join(format!("{id}.cooked")),
                kind.kind_code(),
                &payload,
                SchemaVersion::new(0, 1, 0),
            )
            .unwrap();
        };
        let skeleton = Skeleton {
            joints: vec![Joint {
                name: "root".into(),
                parent_index: None,
                local_transform: JointTransform::IDENTITY,
            }],
            inverse_bind_matrices: vec![[
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ]],
        };
        write_extension(
            "hero.skeleton",
            AssetType::Skeleton,
            &bincode::serialize(&skeleton).unwrap(),
        );
        let clip = AnimationClip {
            name: "idle".into(),
            duration: 1.0,
            channels: vec![],
            joint_indices: vec![],
        };
        write_extension(
            "idle.animation",
            AssetType::Animation,
            &bincode::serialize(&clip).unwrap(),
        );
        game_loop.runtime.load_cooked_assets(&cooked).unwrap();

        game_loop.load_scene(engine_scene::sample_scene()).unwrap();
        game_loop
            .runtime
            .with_world_mut(|world| {
                let entity = world.entity_by_persistent_id("cube-01").unwrap();
                world.add_component(entity, engine_scene::components::Transform::default());
                world.add_component(entity, SkeletonComponent::new("hero.skeleton"));
                world.add_component(entity, AnimationPlayer::with_clip("idle.animation"));
            })
            .unwrap();

        game_loop.update(0.25);
        assert_eq!(
            game_loop
                .runtime
                .animation_extension_handles()
                .skinned_extract
                .pending_count(),
            1
        );
        assert_eq!(
            game_loop.runtime.with_world(|world| {
                let entity = world.entity_by_persistent_id("cube-01").unwrap();
                world.get::<AnimationPlayer>(entity).unwrap().current_time
            }),
            Some(0.25)
        );

        game_loop.update(0.25);
        assert_eq!(
            game_loop
                .runtime
                .animation_extension_handles()
                .skinned_extract
                .pending_count(),
            1,
            "fixed updates before a render must replace rather than accumulate poses"
        );
        assert_eq!(
            game_loop.runtime.with_world(|world| {
                let entity = world.entity_by_persistent_id("cube-01").unwrap();
                world.get::<AnimationPlayer>(entity).unwrap().current_time
            }),
            Some(0.5)
        );

        let _ = std::fs::remove_dir_all(cooked);
    }

    #[test]
    fn game_loop_advances_non_primary_character_controllers() {
        let mut scene = engine_scene::sample_scene();
        for entity in &mut scene.entities {
            entity.components.insert(
                "engine.transform".into(),
                engine_scene::ComponentRecord {
                    schema_version: SchemaVersion::new(0, 1, 0),
                    enabled: true,
                    fields: BTreeMap::new(),
                },
            );
            entity.components.insert(
                "engine.character_controller".into(),
                engine_scene::ComponentRecord {
                    schema_version: SchemaVersion::new(0, 1, 0),
                    enabled: true,
                    fields: BTreeMap::from([("gravity_scale".into(), Value::Float32(0.0))]),
                },
            );
        }
        let mut game_loop = GameLoop::new(EngineConfig::default());
        game_loop.load_scene(scene).unwrap();
        let primary = game_loop.character_entity.unwrap();
        let secondary = game_loop
            .runtime
            .with_world(|world| {
                world
                    .query::<CharacterController>()
                    .map(|(entity, _)| entity)
                    .find(|entity| *entity != primary)
                    .unwrap()
            })
            .unwrap();
        game_loop
            .runtime
            .with_world_mut(|world| {
                world
                    .get_mut::<CharacterController>(secondary)
                    .unwrap()
                    .push_command(engine_character::CharacterCommand::move_towards(Vec3::X));
            })
            .unwrap();

        game_loop.update(0.1);

        assert!(
            game_loop
                .runtime
                .with_world(|world| world
                    .get::<engine_scene::components::Transform>(secondary)
                    .unwrap()
                    .translation
                    .x)
                .unwrap()
                > 0.0
        );
    }

    #[cfg(feature = "runtime-subsystems")]
    #[test]
    fn loaded_navmesh_drives_a_scene_character_through_the_standard_game_loop() {
        use engine_asset::cook::{registered_asset_type_id, AssetType};

        let cooked = std::env::temp_dir().join(format!(
            "engine_core_game_loop_navigation_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&cooked);
        std::fs::create_dir_all(&cooked).unwrap();

        let mut game_loop = GameLoop::new(EngineConfig::default());
        let mut navmesh = engine_nav::NavMesh::new();
        let a = navmesh.add_vertex(Vec3::new(-10.0, 0.0, -10.0));
        let b = navmesh.add_vertex(Vec3::new(10.0, 0.0, -10.0));
        let c = navmesh.add_vertex(Vec3::new(0.0, 0.0, 10.0));
        navmesh.add_polygon(&[a, b, c], 1.0);
        navmesh.rebuild_bvh();
        let extension = game_loop
            .runtime
            .asset_type_registry()
            .get(registered_asset_type_id(&AssetType::NavMesh).unwrap())
            .unwrap();
        let mut payload = Vec::new();
        extension.cooker.unwrap()(&bincode::serialize(&navmesh).unwrap(), &mut payload).unwrap();
        engine_asset::cook::write_cooked_artifact(
            &cooked.join("level.navmesh.cooked"),
            AssetType::NavMesh.kind_code(),
            &payload,
            SchemaVersion::new(0, 1, 0),
        )
        .unwrap();
        game_loop.runtime.load_cooked_assets(&cooked).unwrap();

        let mut scene = engine_scene::sample_scene();
        let cube = scene
            .entities
            .iter_mut()
            .find(|entity| entity.persistent_id == "cube-01")
            .unwrap();
        cube.components.insert(
            "engine.transform".into(),
            engine_scene::ComponentRecord {
                schema_version: SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: BTreeMap::new(),
            },
        );
        cube.components.insert(
            "engine.character_controller".into(),
            engine_scene::ComponentRecord {
                schema_version: SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: BTreeMap::from([("gravity_scale".into(), Value::Float32(0.0))]),
            },
        );
        game_loop.load_scene(scene).unwrap();
        game_loop
            .runtime
            .with_world_mut(|world| {
                let entity = world.entity_by_persistent_id("cube-01").unwrap();
                let mut agent = engine_nav::AiAgent::new();
                agent.navmesh_ref = Some("level.navmesh".into());
                agent.target = Some(Vec3::new(0.0, 0.0, 5.0));
                agent.speed = 2.0;
                world.add_component(entity, agent);
            })
            .unwrap();

        game_loop.update(0.1);

        let translation = game_loop
            .runtime
            .with_world(|world| {
                let entity = world.entity_by_persistent_id("cube-01").unwrap();
                world
                    .get::<engine_scene::components::Transform>(entity)
                    .unwrap()
                    .translation
            })
            .unwrap();
        assert!(
            translation.x * translation.x + translation.z * translation.z > 0.0,
            "navigation intent did not move the character: {translation:?}"
        );
        let _ = std::fs::remove_dir_all(cooked);
    }

    #[test]
    fn loading_a_scene_without_a_character_clears_previous_binding() {
        let mut scene = engine_scene::sample_scene();
        scene.entities[0].components.insert(
            "engine.character_controller".into(),
            engine_scene::ComponentRecord {
                schema_version: SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: BTreeMap::new(),
            },
        );
        let mut game_loop = GameLoop::new(EngineConfig::default());
        game_loop.load_scene(scene).unwrap();
        assert!(game_loop.character.is_some());

        game_loop.load_scene(engine_scene::sample_scene()).unwrap();
        assert!(game_loop.character.is_none());
        assert!(game_loop.character_entity.is_none());
    }

    #[test]
    fn failed_scene_load_keeps_previous_gameplay_bindings() {
        let mut scene = engine_scene::sample_scene();
        scene.entities[0].components.insert(
            "engine.character_controller".into(),
            engine_scene::ComponentRecord {
                schema_version: SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: BTreeMap::new(),
            },
        );
        let mut game_loop = GameLoop::new(EngineConfig::default());
        game_loop.load_scene(scene).unwrap();
        let previous_entity = game_loop.character_entity;
        let previous_position = game_loop.character.as_ref().unwrap().position();
        assert!(game_loop.physics.is_some());

        let mut invalid = engine_scene::sample_scene();
        invalid.entities.push(invalid.entities[0].clone());
        assert!(game_loop.load_scene(invalid).is_err());

        assert_eq!(game_loop.character_entity, previous_entity);
        assert_eq!(
            game_loop.character.as_ref().unwrap().position(),
            previous_position
        );
        assert!(game_loop.physics.is_some());
        assert_eq!(
            game_loop
                .runtime
                .with_world(|world| world.entity_by_persistent_id("camera-main").is_some()),
            Some(true)
        );
    }

    #[test]
    fn physics_events_are_exposed_for_one_frame_and_drained_from_the_backend() {
        use engine_physics::{BodyType, Collider, RigidBody};
        use engine_scene::components::Transform;

        let mut game_loop = GameLoop::new(EngineConfig::default());
        game_loop.load_scene(engine_scene::sample_scene()).unwrap();
        game_loop.runtime.with_world_mut(|world| {
            let dynamic = world.entity_by_persistent_id("cube-01").unwrap();
            let fixed = world.entity_by_persistent_id("camera-main").unwrap();
            world.add_component(dynamic, Transform::default());
            world.add_component(fixed, Transform::default());
            world.add_component(dynamic, RigidBody::default());
            world.add_component(dynamic, Collider::default());
            world.add_component(
                fixed,
                RigidBody {
                    body_type: BodyType::Static,
                    ..RigidBody::default()
                },
            );
            world.add_component(fixed, Collider::default());
        });
        game_loop.init_physics();

        game_loop.update(1.0 / 30.0);

        assert!(!game_loop.physics_events().is_empty());
        assert!(game_loop
            .physics
            .as_ref()
            .unwrap()
            .pending_events()
            .is_empty());
        assert!(game_loop
            .physics
            .as_ref()
            .unwrap()
            .pending_triggers()
            .is_empty());
        let events = game_loop.take_physics_events();
        assert!(!events.is_empty());
        assert!(game_loop.physics_events().is_empty());

        game_loop.update(0.0);
        assert!(game_loop.physics_events().is_empty());
    }
}

#[cfg(all(test, feature = "gameplay", feature = "subsystem-scripting-csharp"))]
mod gameplay_script_bridge_tests {
    use std::collections::BTreeMap;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    use engine_gameplay::{InputAction, InputValue, InputValueType};
    use engine_script::{
        GameplayCommand, GameplayContext, ScriptError, ScriptHandle, ScriptHost, ScriptInstance,
        ScriptTransform, ScriptValue,
    };
    use engine_serialize::{SchemaVersion, Value};

    use super::*;

    struct InputDrivenInstance {
        context: Option<GameplayContext>,
        commands: Vec<GameplayCommand>,
        destroy_count: Arc<AtomicUsize>,
    }

    impl ScriptInstance for InputDrivenInstance {
        fn call(
            &mut self,
            function: &str,
            _args: &[ScriptValue],
        ) -> Result<ScriptValue, ScriptError> {
            if function == engine_script::ON_DESTROY {
                self.destroy_count.fetch_add(1, Ordering::SeqCst);
            } else if function == engine_script::ON_UPDATE {
                let context = self
                    .context
                    .as_ref()
                    .expect("gameplay context before update");
                if context.input_actions.get("jump")
                    == Some(&engine_script::GameplayInputValue::Bool(true))
                {
                    let mut transform = context.transform.clone().expect("owner Transform");
                    transform.translation[0] += 2.0;
                    self.commands
                        .push(GameplayCommand::SetTransform { transform });
                }
                if context.input_actions.get("load_level")
                    == Some(&engine_script::GameplayInputValue::Bool(true))
                {
                    self.commands.push(GameplayCommand::LoadScene {
                        scene_id: "level_two".into(),
                    });
                }
                if context.input_actions.get("load_other")
                    == Some(&engine_script::GameplayInputValue::Bool(true))
                {
                    self.commands.push(GameplayCommand::LoadScene {
                        scene_id: "level_three".into(),
                    });
                }
                if context.entity_id == "cube-01"
                    && context.input_actions.get("move_camera")
                        == Some(&engine_script::GameplayInputValue::Bool(true))
                {
                    let mut transform = context.entities["camera-main"]
                        .transform
                        .clone()
                        .expect("camera Transform snapshot");
                    transform.translation = [7.0, 8.0, 9.0];
                    self.commands.push(GameplayCommand::SetEntityTransform {
                        entity_id: "camera-main".into(),
                        transform,
                    });
                }
                if context.entity_id == "cube-01"
                    && context.input_actions.get("destroy_camera")
                        == Some(&engine_script::GameplayInputValue::Bool(true))
                {
                    self.commands.push(GameplayCommand::DestroyEntity {
                        entity_id: "camera-main".into(),
                    });
                }
            }
            Ok(ScriptValue::Null)
        }

        fn set_field(&mut self, _name: &str, _value: ScriptValue) -> Result<(), ScriptError> {
            Ok(())
        }

        fn get_field(&self, _name: &str) -> Option<ScriptValue> {
            None
        }

        fn set_gameplay_context(&mut self, context: &GameplayContext) -> Result<(), ScriptError> {
            self.context = Some(context.clone());
            Ok(())
        }

        fn drain_gameplay_commands(&mut self) -> Result<Vec<GameplayCommand>, ScriptError> {
            Ok(std::mem::take(&mut self.commands))
        }
    }

    struct InputDrivenHost {
        destroy_count: Arc<AtomicUsize>,
    }

    impl InputDrivenHost {
        fn new(destroy_count: Arc<AtomicUsize>) -> Self {
            Self { destroy_count }
        }
    }

    impl ScriptHost for InputDrivenHost {
        fn name(&self) -> &str {
            "bridge-test"
        }

        fn load_assembly(
            &mut self,
            id: &str,
            _assembly_data: &[u8],
        ) -> Result<ScriptHandle, ScriptError> {
            Ok(ScriptHandle::new(id))
        }

        fn instantiate(
            &mut self,
            _handle: &ScriptHandle,
            _class_name: &str,
        ) -> Result<Box<dyn ScriptInstance>, ScriptError> {
            Ok(Box::new(InputDrivenInstance {
                context: None,
                commands: Vec::new(),
                destroy_count: Arc::clone(&self.destroy_count),
            }))
        }

        fn unload(&mut self, _handle: &ScriptHandle) -> Result<(), ScriptError> {
            Ok(())
        }
    }

    struct ContextRecordingInstance {
        contexts: Arc<std::sync::Mutex<Vec<GameplayContext>>>,
    }

    impl ScriptInstance for ContextRecordingInstance {
        fn call(
            &mut self,
            _function: &str,
            _args: &[ScriptValue],
        ) -> Result<ScriptValue, ScriptError> {
            Ok(ScriptValue::Null)
        }

        fn set_field(&mut self, _name: &str, _value: ScriptValue) -> Result<(), ScriptError> {
            Ok(())
        }

        fn get_field(&self, _name: &str) -> Option<ScriptValue> {
            None
        }

        fn set_gameplay_context(&mut self, context: &GameplayContext) -> Result<(), ScriptError> {
            self.contexts.lock().unwrap().push(context.clone());
            Ok(())
        }
    }

    struct ContextRecordingHost {
        contexts: Arc<std::sync::Mutex<Vec<GameplayContext>>>,
    }

    impl ScriptHost for ContextRecordingHost {
        fn name(&self) -> &str {
            "context-recording"
        }

        fn load_assembly(
            &mut self,
            id: &str,
            _assembly_data: &[u8],
        ) -> Result<ScriptHandle, ScriptError> {
            Ok(ScriptHandle::new(id))
        }

        fn instantiate(
            &mut self,
            _handle: &ScriptHandle,
            _class_name: &str,
        ) -> Result<Box<dyn ScriptInstance>, ScriptError> {
            Ok(Box::new(ContextRecordingInstance {
                contexts: Arc::clone(&self.contexts),
            }))
        }

        fn unload(&mut self, _handle: &ScriptHandle) -> Result<(), ScriptError> {
            Ok(())
        }
    }

    #[test]
    fn resolved_true_input_reaches_script_and_applies_owner_transform_command() {
        let mut game_loop = GameLoop::new(EngineConfig::default());
        let destroy_count = Arc::new(AtomicUsize::new(0));
        game_loop
            .runtime
            .register_script_host(Box::new(InputDrivenHost::new(destroy_count)));
        game_loop.runtime.set_script_host_name("bridge-test");
        game_loop
            .runtime
            .load_script_assembly("game", "bridge-test", b"test")
            .unwrap();

        let mut scene = engine_scene::sample_scene();
        let target = scene
            .entities
            .iter_mut()
            .find(|entity| entity.persistent_id == "cube-01")
            .unwrap();
        target.components.insert(
            "engine.transform".into(),
            engine_scene::ComponentRecord {
                schema_version: SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: BTreeMap::from([
                    ("translation".into(), Value::Vec3([0.0; 3])),
                    ("rotation".into(), Value::Quat([0.0, 0.0, 0.0, 1.0])),
                    ("scale".into(), Value::Vec3([1.0; 3])),
                ]),
            },
        );
        target.components.insert(
            "engine.script".into(),
            engine_scene::ComponentRecord {
                schema_version: SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: BTreeMap::from([
                    ("assembly_id".into(), Value::Str("game".into())),
                    ("class_name".into(), Value::Str("Player".into())),
                ]),
            },
        );
        game_loop.load_scene(scene).unwrap();

        let mut jump = InputAction::new("jump", InputValueType::Digital);
        jump.current_value = InputValue::Bool(true);
        game_loop.input_map.add_action(jump);
        game_loop.update(1.0 / 60.0);

        let translation = game_loop
            .runtime
            .with_world(|world| {
                world
                    .query_all::<engine_scene::components::Transform>()
                    .find_map(|(entity, transform)| {
                        (world.persistent_id(entity) == Some("cube-01"))
                            .then_some(transform.translation)
                    })
                    .unwrap()
            })
            .unwrap();
        assert_eq!(translation, glam::Vec3::new(2.0, 0.0, 0.0));
        assert!(game_loop
            .runtime
            .diagnostics_collector()
            .script_diagnostics
            .is_empty());
    }

    #[test]
    fn entity_snapshot_can_drive_an_explicit_target_transform_command() {
        let destroy_count = Arc::new(AtomicUsize::new(0));
        let mut game_loop = GameLoop::new(EngineConfig::default());
        game_loop
            .runtime
            .register_script_host(Box::new(InputDrivenHost::new(destroy_count)));
        game_loop.runtime.set_script_host_name("bridge-test");
        game_loop
            .runtime
            .load_script_assembly("game", "bridge-test", b"test")
            .unwrap();

        let mut scene = engine_scene::sample_scene();
        scene
            .entities
            .iter_mut()
            .find(|entity| entity.persistent_id == "camera-main")
            .unwrap()
            .components
            .insert(
                "engine.transform".into(),
                engine_scene::ComponentRecord {
                    schema_version: SchemaVersion::new(0, 1, 0),
                    enabled: true,
                    fields: BTreeMap::from([
                        ("translation".into(), Value::Vec3([0.0; 3])),
                        ("rotation".into(), Value::Quat([0.0, 0.0, 0.0, 1.0])),
                        ("scale".into(), Value::Vec3([1.0; 3])),
                    ]),
                },
            );
        scene
            .entities
            .iter_mut()
            .find(|entity| entity.persistent_id == "cube-01")
            .unwrap()
            .components
            .insert(
                "engine.script".into(),
                engine_scene::ComponentRecord {
                    schema_version: SchemaVersion::new(0, 1, 0),
                    enabled: true,
                    fields: BTreeMap::from([
                        ("assembly_id".into(), Value::Str("game".into())),
                        ("class_name".into(), Value::Str("Player".into())),
                    ]),
                },
            );
        game_loop.load_scene(scene).unwrap();

        let mut move_camera = InputAction::new("move_camera", InputValueType::Digital);
        move_camera.current_value = InputValue::Bool(true);
        game_loop.input_map.add_action(move_camera);
        game_loop.update(1.0 / 60.0);

        let camera_translation = game_loop
            .runtime
            .with_world(|world| {
                let camera = world.entity_by_persistent_id("camera-main").unwrap();
                world
                    .get::<engine_scene::components::Transform>(camera)
                    .unwrap()
                    .translation
            })
            .unwrap();
        assert_eq!(camera_translation, glam::Vec3::new(7.0, 8.0, 9.0));
        assert!(game_loop
            .runtime
            .diagnostics_collector()
            .script_diagnostics
            .is_empty());
    }

    #[test]
    fn explicit_destroy_runs_target_ondestroy_and_detaches_only_that_entity() {
        let destroy_count = Arc::new(AtomicUsize::new(0));
        let mut game_loop = GameLoop::new(EngineConfig::default());
        game_loop
            .runtime
            .register_script_host(Box::new(InputDrivenHost::new(Arc::clone(&destroy_count))));
        game_loop.runtime.set_script_host_name("bridge-test");
        game_loop
            .runtime
            .load_script_assembly("game", "bridge-test", b"test")
            .unwrap();

        let mut scene = engine_scene::sample_scene();
        for entity in &mut scene.entities {
            entity.components.insert(
                "engine.script".into(),
                engine_scene::ComponentRecord {
                    schema_version: SchemaVersion::new(0, 1, 0),
                    enabled: true,
                    fields: BTreeMap::from([
                        ("assembly_id".into(), Value::Str("game".into())),
                        ("class_name".into(), Value::Str("Actor".into())),
                    ]),
                },
            );
        }
        game_loop.load_scene(scene).unwrap();
        assert_eq!(
            game_loop.runtime.script_engine().managers()[0].instance_count(),
            2
        );

        let mut destroy_camera = InputAction::new("destroy_camera", InputValueType::Digital);
        destroy_camera.current_value = InputValue::Bool(true);
        game_loop.input_map.add_action(destroy_camera);
        game_loop.update(1.0 / 60.0);

        game_loop
            .runtime
            .with_world(|world| {
                assert!(world.entity_by_persistent_id("camera-main").is_none());
                assert!(world.entity_by_persistent_id("cube-01").is_some());
            })
            .unwrap();
        assert_eq!(destroy_count.load(Ordering::SeqCst), 1);
        assert_eq!(
            game_loop.runtime.script_engine().managers()[0].instance_count(),
            1
        );
        assert!(game_loop
            .runtime
            .diagnostics_collector()
            .script_diagnostics
            .is_empty());
    }

    #[test]
    fn invalid_script_transform_reports_a_specific_diagnostic() {
        assert_eq!(
            super::super::validate_script_transform(&ScriptTransform {
                translation: [f32::NAN, 0.0, 0.0],
                rotation: [0.0, 0.0, 0.0, 1.0],
                scale: [1.0, 1.0, 1.0],
            }),
            Err("translation, rotation, and scale must contain only finite values")
        );
    }

    #[test]
    fn physics_contacts_are_mapped_symmetrically_to_persistent_script_entities() {
        use engine_physics::{CollisionEvent, CollisionEventKind};
        use engine_script::{GameplayPhysicsEvent, GameplayPhysicsEventKind};

        let mut game_loop = GameLoop::new(EngineConfig::default());
        game_loop.load_scene(engine_scene::sample_scene()).unwrap();
        let (cube, camera) = game_loop
            .runtime
            .with_world(|world| {
                (
                    world.entity_by_persistent_id("cube-01").unwrap(),
                    world.entity_by_persistent_id("camera-main").unwrap(),
                )
            })
            .unwrap();
        game_loop.physics_events.collisions.push(CollisionEvent {
            kind: CollisionEventKind::ContactStarted,
            entity_a: cube,
            entity_b: camera,
        });

        let events = game_loop.resolved_script_physics_events();

        assert_eq!(
            events.get("cube-01"),
            Some(&vec![GameplayPhysicsEvent {
                kind: GameplayPhysicsEventKind::CollisionEntered,
                other_entity_id: "camera-main".into(),
            }])
        );
        assert_eq!(
            events.get("camera-main"),
            Some(&vec![GameplayPhysicsEvent {
                kind: GameplayPhysicsEventKind::CollisionEntered,
                other_entity_id: "cube-01".into(),
            }])
        );
    }

    #[test]
    fn entity_relative_physics_events_reach_the_script_gameplay_context() {
        use engine_script::{GameplayPhysicsEvent, GameplayPhysicsEventKind};

        let contexts = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut game_loop = GameLoop::new(EngineConfig::default());
        game_loop
            .runtime
            .register_script_host(Box::new(ContextRecordingHost {
                contexts: Arc::clone(&contexts),
            }));
        game_loop.runtime.set_script_host_name("context-recording");
        game_loop
            .runtime
            .load_script_assembly("game", "context-recording", b"test")
            .unwrap();

        let mut scene = engine_scene::sample_scene();
        let target = scene
            .entities
            .iter_mut()
            .find(|entity| entity.persistent_id == "cube-01")
            .unwrap();
        target.components.insert(
            "engine.transform".into(),
            engine_scene::ComponentRecord {
                schema_version: SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: BTreeMap::new(),
            },
        );
        target.components.insert(
            "engine.script".into(),
            engine_scene::ComponentRecord {
                schema_version: SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: BTreeMap::from([
                    ("assembly_id".into(), Value::Str("game".into())),
                    ("class_name".into(), Value::Str("Player".into())),
                ]),
            },
        );
        game_loop.load_scene(scene).unwrap();

        let expected = GameplayPhysicsEvent {
            kind: GameplayPhysicsEventKind::TriggerEntered,
            other_entity_id: "camera-main".into(),
        };
        game_loop.runtime.tick_scripts_with_input_and_physics(
            1.0 / 60.0,
            &BTreeMap::new(),
            &BTreeMap::from([("cube-01".into(), vec![expected.clone()])]),
        );

        let contexts = contexts.lock().unwrap();
        let latest = contexts.last().expect("script gameplay context");
        assert_eq!(latest.entity_id, "cube-01");
        assert_eq!(latest.physics_events, vec![expected]);
    }

    #[test]
    fn script_input_transitions_fire_once_for_press_and_release_edges() {
        let contexts = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut game_loop = GameLoop::new(EngineConfig::default());
        game_loop
            .runtime
            .register_script_host(Box::new(ContextRecordingHost {
                contexts: Arc::clone(&contexts),
            }));
        game_loop.runtime.set_script_host_name("context-recording");
        game_loop
            .runtime
            .load_script_assembly("game", "context-recording", b"test")
            .unwrap();

        let mut scene = engine_scene::sample_scene();
        scene
            .entities
            .iter_mut()
            .find(|entity| entity.persistent_id == "cube-01")
            .unwrap()
            .components
            .insert(
                "engine.script".into(),
                engine_scene::ComponentRecord {
                    schema_version: SchemaVersion::new(0, 1, 0),
                    enabled: true,
                    fields: BTreeMap::from([
                        ("assembly_id".into(), Value::Str("game".into())),
                        ("class_name".into(), Value::Str("Player".into())),
                    ]),
                },
            );
        game_loop.load_scene(scene).unwrap();

        let mut jump = InputAction::new("jump", InputValueType::Digital);
        jump.current_value = InputValue::Bool(true);
        game_loop.input_map.add_action(jump);
        game_loop.update(1.0 / 60.0);
        let pressed = contexts.lock().unwrap().last().unwrap().clone();
        assert!(pressed.input_transitions.was_pressed("jump"));
        assert!(!pressed.input_transitions.was_released("jump"));

        game_loop.update(1.0 / 60.0);
        let held = contexts.lock().unwrap().last().unwrap().clone();
        assert!(!held.input_transitions.was_pressed("jump"));
        assert!(!held.input_transitions.was_released("jump"));

        game_loop
            .input_map
            .action_mut("jump")
            .unwrap()
            .current_value = InputValue::Bool(false);
        game_loop.update(1.0 / 60.0);
        let released = contexts.lock().unwrap().last().unwrap().clone();
        assert!(!released.input_transitions.was_pressed("jump"));
        assert!(released.input_transitions.was_released("jump"));

        game_loop.update(1.0 / 60.0);
        let idle = contexts.lock().unwrap().last().unwrap().clone();
        assert_eq!(
            idle.input_transitions,
            engine_script::GameplayInputTransitions::default()
        );
    }

    #[test]
    fn scalar_and_vector_script_actions_use_the_documented_edge_threshold() {
        use engine_script::GameplayInputValue;

        assert!(!script_input_value_is_active(&GameplayInputValue::Float(
            0.5
        )));
        assert!(script_input_value_is_active(&GameplayInputValue::Float(
            0.51
        )));
        assert!(script_input_value_is_active(&GameplayInputValue::Float(
            -0.51
        )));
        assert!(!script_input_value_is_active(&GameplayInputValue::Vec2([
            0.29, 0.4
        ])));
        assert!(script_input_value_is_active(&GameplayInputValue::Vec2([
            0.31, 0.4
        ])));
    }

    #[test]
    fn script_scene_command_is_deferred_until_the_host_frame_boundary() {
        let mut game_loop = GameLoop::new(EngineConfig::default());
        let destroy_count = Arc::new(AtomicUsize::new(0));
        game_loop
            .runtime
            .register_script_host(Box::new(InputDrivenHost::new(destroy_count)));
        game_loop.runtime.set_script_host_name("bridge-test");
        game_loop
            .runtime
            .load_script_assembly("game", "bridge-test", b"test")
            .unwrap();

        let mut scene = engine_scene::sample_scene();
        let target = scene
            .entities
            .iter_mut()
            .find(|entity| entity.persistent_id == "cube-01")
            .unwrap();
        target.components.insert(
            "engine.transform".into(),
            engine_scene::ComponentRecord {
                schema_version: SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: BTreeMap::new(),
            },
        );
        target.components.insert(
            "engine.script".into(),
            engine_scene::ComponentRecord {
                schema_version: SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: BTreeMap::from([
                    ("assembly_id".into(), Value::Str("game".into())),
                    ("class_name".into(), Value::Str("Player".into())),
                ]),
            },
        );
        game_loop.load_scene(scene).unwrap();

        let mut load = InputAction::new("load_level", InputValueType::Digital);
        load.current_value = InputValue::Bool(true);
        game_loop.input_map.add_action(load);
        game_loop.update(1.0 / 60.0);

        game_loop
            .input_map
            .action_mut("load_level")
            .unwrap()
            .current_value = InputValue::Bool(false);
        let mut load_other = InputAction::new("load_other", InputValueType::Digital);
        load_other.current_value = InputValue::Bool(true);
        game_loop.input_map.add_action(load_other);
        game_loop.update(1.0 / 60.0);

        assert_eq!(
            game_loop.runtime.take_pending_scene_request(),
            Some(crate::SceneLoadRequest {
                scene_id: "level_two".into(),
                requested_by: "cube-01".into(),
            })
        );
        assert_eq!(game_loop.runtime.take_pending_scene_request(), None);
        assert!(game_loop
            .runtime
            .diagnostics_collector()
            .script_diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SCRIPT_SCENE_REQUEST_CONFLICT"));
        assert_eq!(
            game_loop
                .runtime
                .scene_ref()
                .map(|scene| scene.scene_id.as_str()),
            Some("scene-gate04-valid")
        );
    }

    struct PhysicsQueryInstance {
        contexts: Arc<std::sync::Mutex<Vec<GameplayContext>>>,
        commands: Vec<GameplayCommand>,
        issued: bool,
    }

    impl ScriptInstance for PhysicsQueryInstance {
        fn call(
            &mut self,
            function: &str,
            _args: &[ScriptValue],
        ) -> Result<ScriptValue, ScriptError> {
            if function == engine_script::ON_UPDATE && !self.issued {
                self.issued = true;
                let raycast = |query_id, direction| GameplayCommand::PhysicsQuery {
                    query: engine_script::GameplayPhysicsQuery::Raycast {
                        query_id,
                        origin: [0.0, 5.0, 0.0],
                        direction,
                        max_distance: 10.0,
                    },
                };
                // Downward ray hits the owning cube's top face at y = 0.5;
                // the upward ray misses every collider.
                self.commands.push(raycast(11, [0.0, -1.0, 0.0]));
                self.commands.push(raycast(12, [0.0, 1.0, 0.0]));
                self.commands.push(GameplayCommand::PhysicsQuery {
                    query: engine_script::GameplayPhysicsQuery::OverlapSphere {
                        query_id: 13,
                        center: [0.0, 0.0, 0.0],
                        radius: 1.0,
                    },
                });
            }
            Ok(ScriptValue::Null)
        }

        fn set_field(&mut self, _name: &str, _value: ScriptValue) -> Result<(), ScriptError> {
            Ok(())
        }

        fn get_field(&self, _name: &str) -> Option<ScriptValue> {
            None
        }

        fn set_gameplay_context(&mut self, context: &GameplayContext) -> Result<(), ScriptError> {
            self.contexts.lock().unwrap().push(context.clone());
            Ok(())
        }

        fn drain_gameplay_commands(&mut self) -> Result<Vec<GameplayCommand>, ScriptError> {
            Ok(std::mem::take(&mut self.commands))
        }
    }

    struct PhysicsQueryHost {
        contexts: Arc<std::sync::Mutex<Vec<GameplayContext>>>,
    }

    impl ScriptHost for PhysicsQueryHost {
        fn name(&self) -> &str {
            "physics-query-test"
        }

        fn load_assembly(
            &mut self,
            id: &str,
            _assembly_data: &[u8],
        ) -> Result<ScriptHandle, ScriptError> {
            Ok(ScriptHandle::new(id))
        }

        fn instantiate(
            &mut self,
            _handle: &ScriptHandle,
            _class_name: &str,
        ) -> Result<Box<dyn ScriptInstance>, ScriptError> {
            Ok(Box::new(PhysicsQueryInstance {
                contexts: Arc::clone(&self.contexts),
                commands: Vec::new(),
                issued: false,
            }))
        }

        fn unload(&mut self, _handle: &ScriptHandle) -> Result<(), ScriptError> {
            Ok(())
        }
    }

    fn physics_query_game_loop(contexts: &Arc<std::sync::Mutex<Vec<GameplayContext>>>) -> GameLoop {
        let mut game_loop = GameLoop::new(EngineConfig::default());
        game_loop
            .runtime
            .register_script_host(Box::new(PhysicsQueryHost {
                contexts: Arc::clone(contexts),
            }));
        game_loop.runtime.set_script_host_name("physics-query-test");
        game_loop
            .runtime
            .load_script_assembly("game", "physics-query-test", b"test")
            .unwrap();

        let mut scene = engine_scene::sample_scene();
        let target = scene
            .entities
            .iter_mut()
            .find(|entity| entity.persistent_id == "cube-01")
            .unwrap();
        target.components.insert(
            "engine.transform".into(),
            engine_scene::ComponentRecord {
                schema_version: SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: BTreeMap::new(),
            },
        );
        target.components.insert(
            "engine.physics.rigid_body".into(),
            engine_scene::ComponentRecord {
                schema_version: SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: BTreeMap::from([("body_type".into(), Value::Enum("Static".into()))]),
            },
        );
        target.components.insert(
            "engine.physics.collider".into(),
            engine_scene::ComponentRecord {
                schema_version: SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: BTreeMap::new(),
            },
        );
        target.components.insert(
            "engine.script".into(),
            engine_scene::ComponentRecord {
                schema_version: SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: BTreeMap::from([
                    ("assembly_id".into(), Value::Str("game".into())),
                    ("class_name".into(), Value::Str("Probe".into())),
                ]),
            },
        );
        game_loop.load_scene(scene).unwrap();
        game_loop
    }

    #[test]
    fn physics_queries_report_persistent_ids_in_the_next_frame_snapshot() {
        use engine_script::GameplayPhysicsQueryResult;

        let contexts = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut game_loop = physics_query_game_loop(&contexts);

        // Frame 1: the script issues its queries; no results yet.
        game_loop.update(1.0 / 60.0);
        let first = contexts.lock().unwrap().last().unwrap().clone();
        assert!(first.physics_query_results.is_empty());

        // Frame 2: results arrive keyed by the script-chosen query ids.
        game_loop.update(1.0 / 60.0);
        let second = contexts.lock().unwrap().last().unwrap().clone();
        assert_eq!(second.entity_id, "cube-01");
        assert_eq!(second.physics_query_results.len(), 3);

        let hit = second
            .physics_query_results
            .iter()
            .find_map(|result| match result {
                GameplayPhysicsQueryResult::RaycastHit {
                    query_id: 11,
                    entity_id,
                    point,
                    normal,
                    distance,
                } => Some((entity_id.clone(), *point, *normal, *distance)),
                _ => None,
            })
            .expect("raycast hit result for query 11");
        assert_eq!(hit.0, "cube-01");
        assert!((hit.1[1] - 0.5).abs() < 1.0e-4, "hit point: {:?}", hit.1);
        assert!(
            hit.1[0].abs() < 1.0e-4 && hit.1[2].abs() < 1.0e-4,
            "hit point: {:?}",
            hit.1
        );
        assert!(
            (hit.2[1] - 1.0).abs() < 1.0e-4 && hit.2[0].abs() < 1.0e-4 && hit.2[2].abs() < 1.0e-4,
            "hit normal: {:?}",
            hit.2
        );
        assert!((hit.3 - 4.5).abs() < 1.0e-4, "hit distance: {}", hit.3);

        assert!(second.physics_query_results.iter().any(|result| matches!(
            result,
            GameplayPhysicsQueryResult::RaycastMiss { query_id: 12 }
        )));
        assert!(second.physics_query_results.iter().any(|result| matches!(
            result,
            GameplayPhysicsQueryResult::OverlapSphere { query_id: 13, entity_ids }
                if entity_ids == &vec!["cube-01".to_string()]
        )));

        // Frame 3: results are frame-local and expire with the next snapshot.
        game_loop.update(1.0 / 60.0);
        let third = contexts.lock().unwrap().last().unwrap().clone();
        assert!(third.physics_query_results.is_empty());
        assert!(game_loop
            .runtime
            .diagnostics_collector()
            .script_diagnostics
            .is_empty());
    }

    #[test]
    fn invalid_physics_queries_report_script_diagnostics_and_never_execute() {
        use engine_script::GameplayPhysicsQueryResult;

        let contexts = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut game_loop = GameLoop::new(EngineConfig::default());
        game_loop
            .runtime
            .register_script_host(Box::new(ContextRecordingHost {
                contexts: Arc::clone(&contexts),
            }));
        game_loop.runtime.set_script_host_name("context-recording");
        game_loop
            .runtime
            .load_script_assembly("game", "context-recording", b"test")
            .unwrap();
        game_loop.load_scene(engine_scene::sample_scene()).unwrap();

        // A typed host can bypass the JSON decoder, so the runtime
        // re-validates before staging anything for the physics world.
        let invalid = engine_script::OwnedGameplayCommand {
            entity_id: "cube-01".into(),
            command: GameplayCommand::PhysicsQuery {
                query: engine_script::GameplayPhysicsQuery::Raycast {
                    query_id: 1,
                    origin: [f32::NAN, 0.0, 0.0],
                    direction: [0.0, -1.0, 0.0],
                    max_distance: 10.0,
                },
            },
        };
        let diagnostics = game_loop
            .runtime
            .apply_script_gameplay_commands(vec![invalid]);
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SCRIPT_PHYSICS_QUERY_INVALID"));
        assert!(game_loop.runtime.take_pending_physics_queries().is_empty());

        // The invalid query never reaches a script snapshot.
        game_loop.update(1.0 / 60.0);
        assert!(contexts.lock().unwrap().iter().all(|context| context
            .physics_query_results
            .iter()
            .all(|result| !matches!(
                result,
                GameplayPhysicsQueryResult::RaycastHit { query_id: 1, .. }
            ))));
    }
}

#[cfg(test)]
mod world_origin_tests {
    use engine_scene::components::Transform;

    use super::*;

    /// Load the sample scene with the origin-shift trigger enabled and give
    /// the camera and cube explicit root transforms.
    fn shiftable_game_loop(threshold: f32) -> GameLoop {
        let mut game_loop = GameLoop::new(EngineConfig::default());
        game_loop.load_scene(engine_scene::sample_scene()).unwrap();
        game_loop
            .runtime
            .with_world_mut(|world| {
                world.scene_settings_mut().origin_shift.enabled = true;
                world.scene_settings_mut().origin_shift.threshold = threshold;
                let camera = world.entity_by_persistent_id("camera-main").unwrap();
                world.add_component(camera, Transform::default());
                let cube = world.entity_by_persistent_id("cube-01").unwrap();
                world.add_component(cube, Transform::default());
            })
            .unwrap();
        game_loop
    }

    fn set_translation(game_loop: &GameLoop, persistent_id: &str, translation: Vec3) {
        game_loop
            .runtime
            .with_world_mut(|world| {
                let entity = world.entity_by_persistent_id(persistent_id).unwrap();
                world.get_mut::<Transform>(entity).unwrap().translation = translation;
            })
            .unwrap();
    }

    fn translation_of(game_loop: &GameLoop, persistent_id: &str) -> Vec3 {
        game_loop
            .runtime
            .with_world(|world| {
                let entity = world.entity_by_persistent_id(persistent_id).unwrap();
                world.get::<Transform>(entity).unwrap().translation
            })
            .unwrap()
    }

    /// Logical position: `world_origin + world_position`. This is the
    /// invariant every origin shift must preserve.
    fn logical_position(game_loop: &GameLoop, persistent_id: &str) -> [f64; 3] {
        game_loop
            .runtime
            .with_world(|world| {
                let entity = world.entity_by_persistent_id(persistent_id).unwrap();
                let position = engine_scene::entity_world_position(world, entity).unwrap();
                let origin = world.world_origin();
                [
                    origin[0] + f64::from(position.x),
                    origin[1] + f64::from(position.y),
                    origin[2] + f64::from(position.z),
                ]
            })
            .unwrap()
    }

    #[test]
    fn origin_shift_is_disabled_by_default() {
        let mut game_loop = GameLoop::new(EngineConfig::default());
        game_loop.load_scene(engine_scene::sample_scene()).unwrap();
        game_loop
            .runtime
            .with_world_mut(|world| {
                let camera = world.entity_by_persistent_id("camera-main").unwrap();
                world.add_component(camera, Transform::default());
            })
            .unwrap();
        // Past the default 8 km threshold, but the opt-in flag stays off.
        set_translation(&game_loop, "camera-main", Vec3::new(9000.0, 0.0, 0.0));

        assert!(game_loop.tick_world_origin_shift().is_none());
        assert_eq!(game_loop.world_origin(), [0.0; 3]);
        assert_eq!(game_loop.world_origin_shift_count(), 0);
        assert_eq!(game_loop.last_world_origin_shift(), None);
        assert_eq!(
            translation_of(&game_loop, "camera-main"),
            Vec3::new(9000.0, 0.0, 0.0)
        );
    }

    #[test]
    fn origin_shift_triggers_past_threshold_and_preserves_logical_positions() {
        let mut game_loop = shiftable_game_loop(100.0);
        set_translation(&game_loop, "camera-main", Vec3::new(150.0, 0.0, 0.0));
        set_translation(&game_loop, "cube-01", Vec3::new(160.0, 5.0, -20.0));
        let cube_logical_before = logical_position(&game_loop, "cube-01");

        let shift = game_loop.tick_world_origin_shift().expect("shift runs");

        assert_eq!(shift.delta, [150.0, 0.0, 0.0]);
        assert_eq!(shift.origin, [150.0, 0.0, 0.0]);
        assert_eq!(shift.transforms, 2);
        assert_eq!(game_loop.world_origin(), [150.0, 0.0, 0.0]);
        assert_eq!(game_loop.world_origin_shift_count(), 1);
        assert_eq!(game_loop.last_world_origin_shift(), Some(shift));
        // The reference camera lands back on the relative origin and every
        // logical position is unchanged.
        assert!(translation_of(&game_loop, "camera-main").length() < 1e-4);
        assert_eq!(logical_position(&game_loop, "cube-01"), cube_logical_before);
        assert_eq!(
            logical_position(&game_loop, "camera-main"),
            [150.0, 0.0, 0.0]
        );
    }

    #[test]
    fn origin_shift_stays_put_below_threshold() {
        let mut game_loop = shiftable_game_loop(100.0);
        set_translation(&game_loop, "camera-main", Vec3::new(50.0, 0.0, 0.0));

        assert!(game_loop.tick_world_origin_shift().is_none());
        assert_eq!(game_loop.world_origin(), [0.0; 3]);
        assert_eq!(game_loop.world_origin_shift_count(), 0);
        assert_eq!(
            translation_of(&game_loop, "camera-main"),
            Vec3::new(50.0, 0.0, 0.0)
        );
    }

    #[test]
    fn origin_shift_runs_at_most_once_per_tick() {
        let mut game_loop = shiftable_game_loop(100.0);
        set_translation(&game_loop, "camera-main", Vec3::new(150.0, 0.0, 0.0));

        assert!(game_loop.tick_world_origin_shift().is_some());
        // After the shift the reference sits at the relative origin, so a
        // second evaluation in the same frame must not shift again.
        assert!(game_loop.tick_world_origin_shift().is_none());
        assert_eq!(game_loop.world_origin_shift_count(), 1);
    }

    #[test]
    fn origin_shift_watches_the_configured_reference_entity() {
        let mut game_loop = shiftable_game_loop(100.0);
        game_loop
            .runtime
            .with_world_mut(|world| {
                world.scene_settings_mut().origin_shift.reference_entity =
                    Some("cube-01".to_string());
            })
            .unwrap();
        set_translation(&game_loop, "camera-main", Vec3::new(10.0, 0.0, 0.0));
        set_translation(&game_loop, "cube-01", Vec3::new(220.0, 0.0, 30.0));

        let shift = game_loop.tick_world_origin_shift().expect("shift runs");

        assert_eq!(shift.delta, [220.0, 0.0, 30.0]);
        // The reference entity lands on the relative origin while the camera
        // keeps its logical position.
        assert!(translation_of(&game_loop, "cube-01").length() < 1e-4);
        assert_eq!(
            logical_position(&game_loop, "camera-main"),
            [10.0, 0.0, 0.0]
        );
    }

    #[test]
    fn origin_shift_waits_for_the_frame_boundary() {
        let mut game_loop = shiftable_game_loop(100.0);
        set_translation(&game_loop, "camera-main", Vec3::new(150.0, 0.0, 0.0));

        // `update()` never triggers a shift mid-frame; the host calls
        // `tick_world_origin_shift` at the frame boundary.
        game_loop.update(0.1);
        assert_eq!(game_loop.world_origin(), [0.0; 3]);
        assert_eq!(game_loop.world_origin_shift_count(), 0);

        assert!(game_loop.tick_world_origin_shift().is_some());
        assert_eq!(game_loop.world_origin_shift_count(), 1);
    }

    #[test]
    fn origin_shift_moves_character_controllers_and_the_primary_mirror() {
        let mut game_loop = shiftable_game_loop(100.0);
        game_loop
            .runtime
            .with_world_mut(|world| {
                let cube = world.entity_by_persistent_id("cube-01").unwrap();
                let mut controller = CharacterController::new();
                controller.set_position(Vec3::new(200.0, 1.0, 0.0));
                world.add_component(cube, controller);
            })
            .unwrap();
        let mut mirror = CharacterController::new();
        mirror.set_position(Vec3::new(200.0, 1.0, 0.0));
        game_loop.character = Some(mirror);
        set_translation(&game_loop, "camera-main", Vec3::new(150.0, 0.0, 0.0));

        let shift = game_loop.tick_world_origin_shift().expect("shift runs");

        assert_eq!(shift.character_controllers, 1);
        game_loop
            .runtime
            .with_world(|world| {
                let cube = world.entity_by_persistent_id("cube-01").unwrap();
                assert_eq!(
                    world.get::<CharacterController>(cube).unwrap().position(),
                    Vec3::new(50.0, 1.0, 0.0)
                );
            })
            .unwrap();
        // The primary mirror moves with the component so a same-frame read
        // cannot observe the pre-shift position.
        assert_eq!(
            game_loop.character.as_ref().unwrap().position(),
            Vec3::new(50.0, 1.0, 0.0)
        );
    }

    #[cfg(feature = "gameplay")]
    #[test]
    fn origin_shift_teleports_physics_bodies_and_sweeps_gravity_sources() {
        use engine_physics::{Collider, GravityMode, GravitySource, RigidBody};

        let mut game_loop = shiftable_game_loop(100.0);
        game_loop
            .runtime
            .with_world_mut(|world| {
                let cube = world.entity_by_persistent_id("cube-01").unwrap();
                world.add_component(cube, RigidBody::default());
                world.add_component(cube, Collider::default());
                world.add_component(
                    cube,
                    GravitySource {
                        mode: GravityMode::Point,
                        center: Vec3::new(400.0, 0.0, 0.0),
                        ..GravitySource::default()
                    },
                );
            })
            .unwrap();
        set_translation(&game_loop, "cube-01", Vec3::new(300.0, 0.0, 0.0));
        game_loop.init_physics();
        set_translation(&game_loop, "camera-main", Vec3::new(150.0, 0.0, 0.0));

        let shift = game_loop.tick_world_origin_shift().expect("shift runs");

        assert_eq!(shift.physics_bodies, 1);
        assert_eq!(shift.gravity_sources, 1);
        // The point gravity centre moved by -delta.
        game_loop
            .runtime
            .with_world(|world| {
                let cube = world.entity_by_persistent_id("cube-01").unwrap();
                assert_eq!(
                    world.get::<GravitySource>(cube).unwrap().center,
                    Vec3::new(250.0, 0.0, 0.0)
                );
            })
            .unwrap();
        // Queries observe the teleported body immediately: a ray over the
        // shifted position hits, a ray over the stale position misses.
        let physics = game_loop.physics.as_ref().unwrap();
        assert!(physics
            .raycast(Vec3::new(150.0, 10.0, 0.0), Vec3::NEG_Y, 100.0)
            .is_some());
        assert!(physics
            .raycast(Vec3::new(300.0, 10.0, 0.0), Vec3::NEG_Y, 100.0)
            .is_none());

        // A subsequent frame keeps the entity at its logical position: the
        // physics -> ECS resync must not yank the body back to pre-shift
        // coordinates.
        game_loop.update(0.0);
        let cube_x = translation_of(&game_loop, "cube-01").x;
        assert!((cube_x - 150.0).abs() < 1e-3, "{cube_x}");
        assert_eq!(logical_position(&game_loop, "cube-01")[0], 300.0);
    }

    #[cfg(feature = "runtime-subsystems")]
    #[test]
    fn origin_shift_moves_nav_agent_targets() {
        let mut game_loop = shiftable_game_loop(100.0);
        game_loop
            .runtime
            .with_world_mut(|world| {
                let cube = world.entity_by_persistent_id("cube-01").unwrap();
                let mut agent = engine_nav::AiAgent::new();
                agent.target = Some(Vec3::new(500.0, 0.0, -100.0));
                world.add_component(cube, agent);
            })
            .unwrap();
        set_translation(&game_loop, "camera-main", Vec3::new(150.0, 0.0, 0.0));

        let shift = game_loop.tick_world_origin_shift().expect("shift runs");

        assert_eq!(shift.nav_agents, 1);
        game_loop
            .runtime
            .with_world(|world| {
                let cube = world.entity_by_persistent_id("cube-01").unwrap();
                assert_eq!(
                    world.get::<engine_nav::AiAgent>(cube).unwrap().target,
                    Some(Vec3::new(350.0, 0.0, -100.0))
                );
            })
            .unwrap();
    }

    #[cfg(feature = "runtime-audio-output")]
    #[test]
    fn origin_shift_moves_audio_snapshot_positions() {
        let mut game_loop = shiftable_game_loop(100.0);
        game_loop
            .runtime
            .with_world_mut(|world| {
                let cube = world.entity_by_persistent_id("cube-01").unwrap();
                let mut source = engine_audio::AudioSourceComponent::default();
                source.spatial = true;
                world.add_component(cube, source);
                let camera = world.entity_by_persistent_id("camera-main").unwrap();
                world.add_component(camera, engine_audio::AudioListenerComponent::default());
            })
            .unwrap();
        set_translation(&game_loop, "camera-main", Vec3::new(150.0, 0.0, 0.0));
        set_translation(&game_loop, "cube-01", Vec3::new(300.0, 0.0, 0.0));

        game_loop.tick_world_origin_shift().expect("shift runs");

        // Audio state is rebuilt from ECS transforms every frame, so the
        // next snapshot already observes the shifted positions.
        let frame = game_loop.runtime_audio_frame();
        assert_eq!(frame.sources.len(), 1);
        assert_eq!(
            frame.sources[0].emitter.as_ref().unwrap().position,
            Vec3::new(150.0, 0.0, 0.0)
        );
        let listener = frame.listener.as_ref().unwrap();
        assert!(listener.position.length() < 1e-4, "{listener:?}");
    }
}
