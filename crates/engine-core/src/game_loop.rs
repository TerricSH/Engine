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

#[cfg(feature = "subsystem-ui")]
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
#[cfg(feature = "subsystem-ui")]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RuntimeUiValue {
    Bool(bool),
    Float(f32),
}

#[cfg(all(test, feature = "subsystem-ui"))]
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

#[cfg(feature = "subsystem-ui")]
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

    /// Capture a complete live-world checkpoint.
    ///
    /// `custom_state` is the project-owned portion of the save (inventory,
    /// objectives, dialogue flags, and similar rules). The engine adds the
    /// live ECS scene, world origin, game state, and transient rigid-body
    /// state.
    pub fn capture_save_game(
        &self,
        custom_state: std::collections::BTreeMap<String, engine_serialize::Value>,
    ) -> Result<crate::SaveGameSnapshot, crate::SaveGameError> {
        let scene = crate::savegame::capture_live_scene(&self.runtime)?;
        let world_origin = self
            .runtime
            .with_world(|world| world.world_origin())
            .ok_or(crate::SaveGameError::NoWorld)?;

        #[cfg(feature = "subsystem-physics")]
        let physics_bodies = if let Some(physics) = &self.physics {
            let states = physics.runtime_body_states();
            self.runtime
                .with_world(|world| {
                    states
                        .into_iter()
                        .filter_map(|(entity, state)| {
                            Some(crate::SavedPhysicsBody {
                                entity_id: world.persistent_id(entity)?.to_string(),
                                position: state.position,
                                rotation: state.rotation,
                                linear_velocity: state.linear_velocity,
                                angular_velocity: state.angular_velocity,
                                sleeping: state.sleeping,
                            })
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        #[cfg(not(feature = "subsystem-physics"))]
        let physics_bodies = Vec::new();

        #[cfg(feature = "subsystem-gameplay")]
        let game_state = Some(self.state_manager.current().to_u32());
        #[cfg(not(feature = "subsystem-gameplay"))]
        let game_state = None;

        let mut snapshot = crate::SaveGameSnapshot {
            schema_version: crate::SAVE_GAME_SCHEMA_VERSION,
            scene,
            world_origin,
            world_origin_shift_count: self.world_origin_shift_count,
            game_state,
            physics_bodies,
            custom_state,
        };
        snapshot
            .physics_bodies
            .sort_by(|left, right| left.entity_id.cmp(&right.entity_id));
        snapshot.validate()?;
        Ok(snapshot)
    }

    /// Restore a previously decoded checkpoint.
    ///
    /// Scene installation is transactional: validation and ECS construction
    /// finish before the active world is replaced. Missing physics entities
    /// are reported as skips so saves remain forward-compatible with a scene
    /// that intentionally removed a prop.
    pub fn restore_save_game(
        &mut self,
        snapshot: crate::SaveGameSnapshot,
    ) -> Result<crate::SaveGameRestoreReport, crate::SaveGameError> {
        snapshot.validate()?;
        let crate::SaveGameSnapshot {
            scene,
            world_origin,
            world_origin_shift_count,
            game_state,
            physics_bodies,
            custom_state,
            ..
        } = snapshot;
        self.load_scene(scene).map_err(|diagnostics| {
            crate::SaveGameError::SceneRestore(
                diagnostics
                    .iter()
                    .map(|diagnostic| format!("{}: {}", diagnostic.code, diagnostic.message))
                    .collect::<Vec<_>>()
                    .join("; "),
            )
        })?;
        self.runtime
            .with_world_mut(|world| world.restore_world_origin(world_origin))
            .expect("load_scene installed a world")
            .map_err(|error| crate::SaveGameError::InvalidSnapshot(error.to_string()))?;
        self.world_origin_shift_count = world_origin_shift_count;
        self.last_world_origin_shift = None;

        #[cfg(feature = "subsystem-gameplay")]
        if let Some(state) = game_state.and_then(engine_gameplay::GameState::from_u32) {
            self.state_manager.force_transition(state);
        }
        #[cfg(not(feature = "subsystem-gameplay"))]
        let _ = game_state;

        #[cfg(feature = "subsystem-physics")]
        let (restored_physics_bodies, skipped_physics_bodies) = {
            let mut restored = 0;
            let mut skipped = Vec::new();
            let runtime = &self.runtime;
            let resolved = runtime
                .with_world(|world| {
                    physics_bodies
                        .iter()
                        .map(|body| (body, world.entity_by_persistent_id(&body.entity_id)))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if let Some(physics) = &mut self.physics {
                for (body, entity) in resolved {
                    let Some(entity) = entity else {
                        skipped.push(body.entity_id.clone());
                        continue;
                    };
                    let state = engine_physics::RigidBodyRuntimeState {
                        position: body.position,
                        rotation: body.rotation,
                        linear_velocity: body.linear_velocity,
                        angular_velocity: body.angular_velocity,
                        sleeping: body.sleeping,
                    };
                    if physics.restore_runtime_body_state(entity, &state) {
                        restored += 1;
                    } else {
                        skipped.push(body.entity_id.clone());
                    }
                }
                runtime.with_world_mut(|world| physics.sync_to_ecs(world));
            } else {
                skipped.extend(physics_bodies.iter().map(|body| body.entity_id.clone()));
            }
            (restored, skipped)
        };
        #[cfg(not(feature = "subsystem-physics"))]
        let (restored_physics_bodies, skipped_physics_bodies) = {
            let skipped = physics_bodies
                .into_iter()
                .map(|body| body.entity_id)
                .collect();
            (0, skipped)
        };

        Ok(crate::SaveGameRestoreReport {
            restored_physics_bodies,
            skipped_physics_bodies,
            custom_state,
        })
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
    /// No-op when the `subsystem-physics` feature is not enabled.
    pub fn init_physics(&mut self) {
        #[cfg(feature = "subsystem-physics")]
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
    #[cfg(feature = "subsystem-physics")]
    pub fn physics_events(&self) -> &PhysicsEvents {
        &self.physics_events
    }

    /// Re-synchronise the physics world after direct ECS world mutations
    /// that bypass [`load_scene`](Self::load_scene) — world-partition cell
    /// streaming merges/unloads commit at the frame boundary and call this.
    ///
    /// With the `subsystem-physics` feature this runs the incremental
    /// `PhysicsWorld::sync_from_ecs`: bodies and colliders are created for
    /// newly merged entities and removed for unloaded ones, while every
    /// untouched entity keeps its exact simulation state. Scene-level
    /// physics settings (gravity) cannot change through cell merges because
    /// merges preserve world scene metadata, so no full rebuild is needed.
    /// Without the `subsystem-physics` feature this is a no-op.
    pub fn resync_physics_from_world(&mut self) {
        #[cfg(feature = "subsystem-physics")]
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
    ///   joints, and sleep state preserved (`subsystem-physics` feature),
    /// - every `CharacterController` position, including the primary mirror
    ///   used by [`update_character`](Self::update_character),
    /// - every navigation agent's target and in-progress path
    ///   (`subsystem-navigation` feature),
    /// - every point `GravitySource` center (`subsystem-physics` feature).
    /// - every live world-space CPU particle.
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
        let (transforms, character_controllers, nav_agents, gravity_sources, vfx_particles) =
            self.runtime.with_world_mut(|world| {
                let transforms = world.shift_world_origin(delta);

                let mut characters = 0usize;
                for (_, controller) in world.query_all_mut::<CharacterController>() {
                    let position = controller.position();
                    controller.set_position(position - offset);
                    characters += 1;
                }

                #[cfg(feature = "subsystem-navigation")]
                let nav_agents = {
                    let mut count = 0usize;
                    for (_, agent) in world.query_all_mut::<engine_nav::AiAgent>() {
                        agent.shift_world_positions(-offset);
                        count += 1;
                    }
                    count
                };
                #[cfg(not(feature = "subsystem-navigation"))]
                let nav_agents = 0usize;

                #[cfg(feature = "subsystem-physics")]
                let gravity_sources = engine_physics::shift_gravity_source_centers(world, -offset);
                #[cfg(not(feature = "subsystem-physics"))]
                let gravity_sources = 0usize;

                let vfx_particles = engine_vfx::shift_world_positions(world, -offset);

                (
                    transforms,
                    characters,
                    nav_agents,
                    gravity_sources,
                    vfx_particles,
                )
            })?;

        #[cfg(feature = "subsystem-physics")]
        let physics_bodies = self
            .physics
            .as_mut()
            .map(|physics| physics.translate_bodies(-offset))
            .unwrap_or(0);
        #[cfg(not(feature = "subsystem-physics"))]
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
            vfx_particles,
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
            vfx_particles = shift.vfx_particles,
            count = self.world_origin_shift_count,
            "world origin shifted"
        );
        Some(shift)
    }

    /// Take the most recent physics event snapshot, leaving it empty.
    #[cfg(feature = "subsystem-physics")]
    pub fn take_physics_events(&mut self) -> PhysicsEvents {
        std::mem::take(&mut self.physics_events)
    }

    /// Switch a scene-authored ragdoll between animation and physics
    /// ownership. Activation impulse is distributed across generated bodies;
    /// deactivation blends back over `recovery_duration` seconds.
    #[cfg(all(feature = "subsystem-animation", feature = "subsystem-physics"))]
    pub fn set_ragdoll_active(
        &mut self,
        entity_id: &str,
        active: bool,
        recovery_duration: f32,
        impulse: Vec3,
    ) -> Result<Vec<String>, String> {
        let previous = self
            .runtime
            .with_world(|world| {
                world
                    .entity_by_persistent_id(entity_id)
                    .and_then(|entity| world.get::<engine_animation::RagdollComponent>(entity))
                    .cloned()
            })
            .flatten()
            .ok_or_else(|| format!("entity '{entity_id}' has no Ragdoll component"))?;
        crate::ragdoll_runtime::set_active(self, entity_id, active, recovery_duration, impulse)?;
        crate::ragdoll_runtime::reconcile_before_physics(self);
        let generated = self
            .runtime
            .with_world(|world| {
                let entity = world
                    .entity_by_persistent_id(entity_id)
                    .ok_or_else(|| format!("ragdoll target '{entity_id}' disappeared"))?;
                let ragdoll = world
                    .get::<engine_animation::RagdollComponent>(entity)
                    .ok_or_else(|| format!("entity '{entity_id}' lost its Ragdoll component"))?;
                if ragdoll.generated_body_ids.len() != ragdoll.bodies.len()
                    || ragdoll.generated_joint_ids.len() != ragdoll.constraints.len()
                {
                    return Err(format!(
                        "ragdoll graph for '{entity_id}' could not be generated"
                    ));
                }
                let mut body_ids = Vec::with_capacity(ragdoll.generated_body_ids.len());
                for body_id in ragdoll.generated_body_ids.values() {
                    let body = world.entity_by_persistent_id(body_id).ok_or_else(|| {
                        format!("ragdoll body '{body_id}' for '{entity_id}' is missing")
                    })?;
                    if world.get::<engine_physics::RigidBody>(body).is_none()
                        || world.get::<engine_physics::Collider>(body).is_none()
                    {
                        return Err(format!(
                            "ragdoll body '{body_id}' for '{entity_id}' is incomplete"
                        ));
                    }
                    body_ids.push(body_id.clone());
                }
                for joint_id in &ragdoll.generated_joint_ids {
                    let joint = world.entity_by_persistent_id(joint_id).ok_or_else(|| {
                        format!("ragdoll joint '{joint_id}' for '{entity_id}' is missing")
                    })?;
                    if world.get::<engine_physics::PhysicsJoint>(joint).is_none() {
                        return Err(format!(
                            "ragdoll joint '{joint_id}' for '{entity_id}' is incomplete"
                        ));
                    }
                }
                Ok(body_ids)
            })
            .ok_or_else(|| "no active world".to_string())?;
        match generated {
            Ok(body_ids) => Ok(body_ids),
            Err(error) => {
                self.runtime.with_world_mut(|world| {
                    if let Some(entity) = world.entity_by_persistent_id(entity_id) {
                        world.add_component(entity, previous);
                    }
                });
                crate::ragdoll_runtime::reconcile_before_physics(self);
                Err(error)
            }
        }
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
        #[cfg(feature = "subsystem-physics")]
        {
            let physics: Option<&PhysicsWorld> = self.physics.as_ref();
            ctrl.update(&input, physics);
        }
        #[cfg(not(feature = "subsystem-physics"))]
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

    /// Refresh the primary controller mirror after script commands have
    /// queued movement intent on the ECS component. The mirror drives the
    /// next frame and would otherwise overwrite those pending commands.
    #[cfg(feature = "subsystem-scripting-csharp")]
    fn refresh_primary_character_from_world(&mut self) {
        let Some(entity) = self.character_entity else {
            return;
        };
        if let Some(controller) = self
            .runtime
            .with_world(|world| world.get::<CharacterController>(entity).cloned())
            .flatten()
        {
            self.character = Some(controller);
        }
    }

    /// Advance the simulation by `dt` seconds.
    ///
    /// Handles physics stepping and ECS ↔ physics sync when
    /// `subsystem-physics` is enabled. Script ticking runs when the
    /// `subsystem-scripting-csharp` feature is active.
    ///
    /// Typical per-frame orchestration:
    /// 1. Resolve input events against `input_map`
    /// 2. Call `update(dt)` for physics + character + scripts
    /// 3. Call `render(frame_idx)` for extraction + draw
    pub fn update(&mut self, dt: f32) {
        // Frame-time attribution (ENG-04): the whole simulation step is the
        // `update` stage; script ticking nests inside it as `script_tick`.
        self.runtime.frame_timing_begin_stage("update");
        self.update_inner(dt);
        self.runtime.frame_timing_end_stage("update");
    }

    fn update_inner(&mut self, dt: f32) {
        #[cfg(feature = "subsystem-terrain")]
        self.tick_terrain(None);
        #[cfg(all(feature = "subsystem-animation", feature = "subsystem-physics"))]
        crate::ragdoll_runtime::reconcile_before_physics(self);
        // Tick physics (ECS → physics → ECS sync).
        #[cfg(feature = "subsystem-physics")]
        {
            self.physics_events.clear();
            if let Some(ref mut physics) = self.physics {
                self.runtime.with_world_mut(|world| {
                    physics.step(dt, world);
                });
                self.physics_events = physics.drain_events();
            }
        }

        #[cfg(feature = "subsystem-gameplay")]
        let (character_direction, character_jump) = self.resolved_character_input();
        #[cfg(not(feature = "subsystem-gameplay"))]
        let (character_direction, character_jump) = (Vec3::ZERO, false);
        #[cfg(feature = "subsystem-navigation")]
        self.queue_runtime_navigation(dt);
        self.update_character(character_direction, character_jump, dt);
        self.update_additional_characters(dt);

        #[cfg(feature = "subsystem-scripting-csharp")]
        let script_ui_events = {
            #[cfg(feature = "subsystem-ui")]
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
            #[cfg(not(feature = "subsystem-ui"))]
            {
                Vec::<engine_script::GameplayUiEvent>::new()
            }
        };

        #[cfg(feature = "subsystem-scripting-csharp")]
        self.refresh_script_view_context();

        // Build each optional input independently so scripting no longer
        // drags the gameplay, physics, animation, or UI subsystems with it.
        #[cfg(feature = "subsystem-scripting-csharp")]
        {
            self.runtime.frame_timing_begin_stage("script_tick");
            #[cfg(feature = "subsystem-gameplay")]
            let input_actions = self.resolved_script_input_actions();
            #[cfg(not(feature = "subsystem-gameplay"))]
            let input_actions = std::collections::BTreeMap::new();
            #[cfg(feature = "subsystem-gameplay")]
            let input_transitions = self.resolved_script_input_transitions(&input_actions);
            #[cfg(not(feature = "subsystem-gameplay"))]
            let input_transitions = engine_script::GameplayInputTransitions::default();
            #[cfg(feature = "subsystem-physics")]
            let physics_events = self.resolved_script_physics_events();
            #[cfg(not(feature = "subsystem-physics"))]
            let physics_events = std::collections::BTreeMap::new();
            #[cfg(feature = "subsystem-physics")]
            let physics_query_results = std::mem::take(&mut self.script_physics_query_results);
            #[cfg(not(feature = "subsystem-physics"))]
            let physics_query_results = std::collections::BTreeMap::new();

            self.runtime
                .tick_scripts_with_frame_input_ui_and_physics_queries(
                    dt,
                    &input_actions,
                    &input_transitions,
                    &physics_events,
                    &script_ui_events,
                    &physics_query_results,
                );

            #[cfg(feature = "subsystem-gameplay")]
            {
                self.previous_script_input_actions = input_actions;
            }
            #[cfg(feature = "subsystem-physics")]
            {
                self.execute_script_physics_queries();
                self.queue_script_physics_mutations();
                self.process_script_damage_requests();
            }
            #[cfg(not(feature = "subsystem-physics"))]
            {
                let _ = self.runtime.take_pending_physics_queries();
                let _ = self.runtime.take_pending_physics_mutations();
                let _ = self.runtime.take_pending_damage_requests();
            }
            #[cfg(all(feature = "subsystem-animation", feature = "subsystem-physics"))]
            self.process_script_ragdoll_requests();
            #[cfg(not(all(feature = "subsystem-animation", feature = "subsystem-physics")))]
            let _ = self.runtime.take_pending_ragdoll_requests();
            self.runtime.frame_timing_end_stage("script_tick");
        }
        #[cfg(feature = "subsystem-scripting-csharp")]
        self.process_script_save_requests();
        #[cfg(feature = "subsystem-scripting-csharp")]
        self.refresh_primary_character_from_world();
        #[cfg(feature = "subsystem-scripting-csharp")]
        self.finish_script_pointer_frame();

        #[cfg(all(feature = "subsystem-animation", feature = "subsystem-physics"))]
        {
            crate::ragdoll_runtime::reconcile_after_physics(self, dt);
        }

        #[cfg(feature = "subsystem-animation")]
        self.update_runtime_animation(dt);
        self.runtime
            .with_world_mut(|world| engine_vfx::update_vfx(world, dt));
        #[cfg(feature = "runtime-audio-output")]
        self.update_runtime_audio(dt);
    }

    /// Tick the optional terrain component at the frame boundary. Normal
    /// [`update`](Self::update) calls this with the active camera; editor and
    /// server hosts may supply an absolute/logical focus explicitly.
    #[cfg(feature = "subsystem-terrain")]
    pub fn tick_terrain(&mut self, focus_logical: Option<[f64; 3]>) {
        self.runtime.frame_timing_begin_stage("terrain_stream");
        let physics_changed = self.terrain.tick(&mut self.runtime, focus_logical);
        if physics_changed {
            self.resync_physics_from_world();
        }
        self.runtime.frame_timing_end_stage("terrain_stream");
    }

    /// Snapshot used by the ENG-70 editor panel and headless diagnostics.
    #[cfg(feature = "subsystem-terrain")]
    pub fn terrain_debug_snapshot(&self) -> engine_terrain::TerrainDebugSnapshot {
        self.terrain.debug_snapshot()
    }

    /// Regenerate resident and in-flight chunks without changing authored
    /// terrain parameters.
    #[cfg(feature = "subsystem-terrain")]
    pub fn terrain_force_regenerate(&mut self) {
        self.terrain.force_regenerate();
    }

    #[cfg(feature = "subsystem-gameplay")]
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

    /// Rolling per-pass CPU/GPU frame timing statistics (ENG-04).
    ///
    /// CPU stages: `update` (whole simulation step, with `script_tick`
    /// nested), `extraction`, `sync_render_assets`, `render_submit`. GPU pass
    /// times appear only when the active backend supports timestamps.
    pub fn frame_timing_summary(&self) -> engine_renderer::FrameTimingSummary {
        self.runtime.frame_timing_summary()
    }

    /// Set the pixel extent used by script pointer and camera projection data.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn set_script_viewport_size(&mut self, width: u32, height: u32) {
        self.script_pointer.viewport = [width.max(1) as f32, height.max(1) as f32];
        self.update_script_pointer_inside();
    }

    /// Configure the trusted directory used by script save slots.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn set_script_save_directory(&mut self, directory: impl Into<std::path::PathBuf>) {
        self.script_save_directory = Some(directory.into());
    }

    /// Update the gameplay pointer without coupling game input to retained UI.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn script_pointer_move(&mut self, x: f32, y: f32) {
        if !x.is_finite() || !y.is_finite() {
            self.script_pointer_focus(false);
            return;
        }
        self.script_pointer.delta[0] += x - self.script_pointer.position[0];
        self.script_pointer.delta[1] += y - self.script_pointer.position[1];
        self.script_pointer.position = [x, y];
        self.update_script_pointer_inside();
    }

    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn script_pointer_primary(&mut self, down: bool) {
        if down && !self.script_pointer.primary_down {
            self.script_pointer.primary_pressed = true;
        } else if !down && self.script_pointer.primary_down {
            self.script_pointer.primary_released = true;
        }
        self.script_pointer.primary_down = down;
    }

    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn script_pointer_secondary(&mut self, down: bool) {
        self.script_pointer.secondary_down = down;
    }

    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn script_pointer_middle(&mut self, down: bool) {
        self.script_pointer.middle_down = down;
    }

    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn script_pointer_scroll(&mut self, x: f32, y: f32) {
        if x.is_finite() && y.is_finite() {
            self.script_pointer.scroll[0] += x;
            self.script_pointer.scroll[1] += y;
        }
    }

    #[cfg(feature = "subsystem-scripting-csharp")]
    pub fn script_pointer_focus(&mut self, focused: bool) {
        self.script_pointer.focused = focused;
        if !focused {
            self.script_pointer.primary_released |= self.script_pointer.primary_down;
            self.script_pointer.primary_down = false;
            self.script_pointer.secondary_down = false;
            self.script_pointer.middle_down = false;
            self.script_pointer.inside_viewport = false;
        } else {
            self.update_script_pointer_inside();
        }
    }

    #[cfg(feature = "subsystem-scripting-csharp")]
    fn update_script_pointer_inside(&mut self) {
        let [width, height] = self.script_pointer.viewport;
        let [x, y] = self.script_pointer.position;
        self.script_pointer.inside_viewport = self.script_pointer.focused
            && width > 0.0
            && height > 0.0
            && x >= 0.0
            && y >= 0.0
            && x <= width
            && y <= height;
    }

    #[cfg(feature = "subsystem-scripting-csharp")]
    fn refresh_script_view_context(&mut self) {
        let [width, height] = self.script_pointer.viewport;
        let viewport = if width > 0.0 && height > 0.0 {
            engine_scene::RenderViewportContext::new(
                width.round() as u32,
                height.round() as u32,
                engine_renderer::Rect::FULL,
            )
            .unwrap_or_default()
        } else {
            engine_scene::RenderViewportContext::default()
        };
        let camera = self
            .runtime
            .with_world(|world| engine_scene::active_camera_view(world, viewport))
            .flatten();
        self.script_pointer.ray_origin = None;
        self.script_pointer.ray_direction = None;
        if self.script_pointer.inside_viewport {
            if let Some((origin, direction)) = camera
                .as_ref()
                .and_then(|camera| camera.screen_ray(self.script_pointer.position))
            {
                self.script_pointer.ray_origin = Some(origin.to_array());
                self.script_pointer.ray_direction = Some(direction.to_array());
            }
        }
        let camera = camera.map(|camera| engine_script::GameplayCameraSnapshot {
            entity_id: camera.entity_id,
            perspective: camera.perspective,
            position: camera.position.to_array(),
            forward: camera.forward.to_array(),
            right: camera.right.to_array(),
            up: camera.up.to_array(),
            viewport: camera.viewport_pixels,
            view_projection: camera.view_projection.to_cols_array(),
            inverse_view_projection: camera.inverse_view_projection.to_cols_array(),
        });
        self.runtime
            .set_script_view_context(self.script_pointer.clone(), camera);
    }

    #[cfg(feature = "subsystem-scripting-csharp")]
    fn finish_script_pointer_frame(&mut self) {
        self.script_pointer.delta = [0.0; 2];
        self.script_pointer.scroll = [0.0; 2];
        self.script_pointer.primary_pressed = false;
        self.script_pointer.primary_released = false;
    }

    #[cfg(feature = "subsystem-scripting-csharp")]
    fn process_script_save_requests(&mut self) {
        const SCRIPT_STATE_KEY: &str = "script_state_json";
        let requests = self.runtime.take_pending_save_requests();
        for (index, request) in requests.into_iter().enumerate() {
            let engine_script::OwnedGameplaySaveRequest {
                owner_entity_id,
                slot,
                operation,
            } = request;
            let outcome = if index > 0 {
                Err("only one save or load operation may execute per frame".to_string())
            } else if let Some(directory) = self.script_save_directory.clone() {
                let path = directory.join(format!("{slot}.save"));
                match operation {
                    engine_script::GameplaySaveOperation::Save { state_json } => self
                        .capture_save_game(std::collections::BTreeMap::from([(
                            SCRIPT_STATE_KEY.to_string(),
                            engine_serialize::Value::Str(state_json),
                        )]))
                        .and_then(|snapshot| crate::write_save_game(path, &snapshot))
                        .map(|_| (engine_script::GameplaySaveEventKind::Saved, None))
                        .map_err(|error| error.to_string()),
                    engine_script::GameplaySaveOperation::Load => crate::read_save_game(path)
                        .and_then(|snapshot| self.restore_save_game(snapshot))
                        .and_then(|report| {
                            let state_json = report
                                .custom_state
                                .get(SCRIPT_STATE_KEY)
                                .and_then(|value| match value {
                                    engine_serialize::Value::Str(value) => Some(value.clone()),
                                    _ => None,
                                })
                                .ok_or_else(|| {
                                    crate::SaveGameError::InvalidSnapshot(
                                        "checkpoint does not contain script state JSON".into(),
                                    )
                                })?;
                            Ok((
                                engine_script::GameplaySaveEventKind::Loaded,
                                Some(state_json),
                            ))
                        })
                        .map_err(|error| error.to_string()),
                }
            } else {
                Err("the runtime host did not configure a script save directory".to_string())
            };
            let event = match outcome {
                Ok((kind, state_json)) => engine_script::GameplaySaveEvent {
                    slot,
                    kind,
                    state_json,
                    error: None,
                },
                Err(error) => engine_script::GameplaySaveEvent {
                    slot,
                    kind: engine_script::GameplaySaveEventKind::Failed,
                    state_json: None,
                    error: Some(error),
                },
            };
            self.runtime.push_script_save_event(owner_entity_id, event);
        }
    }

    /// Produce a single rendered frame.
    pub fn render(&mut self, frame_index: u64) -> Result<FrameStats, Vec<Diagnostic>> {
        #[cfg(feature = "subsystem-ui")]
        {
            let ui_batches = self.runtime_ui_batches();
            self.runtime.render_frame_with_ui(frame_index, ui_batches)
        }
        #[cfg(not(feature = "subsystem-ui"))]
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
        #[cfg(feature = "subsystem-ui")]
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
        #[cfg(not(feature = "subsystem-ui"))]
        let ui_batches = engine_overlay_batches;

        self.runtime
            .render_frame_with_ui_in_viewport(frame_index, ui_batches, viewport)
    }

    /// Drain retained Canvas click events for a native host.
    ///
    /// Script-enabled [`update`](Self::update) consumes the same queue once
    /// when building gameplay contexts. Native hosts that want ownership must
    /// therefore call this before that update.
    #[cfg(feature = "subsystem-ui")]
    pub fn take_ui_events(&mut self) -> Vec<RuntimeUiEvent> {
        std::mem::take(&mut self.runtime_ui_events)
    }

    /// Whether a scene Canvas currently owns the primary pointer gesture.
    #[cfg(feature = "subsystem-ui")]
    pub fn ui_has_pointer_capture(&self) -> bool {
        self.runtime_ui_captured_canvas.is_some()
    }

    /// Update the screen viewport used by retained UI scaling and hit tests.
    #[cfg(feature = "subsystem-ui")]
    pub fn set_ui_viewport_size(&mut self, width: u32, height: u32) {
        self.runtime_ui_viewport = [width.max(1) as f32, height.max(1) as f32];
    }

    /// Update the retained UI primary-pointer position in Canvas coordinates.
    ///
    /// While a Canvas owns capture, movement is delivered only to that
    /// Canvas. Otherwise the topmost interactive Canvas under the pointer is
    /// selected using the same persistent-ID order as UI rendering.
    #[cfg(feature = "subsystem-ui")]
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
    #[cfg(feature = "subsystem-ui")]
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
    #[cfg(feature = "subsystem-ui")]
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
    #[cfg(feature = "subsystem-ui")]
    pub fn cancel_ui_pointer(&mut self) {
        let mut canvases = self.runtime_ui_canvases();
        for (canvas_id, canvas) in &mut canvases {
            if let Some(state) = self.runtime_ui_input_states.get_mut(canvas_id) {
                state.process_event(canvas, engine_ui::UiPointerEvent::Cancel);
            }
        }
        self.cancel_ui_pointer_state();
    }

    #[cfg(feature = "subsystem-ui")]
    fn cancel_ui_pointer_state(&mut self) {
        self.runtime_ui_input_states.clear();
        self.runtime_ui_captured_canvas = None;
    }

    #[cfg(feature = "subsystem-ui")]
    fn reset_runtime_ui_input(&mut self) {
        self.cancel_ui_pointer_state();
        self.runtime_ui_events.clear();
    }

    /// Snapshot and lay out all retained scene canvases in renderer order.
    #[cfg(feature = "subsystem-ui")]
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

    #[cfg(feature = "subsystem-ui")]
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

    #[cfg(feature = "subsystem-ui")]
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
    #[cfg(feature = "subsystem-ui")]
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
    #[cfg(feature = "subsystem-animation")]
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
    #[cfg(feature = "subsystem-navigation")]
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
        #[cfg(feature = "subsystem-physics")]
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
                #[cfg(feature = "subsystem-physics")]
                controller.update(&input, physics);
                #[cfg(not(feature = "subsystem-physics"))]
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

    #[cfg(all(feature = "subsystem-scripting-csharp", feature = "subsystem-gameplay"))]
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

    #[cfg(all(feature = "subsystem-scripting-csharp", feature = "subsystem-physics"))]
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
                    |entity_a,
                     entity_b,
                     kind: GameplayPhysicsEventKind,
                     joint_id: Option<String>,
                     force: Option<f32>,
                     torque: Option<f32>| {
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
                                joint_id: joint_id.clone(),
                                force,
                                torque,
                            },
                        );
                        by_entity.entry(entity_b.to_owned()).or_default().push(
                            GameplayPhysicsEvent {
                                kind,
                                other_entity_id: entity_a.to_owned(),
                                joint_id,
                                force,
                                torque,
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
                    record_pair(event.entity_a, event.entity_b, kind, None, None, None);
                }
                for event in &self.physics_events.triggers {
                    let kind = match event.kind {
                        TriggerEventKind::Entered => GameplayPhysicsEventKind::TriggerEntered,
                        TriggerEventKind::Stay => GameplayPhysicsEventKind::TriggerStayed,
                        TriggerEventKind::Exited => GameplayPhysicsEventKind::TriggerExited,
                    };
                    record_pair(event.entity_a, event.entity_b, kind, None, None, None);
                }
                for event in &self.physics_events.joint_breaks {
                    let joint_id = event
                        .joint_entity
                        .and_then(|entity| world.persistent_id(entity))
                        .map(str::to_owned);
                    record_pair(
                        event.entity_a,
                        event.entity_b,
                        GameplayPhysicsEventKind::JointBroken,
                        joint_id,
                        Some(event.force),
                        Some(event.torque),
                    );
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
    #[cfg(all(feature = "subsystem-scripting-csharp", feature = "subsystem-physics"))]
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

    /// Resolve validated script forces/impulses by persistent id and queue
    /// them for the next safe physics step.
    #[cfg(all(feature = "subsystem-scripting-csharp", feature = "subsystem-physics"))]
    fn queue_script_physics_mutations(&mut self) {
        let pending = self.runtime.take_pending_physics_mutations();
        if self.physics.is_none() {
            return;
        }
        for engine_script::OwnedGameplayPhysicsMutation {
            owner_entity_id: _,
            mutation,
        } in pending
        {
            use engine_script::{GameplayJointType, GameplayPhysicsMutation};
            match mutation {
                GameplayPhysicsMutation::ApplyForce { entity_id, force } => {
                    let entity = self
                        .runtime
                        .with_world(|world| world.entity_by_persistent_id(&entity_id))
                        .flatten();
                    if let (Some(entity), Some(physics)) = (entity, self.physics.as_mut()) {
                        physics.queue_command(engine_physics::PhysicsCommand::ApplyForce {
                            entity,
                            force: Vec3::from(force),
                        });
                    }
                }
                GameplayPhysicsMutation::ApplyImpulse { entity_id, impulse } => {
                    let entity = self
                        .runtime
                        .with_world(|world| world.entity_by_persistent_id(&entity_id))
                        .flatten();
                    if let (Some(entity), Some(physics)) = (entity, self.physics.as_mut()) {
                        physics.queue_command(engine_physics::PhysicsCommand::ApplyImpulse {
                            entity,
                            impulse: Vec3::from(impulse),
                        });
                    }
                }
                GameplayPhysicsMutation::ApplyTorque { entity_id, torque } => {
                    let entity = self
                        .runtime
                        .with_world(|world| world.entity_by_persistent_id(&entity_id))
                        .flatten();
                    if let (Some(entity), Some(physics)) = (entity, self.physics.as_mut()) {
                        physics.queue_command(engine_physics::PhysicsCommand::ApplyTorque {
                            entity,
                            torque: Vec3::from(torque),
                        });
                    }
                }
                GameplayPhysicsMutation::ApplyTorqueImpulse {
                    entity_id,
                    torque_impulse,
                } => {
                    let entity = self
                        .runtime
                        .with_world(|world| world.entity_by_persistent_id(&entity_id))
                        .flatten();
                    if let (Some(entity), Some(physics)) = (entity, self.physics.as_mut()) {
                        physics.queue_command(engine_physics::PhysicsCommand::ApplyTorqueImpulse {
                            entity,
                            torque_impulse: Vec3::from(torque_impulse),
                        });
                    }
                }
                GameplayPhysicsMutation::CreateJoint {
                    joint_id,
                    body_a,
                    body_b,
                    joint_type,
                    anchor_a,
                    anchor_b,
                    axis,
                    limits,
                    motor,
                    break_force,
                    break_torque,
                } => {
                    self.runtime.with_world_mut(|world| {
                        if world.entity_by_persistent_id(&body_a).is_none()
                            || world.entity_by_persistent_id(&body_b).is_none()
                        {
                            return;
                        }
                        let constraint = match world.entity_by_persistent_id(&joint_id) {
                            Some(entity) => entity,
                            None => {
                                let Ok(entity) = world.create_persistent_entity(joint_id.clone())
                                else {
                                    return;
                                };
                                entity
                            }
                        };
                        world.add_component(
                            constraint,
                            engine_physics::PhysicsJoint {
                                enabled: true,
                                body_a,
                                body_b,
                                joint_type: match joint_type {
                                    GameplayJointType::Fixed => engine_physics::JointType::Fixed,
                                    GameplayJointType::Revolute => {
                                        engine_physics::JointType::Revolute
                                    }
                                    GameplayJointType::Prismatic => {
                                        engine_physics::JointType::Prismatic
                                    }
                                    GameplayJointType::Spherical => {
                                        engine_physics::JointType::Spherical
                                    }
                                },
                                anchor_a,
                                anchor_b,
                                axis,
                                limits: limits.map(|limits| engine_physics::JointLimits {
                                    min: limits.min,
                                    max: limits.max,
                                    stiffness: limits.stiffness,
                                    damping: limits.damping,
                                }),
                                motor: motor.map(|motor| engine_physics::JointMotor {
                                    target_vel: motor.target_vel,
                                    target_pos: motor.target_pos,
                                    stiffness: motor.stiffness,
                                    damping: motor.damping,
                                }),
                                break_force,
                                break_torque,
                            },
                        );
                    });
                }
                GameplayPhysicsMutation::RemoveJoint { joint_id } => {
                    self.runtime.with_world_mut(|world| {
                        if let Some(entity) = world.entity_by_persistent_id(&joint_id) {
                            world.remove_component::<engine_physics::PhysicsJoint>(entity);
                        }
                    });
                }
            }
        }
    }

    #[cfg(all(feature = "subsystem-scripting-csharp", feature = "subsystem-physics"))]
    fn process_script_damage_requests(&mut self) {
        let pending = self.runtime.take_pending_damage_requests();
        for request in pending {
            let target = self
                .runtime
                .with_world(|world| world.entity_by_persistent_id(&request.target_entity_id))
                .flatten();
            let source = self
                .runtime
                .with_world(|world| world.entity_by_persistent_id(&request.owner_entity_id))
                .flatten();
            let Some(target) = target else {
                continue;
            };
            let damage_request = engine_physics::DamageRequest {
                source,
                amount: request.amount,
                kind: match request.damage_kind {
                    engine_script::GameplayDamageKind::Generic => {
                        engine_physics::DamageKind::Generic
                    }
                    engine_script::GameplayDamageKind::Impact => engine_physics::DamageKind::Impact,
                    engine_script::GameplayDamageKind::Bullet => engine_physics::DamageKind::Bullet,
                    engine_script::GameplayDamageKind::Blast => engine_physics::DamageKind::Blast,
                    engine_script::GameplayDamageKind::Fire => engine_physics::DamageKind::Fire,
                },
                hit_position: request.hit_position,
                impulse: request.impulse,
            };
            let result = self.runtime.with_world_mut(|world| {
                engine_physics::apply_damage(world, target, &damage_request)
            });
            let event = match result {
                Some(Ok(Some(event))) => event,
                Some(Ok(None)) => continue,
                Some(Err(error)) => {
                    let mut diagnostic = engine_serialize::Diagnostic::new(
                        "SCRIPT_DAMAGE_REJECTED",
                        engine_serialize::DiagnosticSeverity::Error,
                        "script",
                        format!(
                            "script entity '{}' could not damage '{}': {error}",
                            request.owner_entity_id, request.target_entity_id
                        ),
                    );
                    diagnostic.entity = Some(request.owner_entity_id);
                    self.runtime
                        .diagnostics_collector_mut()
                        .push_script_diags(vec![diagnostic]);
                    continue;
                }
                None => continue,
            };

            let mut spawned_entity_ids = Vec::new();
            if event.broke {
                let source_state = self.physics.as_ref().and_then(|physics| {
                    physics
                        .runtime_body_states()
                        .into_iter()
                        .find_map(|(entity, state)| (entity == target).then_some(state))
                });
                let target_translation = self
                    .runtime
                    .with_world(|world| {
                        world
                            .get::<engine_scene::components::Transform>(target)
                            .map(|transform| transform.translation.to_array())
                    })
                    .flatten()
                    .or(event.hit_position);
                let before_ids = self
                    .runtime
                    .with_world(|world| {
                        world
                            .persistent_entities()
                            .map(|(id, _)| id.to_owned())
                            .collect::<std::collections::BTreeSet<_>>()
                    })
                    .unwrap_or_default();

                let mut fracture_diagnostics = Vec::new();
                let replacement_succeeded = if let Some(prefab) = event.replacement_prefab.as_ref()
                {
                    self.runtime.spawn_script_prefab(
                        &request.owner_entity_id,
                        &prefab.id,
                        target_translation,
                        &mut fracture_diagnostics,
                        0,
                    );
                    let after_ids = self
                        .runtime
                        .with_world(|world| {
                            world
                                .persistent_entities()
                                .map(|(id, _)| id.to_owned())
                                .collect::<std::collections::BTreeSet<_>>()
                        })
                        .unwrap_or_default();
                    spawned_entity_ids = after_ids
                        .difference(&before_ids)
                        .cloned()
                        .collect::<Vec<_>>();
                    !spawned_entity_ids.is_empty()
                } else {
                    true
                };

                if replacement_succeeded {
                    if let Some(physics) = self.physics.as_mut() {
                        let rigid_pieces = self
                            .runtime
                            .with_world(|world| {
                                spawned_entity_ids
                                    .iter()
                                    .filter_map(|id| {
                                        let entity = world.entity_by_persistent_id(id)?;
                                        world
                                            .get::<engine_physics::RigidBody>(entity)
                                            .map(|_| entity)
                                    })
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_default();
                        let piece_count = rigid_pieces.len().max(1) as f32;
                        for piece in rigid_pieces {
                            if event.inherit_velocity {
                                if let Some(state) = source_state.as_ref() {
                                    physics.queue_command(
                                        engine_physics::PhysicsCommand::SetLinearVelocity {
                                            entity: piece,
                                            velocity: Vec3::from(state.linear_velocity),
                                        },
                                    );
                                    physics.queue_command(
                                        engine_physics::PhysicsCommand::SetAngularVelocity {
                                            entity: piece,
                                            velocity: Vec3::from(state.angular_velocity),
                                        },
                                    );
                                }
                            }
                            let impulse = Vec3::from(event.impulse)
                                * (event.fracture_impulse_scale / piece_count);
                            if impulse != Vec3::ZERO {
                                physics.queue_command(
                                    engine_physics::PhysicsCommand::ApplyImpulse {
                                        entity: piece,
                                        impulse,
                                    },
                                );
                            }
                        }
                    }

                    if event.destroy_on_break {
                        crate::destroy_script_entity(
                            &self.runtime.world_slot,
                            &mut self.runtime.script_engine,
                            &request.owner_entity_id,
                            &request.target_entity_id,
                            &mut fracture_diagnostics,
                        );
                    }
                }
                if !fracture_diagnostics.is_empty() {
                    self.runtime
                        .diagnostics_collector_mut()
                        .push_script_diags(fracture_diagnostics);
                }
            }

            let gameplay_event = engine_script::GameplayDamageEvent {
                target_entity_id: request.target_entity_id.clone(),
                source_entity_id: Some(request.owner_entity_id.clone()),
                damage_kind: request.damage_kind,
                raw_damage: event.raw_damage,
                applied_damage: event.applied_damage,
                remaining_health: event.remaining_health,
                hit_position: event.hit_position,
                impulse: event.impulse,
                broke: event.broke,
                spawned_entity_ids,
            };
            self.runtime
                .push_script_damage_event(request.target_entity_id.clone(), gameplay_event.clone());
            if request.owner_entity_id != request.target_entity_id {
                self.runtime
                    .push_script_damage_event(request.owner_entity_id, gameplay_event);
            }
        }
    }

    #[cfg(all(
        feature = "subsystem-scripting-csharp",
        feature = "subsystem-physics",
        feature = "subsystem-animation"
    ))]
    fn process_script_ragdoll_requests(&mut self) {
        let pending = self.runtime.take_pending_ragdoll_requests();
        for request in pending {
            match self.set_ragdoll_active(
                &request.target_entity_id,
                request.active,
                request.recovery_duration,
                Vec3::from(request.impulse),
            ) {
                Ok(body_entity_ids) => {
                    let event = engine_script::GameplayRagdollEvent {
                        entity_id: request.target_entity_id.clone(),
                        active: request.active,
                        recovering: !request.active && request.recovery_duration > 0.0,
                        body_entity_ids,
                    };
                    self.runtime
                        .push_script_ragdoll_event(request.target_entity_id.clone(), event.clone());
                    if request.owner_entity_id != request.target_entity_id {
                        self.runtime
                            .push_script_ragdoll_event(request.owner_entity_id, event);
                    }
                }
                Err(error) => {
                    let mut diagnostic = engine_serialize::Diagnostic::new(
                        "SCRIPT_RAGDOLL_REJECTED",
                        engine_serialize::DiagnosticSeverity::Error,
                        "script",
                        format!(
                            "script entity '{}' could not change ragdoll ownership for '{}': {error}",
                            request.owner_entity_id, request.target_entity_id
                        ),
                    );
                    diagnostic.entity = Some(request.owner_entity_id);
                    self.runtime
                        .diagnostics_collector_mut()
                        .push_script_diags(vec![diagnostic]);
                }
            }
        }
    }

    /// Run one validated script physics query against the physics world,
    /// translating backend hits into persistent entity ids so scripts never
    /// observe raw ECS handles.
    #[cfg(all(feature = "subsystem-scripting-csharp", feature = "subsystem-physics"))]
    fn execute_script_physics_query(
        &self,
        query: &engine_script::GameplayPhysicsQuery,
    ) -> engine_script::GameplayPhysicsQueryResult {
        use engine_script::{GameplayPhysicsQuery, GameplayPhysicsQueryResult};

        // Translate the script-side filter into backend terms: the
        // persistent exclude id becomes the ECS entity it names (already
        // validated to exist when the command was applied).
        let query_filter = query
            .filter()
            .map(|filter| engine_physics::PhysicsQueryFilter {
                layer_mask: filter.layer_mask,
                include_sensors: filter.include_sensors,
                exclude_entity: filter.exclude_entity.as_deref().and_then(|persistent_id| {
                    self.runtime
                        .with_world(|world| world.entity_by_persistent_id(persistent_id))
                        .flatten()
                }),
            });
        let query_filter = query_filter.unwrap_or_default();

        /// Translate a backend hit into a script result, reporting a miss
        /// when the hit collider has no persistent id to name.
        fn hit_result(
            hit: Option<engine_physics::RaycastHit>,
            metadata: impl Fn(
                engine_physics::Entity,
                f32,
            )
                -> Option<(String, Option<engine_script::GameplayInteractionSnapshot>)>,
            hit_kind: impl Fn(
                String,
                [f32; 3],
                [f32; 3],
                f32,
                Option<engine_script::GameplayInteractionSnapshot>,
            ) -> GameplayPhysicsQueryResult,
            miss: impl Fn() -> GameplayPhysicsQueryResult,
        ) -> GameplayPhysicsQueryResult {
            let Some(hit) = hit else {
                return miss();
            };
            match metadata(hit.entity, hit.distance) {
                Some((entity_id, interaction)) => hit_kind(
                    entity_id,
                    hit.point.to_array(),
                    hit.normal.to_array(),
                    hit.distance,
                    interaction,
                ),
                // A collider without a persistent id cannot be named to
                // scripts, so the query reports no usable hit.
                None => miss(),
            }
        }

        let hit_metadata = |entity: engine_physics::Entity, distance: f32| {
            self.runtime
                .with_world(|world| {
                    let entity_id = world.persistent_id(entity)?.to_owned();
                    let interaction = world
                        .get::<engine_scene::components::Interactable>(entity)
                        .filter(|interactable| {
                            interactable.enabled && distance <= interactable.max_distance
                        })
                        .map(|interactable| engine_script::GameplayInteractionSnapshot {
                            prompt: interactable.prompt.clone(),
                            action: interactable.action.clone(),
                            max_distance: interactable.max_distance,
                            grabbable: interactable.grabbable,
                        });
                    Some((entity_id, interaction))
                })
                .flatten()
        };

        match *query {
            GameplayPhysicsQuery::Raycast {
                query_id,
                origin,
                direction,
                max_distance,
                ..
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
                let hit = physics.raycast_filtered(
                    Vec3::from(origin),
                    direction,
                    max_distance,
                    &query_filter,
                );
                hit_result(
                    hit,
                    hit_metadata,
                    |entity_id, point, normal, distance, interaction| {
                        GameplayPhysicsQueryResult::RaycastHit {
                            query_id,
                            entity_id,
                            point,
                            normal,
                            distance,
                            interaction,
                        }
                    },
                    miss,
                )
            }
            GameplayPhysicsQuery::SphereCast {
                query_id,
                origin,
                radius,
                direction,
                max_distance,
                ..
            } => {
                let miss = || GameplayPhysicsQueryResult::SphereCastMiss { query_id };
                let Some(physics) = self.physics.as_ref() else {
                    return miss();
                };
                let direction = Vec3::from(direction).normalize_or_zero();
                if direction == Vec3::ZERO {
                    return miss();
                }
                let radius = radius.min(engine_script::MAX_PHYSICS_QUERY_DISTANCE);
                let max_distance = max_distance.min(engine_script::MAX_PHYSICS_QUERY_DISTANCE);
                let hit = physics.cast_shape(
                    &engine_physics::ColliderShape::Ball { radius },
                    Vec3::from(origin),
                    direction,
                    max_distance,
                    &query_filter,
                );
                hit_result(
                    hit,
                    hit_metadata,
                    |entity_id, point, normal, distance, interaction| {
                        GameplayPhysicsQueryResult::SphereCastHit {
                            query_id,
                            entity_id,
                            point,
                            normal,
                            distance,
                            interaction,
                        }
                    },
                    miss,
                )
            }
            GameplayPhysicsQuery::OverlapSphere {
                query_id,
                center,
                radius,
                ..
            } => {
                let mut entity_ids = Vec::new();
                if let Some(physics) = self.physics.as_ref() {
                    let radius = radius.min(engine_script::MAX_PHYSICS_QUERY_DISTANCE);
                    let hits = physics.query_proximity_filtered(
                        &engine_physics::ColliderShape::Ball { radius },
                        Vec3::from(center),
                        &query_filter,
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

    #[cfg(all(feature = "subsystem-scripting-csharp", feature = "subsystem-gameplay"))]
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

#[cfg(all(feature = "subsystem-scripting-csharp", feature = "subsystem-gameplay"))]
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

#[cfg(all(test, feature = "subsystem-physics", feature = "subsystem-gameplay"))]
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

    #[cfg(all(
        feature = "subsystem-animation",
        feature = "subsystem-audio",
        feature = "subsystem-navigation",
        feature = "subsystem-ui"
    ))]
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

    #[cfg(all(
        feature = "subsystem-animation",
        feature = "subsystem-audio",
        feature = "subsystem-navigation",
        feature = "subsystem-ui"
    ))]
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

    #[cfg(all(
        feature = "subsystem-animation",
        feature = "subsystem-audio",
        feature = "subsystem-navigation",
        feature = "subsystem-ui"
    ))]
    #[test]
    fn ragdoll_generates_physics_graph_switches_pose_ownership_and_recovers() {
        use engine_animation::{
            AnimationPlayer, Joint, JointTransform, RagdollBody, RagdollComponent,
            RagdollConstraint, RagdollMode, Skeleton, SkeletonComponent,
        };

        let skeleton = Skeleton {
            joints: vec![
                Joint {
                    name: "hips".into(),
                    parent_index: None,
                    local_transform: JointTransform::IDENTITY,
                },
                Joint {
                    name: "chest".into(),
                    parent_index: Some(0),
                    local_transform: JointTransform {
                        translation: [0.0, 0.75, 0.0],
                        ..JointTransform::IDENTITY
                    },
                },
            ],
            inverse_bind_matrices: vec![
                glam::Mat4::IDENTITY.to_cols_array_2d(),
                glam::Mat4::from_translation(Vec3::new(0.0, -0.75, 0.0)).to_cols_array_2d(),
            ],
        };
        skeleton.validate().unwrap();

        let mut game_loop = GameLoop::new(EngineConfig::default());
        game_loop.load_scene(engine_scene::sample_scene()).unwrap();
        let skeleton_id = engine_serialize::AssetId::new("npc.skeleton");
        game_loop
            .runtime
            .asset_registry_mut()
            .insert_typed(skeleton_id.clone(), skeleton);
        game_loop
            .runtime
            .loaded_extension_asset_ids
            .entry("skeleton".into())
            .or_default()
            .insert(skeleton_id);
        game_loop
            .runtime
            .with_world_mut(|world| {
                let owner = world.entity_by_persistent_id("cube-01").unwrap();
                world.add_component(
                    owner,
                    engine_scene::components::Transform {
                        translation: Vec3::new(0.0, 3.0, 0.0),
                        ..Default::default()
                    },
                );
                world.add_component(owner, SkeletonComponent::new("npc.skeleton"));
                world.add_component(owner, AnimationPlayer::default());
                world.add_component(
                    owner,
                    RagdollComponent {
                        bodies: vec![
                            RagdollBody {
                                bone: "hips".into(),
                                ..Default::default()
                            },
                            RagdollBody {
                                bone: "chest".into(),
                                ..Default::default()
                            },
                        ],
                        constraints: vec![RagdollConstraint {
                            parent_bone: "hips".into(),
                            child_bone: "chest".into(),
                            limits: Some([-0.6, 0.6]),
                            ..Default::default()
                        }],
                        ..Default::default()
                    },
                );
            })
            .unwrap();

        game_loop.update(0.0);
        let (body_ids, joint_ids) = game_loop
            .runtime
            .with_world(|world| {
                let owner = world.entity_by_persistent_id("cube-01").unwrap();
                let ragdoll = world.get::<RagdollComponent>(owner).unwrap();
                (
                    ragdoll
                        .generated_body_ids
                        .values()
                        .cloned()
                        .collect::<Vec<_>>(),
                    ragdoll.generated_joint_ids.clone(),
                )
            })
            .unwrap();
        assert_eq!(body_ids.len(), 2);
        assert_eq!(joint_ids.len(), 1);
        assert_eq!(game_loop.physics.as_ref().unwrap().body_count(), 2);
        assert_eq!(game_loop.physics.as_ref().unwrap().joint_count(), 1);

        assert_eq!(
            game_loop
                .set_ragdoll_active("cube-01", true, 0.0, Vec3::new(4.0, 0.0, 0.0))
                .unwrap(),
            body_ids
        );
        game_loop.update(1.0 / 60.0);
        game_loop
            .runtime
            .with_world(|world| {
                let owner = world.entity_by_persistent_id("cube-01").unwrap();
                let ragdoll = world.get::<RagdollComponent>(owner).unwrap();
                assert_eq!(ragdoll.mode, RagdollMode::Simulated);
                let player = world.get::<AnimationPlayer>(owner).unwrap();
                assert_eq!(
                    player
                        .external_pose_override
                        .as_ref()
                        .expect("physics owns the rendered pose")
                        .weight,
                    1.0
                );
                for id in &body_ids {
                    let body = world.entity_by_persistent_id(id).unwrap();
                    assert_eq!(
                        world
                            .get::<engine_physics::RigidBody>(body)
                            .unwrap()
                            .body_type,
                        engine_physics::BodyType::Dynamic
                    );
                }
            })
            .unwrap();

        let checkpoint = game_loop
            .capture_save_game(std::collections::BTreeMap::new())
            .unwrap();
        game_loop
            .runtime
            .with_world_mut(|world| {
                let first_body = world.entity_by_persistent_id(&body_ids[0]).unwrap();
                assert!(world.destroy_entity(first_body));
                let owner = world.entity_by_persistent_id("cube-01").unwrap();
                world.get_mut::<RagdollComponent>(owner).unwrap().mode = RagdollMode::Animated;
            })
            .unwrap();
        let restore = game_loop.restore_save_game(checkpoint).unwrap();
        assert_eq!(restore.restored_physics_bodies, 2);
        game_loop
            .runtime
            .with_world(|world| {
                let owner = world.entity_by_persistent_id("cube-01").unwrap();
                assert_eq!(
                    world.get::<RagdollComponent>(owner).unwrap().mode,
                    RagdollMode::Simulated
                );
                assert!(body_ids
                    .iter()
                    .all(|id| world.entity_by_persistent_id(id).is_some()));
                assert!(joint_ids
                    .iter()
                    .all(|id| world.entity_by_persistent_id(id).is_some()));
            })
            .unwrap();
        assert_eq!(game_loop.physics.as_ref().unwrap().joint_count(), 1);

        game_loop
            .set_ragdoll_active("cube-01", false, 0.1, Vec3::ZERO)
            .unwrap();
        game_loop.update(0.05);
        let blend_weight = game_loop
            .runtime
            .with_world(|world| {
                let owner = world.entity_by_persistent_id("cube-01").unwrap();
                world
                    .get::<AnimationPlayer>(owner)
                    .unwrap()
                    .external_pose_override
                    .as_ref()
                    .unwrap()
                    .weight
            })
            .unwrap();
        assert!((blend_weight - 0.5).abs() < 1.0e-4, "{blend_weight}");

        game_loop.update(0.05);
        game_loop
            .runtime
            .with_world(|world| {
                let owner = world.entity_by_persistent_id("cube-01").unwrap();
                assert_eq!(
                    world.get::<RagdollComponent>(owner).unwrap().mode,
                    RagdollMode::Animated
                );
                assert!(world
                    .get::<AnimationPlayer>(owner)
                    .unwrap()
                    .external_pose_override
                    .is_none());
            })
            .unwrap();
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

    #[cfg(all(
        feature = "subsystem-animation",
        feature = "subsystem-audio",
        feature = "subsystem-navigation",
        feature = "subsystem-ui"
    ))]
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

#[cfg(all(
    test,
    feature = "subsystem-physics",
    feature = "subsystem-gameplay",
    feature = "subsystem-scripting-csharp"
))]
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
                joint_id: None,
                force: None,
                torque: None,
            }])
        );
        assert_eq!(
            events.get("camera-main"),
            Some(&vec![GameplayPhysicsEvent {
                kind: GameplayPhysicsEventKind::CollisionEntered,
                other_entity_id: "cube-01".into(),
                joint_id: None,
                force: None,
                torque: None,
            }])
        );
    }

    #[test]
    fn joint_breaks_reach_both_script_bodies_with_constraint_and_load() {
        use engine_physics::{JointBreakEvent, JointHandle};
        use engine_script::{GameplayPhysicsEvent, GameplayPhysicsEventKind};

        let mut game_loop = GameLoop::new(EngineConfig::default());
        game_loop.load_scene(engine_scene::sample_scene()).unwrap();
        let (cube, camera, constraint) = game_loop
            .runtime
            .with_world_mut(|world| {
                (
                    world.entity_by_persistent_id("cube-01").unwrap(),
                    world.entity_by_persistent_id("camera-main").unwrap(),
                    world.create_persistent_entity("cube-tether").unwrap(),
                )
            })
            .unwrap();
        game_loop.physics_events.joint_breaks.push(JointBreakEvent {
            handle: JointHandle(7),
            joint_entity: Some(constraint),
            entity_a: cube,
            entity_b: camera,
            force: 1250.0,
            torque: 75.0,
        });

        let events = game_loop.resolved_script_physics_events();
        let expected_for_cube = GameplayPhysicsEvent {
            kind: GameplayPhysicsEventKind::JointBroken,
            other_entity_id: "camera-main".into(),
            joint_id: Some("cube-tether".into()),
            force: Some(1250.0),
            torque: Some(75.0),
        };
        let expected_for_camera = GameplayPhysicsEvent {
            other_entity_id: "cube-01".into(),
            ..expected_for_cube.clone()
        };
        assert_eq!(events.get("cube-01"), Some(&vec![expected_for_cube]));
        assert_eq!(events.get("camera-main"), Some(&vec![expected_for_camera]));
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
            joint_id: None,
            force: None,
            torque: None,
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
                        filter: None,
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
                        filter: None,
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
            "engine.interactable".into(),
            engine_scene::ComponentRecord {
                schema_version: SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: BTreeMap::from([
                    ("prompt".into(), Value::Str("Pick up cube".into())),
                    ("action".into(), Value::Str("pickup".into())),
                    ("max_distance".into(), Value::Float32(5.0)),
                    ("grabbable".into(), Value::Bool(true)),
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
                    interaction,
                } => Some((
                    entity_id.clone(),
                    *point,
                    *normal,
                    *distance,
                    interaction.clone(),
                )),
                _ => None,
            })
            .expect("raycast hit result for query 11");
        assert_eq!(hit.0, "cube-01");
        assert!((hit.1[1] - 0.5).abs() < 1.0e-4, "hit point: {:?}", hit.1);
        let interaction = hit.4.expect("enabled in-range interactable metadata");
        assert_eq!(interaction.prompt, "Pick up cube");
        assert_eq!(interaction.action, "pickup");
        assert!(interaction.grabbable);
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
                    filter: None,
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
        assert!(contexts.lock().unwrap().iter().all(|context| {
            context.physics_query_results.iter().all(|result| {
                !matches!(
                    result,
                    GameplayPhysicsQueryResult::RaycastHit { query_id: 1, .. }
                )
            })
        }));
    }

    #[test]
    fn physics_queries_with_unknown_exclude_entity_are_rejected() {
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

        let unknown_exclusion = |query_id| engine_script::OwnedGameplayCommand {
            entity_id: "cube-01".into(),
            command: GameplayCommand::PhysicsQuery {
                query: engine_script::GameplayPhysicsQuery::Raycast {
                    query_id,
                    origin: [0.0, 5.0, 0.0],
                    direction: [0.0, -1.0, 0.0],
                    max_distance: 10.0,
                    filter: Some(engine_script::GameplayPhysicsQueryFilter {
                        layer_mask: None,
                        include_sensors: false,
                        exclude_entity: Some("ghost-entity".into()),
                    }),
                },
            },
        };
        let diagnostics = game_loop
            .runtime
            .apply_script_gameplay_commands(vec![unknown_exclusion(1)]);
        assert!(
            diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "SCRIPT_PHYSICS_QUERY_INVALID"
                    && diagnostic.message.contains("ghost-entity")
            }),
            "unknown exclude_entity id should be a validation error: {diagnostics:?}"
        );
        assert!(game_loop.runtime.take_pending_physics_queries().is_empty());

        // A known exclusion target passes validation and is queued.
        let self_exclusion = engine_script::OwnedGameplayCommand {
            entity_id: "cube-01".into(),
            command: GameplayCommand::PhysicsQuery {
                query: engine_script::GameplayPhysicsQuery::Raycast {
                    query_id: 2,
                    origin: [0.0, 5.0, 0.0],
                    direction: [0.0, -1.0, 0.0],
                    max_distance: 10.0,
                    filter: Some(engine_script::GameplayPhysicsQueryFilter {
                        layer_mask: None,
                        include_sensors: false,
                        exclude_entity: Some("cube-01".into()),
                    }),
                },
            },
        };
        let diagnostics = game_loop
            .runtime
            .apply_script_gameplay_commands(vec![self_exclusion]);
        assert!(
            diagnostics.is_empty(),
            "known exclude_entity id should validate: {diagnostics:?}"
        );
        assert_eq!(game_loop.runtime.take_pending_physics_queries().len(), 1);
    }

    #[test]
    fn script_impulse_resolves_persistent_id_and_reaches_physics_step() {
        use engine_physics::{Collider, RigidBody};
        use engine_scene::components::Transform;

        let mut game_loop = GameLoop::new(EngineConfig::default());
        game_loop.load_scene(engine_scene::sample_scene()).unwrap();
        game_loop.runtime.with_world_mut(|world| {
            let cube = world.entity_by_persistent_id("cube-01").unwrap();
            world.add_component(cube, Transform::default());
            world.add_component(cube, RigidBody::default());
            world.add_component(cube, Collider::default());
        });
        game_loop.init_physics();

        let diagnostics = game_loop.runtime.apply_script_gameplay_commands(vec![
            engine_script::OwnedGameplayCommand {
                entity_id: "cube-01".into(),
                command: GameplayCommand::PhysicsMutation {
                    mutation: engine_script::GameplayPhysicsMutation::ApplyImpulse {
                        entity_id: "cube-01".into(),
                        impulse: [12.0, 0.0, 0.0],
                    },
                },
            },
        ]);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        game_loop.queue_script_physics_mutations();
        game_loop.update(1.0 / 60.0);

        let cube = game_loop
            .runtime
            .with_world(|world| world.entity_by_persistent_id("cube-01").unwrap())
            .unwrap();
        let state = game_loop
            .physics
            .as_ref()
            .unwrap()
            .runtime_body_states()
            .into_iter()
            .find(|(entity, _)| *entity == cube)
            .unwrap()
            .1;
        assert!(state.linear_velocity[0] > 0.0, "{state:?}");
    }

    #[test]
    fn script_joint_mutations_create_update_and_remove_a_persistent_constraint() {
        use engine_physics::{BodyType, Collider, PhysicsJoint, RigidBody};
        use engine_scene::components::Transform;
        use engine_script::{GameplayJointLimits, GameplayJointType, GameplayPhysicsMutation};

        let mut game_loop = GameLoop::new(EngineConfig::default());
        game_loop.load_scene(engine_scene::sample_scene()).unwrap();
        game_loop.runtime.with_world_mut(|world| {
            let cube = world.entity_by_persistent_id("cube-01").unwrap();
            let camera = world.entity_by_persistent_id("camera-main").unwrap();
            world.add_component(cube, Transform::default());
            world.add_component(cube, RigidBody::default());
            world.add_component(cube, Collider::default());
            world.add_component(camera, Transform::default());
            world.add_component(
                camera,
                RigidBody {
                    body_type: BodyType::Static,
                    ..RigidBody::default()
                },
            );
        });
        game_loop.init_physics();

        let create = |max: f32, break_force: f32| GameplayCommand::PhysicsMutation {
            mutation: GameplayPhysicsMutation::CreateJoint {
                joint_id: "script-hinge".into(),
                body_a: "camera-main".into(),
                body_b: "cube-01".into(),
                joint_type: GameplayJointType::Revolute,
                anchor_a: [0.0; 3],
                anchor_b: [0.0; 3],
                axis: [0.0, 1.0, 0.0],
                limits: Some(GameplayJointLimits {
                    min: -max,
                    max,
                    stiffness: 20.0,
                    damping: 2.0,
                }),
                motor: None,
                break_force,
                break_torque: 0.0,
            },
        };

        for command in [create(1.0, 1000.0), create(0.5, 500.0)] {
            let diagnostics = game_loop.runtime.apply_script_gameplay_commands(vec![
                engine_script::OwnedGameplayCommand {
                    entity_id: "cube-01".into(),
                    command,
                },
            ]);
            assert!(diagnostics.is_empty(), "{diagnostics:?}");
            game_loop.queue_script_physics_mutations();
            game_loop.update(0.0);
            assert_eq!(game_loop.physics.as_ref().unwrap().joint_count(), 1);
        }

        game_loop
            .runtime
            .with_world(|world| {
                let constraint = world.entity_by_persistent_id("script-hinge").unwrap();
                let joint = world.get::<PhysicsJoint>(constraint).unwrap();
                assert_eq!(joint.break_force, 500.0);
                assert_eq!(joint.limits.as_ref().unwrap().max, 0.5);
            })
            .unwrap();

        let diagnostics = game_loop.runtime.apply_script_gameplay_commands(vec![
            engine_script::OwnedGameplayCommand {
                entity_id: "cube-01".into(),
                command: GameplayCommand::PhysicsMutation {
                    mutation: GameplayPhysicsMutation::RemoveJoint {
                        joint_id: "script-hinge".into(),
                    },
                },
            },
        ]);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        game_loop.queue_script_physics_mutations();
        game_loop.update(0.0);
        assert_eq!(game_loop.physics.as_ref().unwrap().joint_count(), 0);
        game_loop
            .runtime
            .with_world(|world| {
                let constraint = world.entity_by_persistent_id("script-hinge").unwrap();
                assert!(world.get::<PhysicsJoint>(constraint).is_none());
            })
            .unwrap();
    }

    // ── Sweep / filter fixtures ─────────────────────────────────────────

    /// Script instance that issues a fixed, pre-built command batch once.
    struct ScriptedQueriesInstance {
        contexts: Arc<std::sync::Mutex<Vec<GameplayContext>>>,
        commands: Vec<GameplayCommand>,
    }

    impl ScriptInstance for ScriptedQueriesInstance {
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

        fn drain_gameplay_commands(&mut self) -> Result<Vec<GameplayCommand>, ScriptError> {
            Ok(std::mem::take(&mut self.commands))
        }
    }

    struct ScriptedQueriesHost {
        contexts: Arc<std::sync::Mutex<Vec<GameplayContext>>>,
        commands: Vec<GameplayCommand>,
    }

    impl ScriptHost for ScriptedQueriesHost {
        fn name(&self) -> &str {
            "scripted-queries"
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
            Ok(Box::new(ScriptedQueriesInstance {
                contexts: Arc::clone(&self.contexts),
                commands: self.commands.clone(),
            }))
        }

        fn unload(&mut self, _handle: &ScriptHandle) -> Result<(), ScriptError> {
            Ok(())
        }
    }

    fn component_record(fields: BTreeMap<String, Value>) -> engine_scene::ComponentRecord {
        engine_scene::ComponentRecord {
            schema_version: SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields,
        }
    }

    fn register_damage_test_prefab(game_loop: &mut GameLoop, prefab_id: &str) {
        let mut prefab = engine_scene::Prefab::new(engine_serialize::AssetId::new(prefab_id));
        prefab.add_entity(engine_scene::EntityRecord {
            persistent_id: "root".into(),
            parent: None,
            name: Some("Fracture piece".into()),
            enabled: true,
            components: BTreeMap::from([
                ("engine.transform".into(), component_record(BTreeMap::new())),
                (
                    "engine.physics.rigid_body".into(),
                    component_record(BTreeMap::new()),
                ),
                (
                    "engine.physics.collider".into(),
                    component_record(BTreeMap::new()),
                ),
            ]),
        });
        let asset_id = engine_serialize::AssetId::new(prefab_id);
        game_loop
            .runtime
            .asset_registry_mut()
            .insert_typed(asset_id.clone(), prefab);
        game_loop
            .runtime
            .loaded_extension_asset_ids
            .entry("prefab".into())
            .or_default()
            .insert(asset_id);
    }

    fn damage_command(
        owner: &str,
        target: &str,
        amount: f32,
        impulse: [f32; 3],
    ) -> engine_script::OwnedGameplayCommand {
        engine_script::OwnedGameplayCommand {
            entity_id: owner.into(),
            command: GameplayCommand::ApplyDamage {
                entity_id: target.into(),
                amount,
                damage_kind: engine_script::GameplayDamageKind::Impact,
                hit_position: Some([3.0, 4.0, 5.0]),
                impulse,
            },
        }
    }

    #[test]
    fn script_damage_breaks_once_replaces_prefab_and_inherits_physics_state() {
        let mut game_loop = GameLoop::new(EngineConfig::default());
        game_loop.load_scene(engine_scene::sample_scene()).unwrap();
        let target = game_loop
            .runtime
            .with_world_mut(|world| {
                let target = world.entity_by_persistent_id("cube-01").unwrap();
                world.add_component(
                    target,
                    engine_scene::components::Transform {
                        translation: Vec3::new(3.0, 4.0, 5.0),
                        ..Default::default()
                    },
                );
                world.add_component(target, engine_physics::RigidBody::default());
                world.add_component(target, engine_physics::Collider::default());
                world.add_component(
                    target,
                    engine_physics::Destructible {
                        max_health: 10.0,
                        health: 10.0,
                        replacement_prefab: Some(engine_serialize::AssetId::new("crate-fracture")),
                        fracture_impulse_scale: 0.5,
                        ..Default::default()
                    },
                );
                target
            })
            .unwrap();
        game_loop.resync_physics_from_world();
        let source_state = engine_physics::RigidBodyRuntimeState {
            position: [3.0, 4.0, 5.0],
            rotation: glam::Quat::IDENTITY.to_array(),
            linear_velocity: [2.0, 3.0, 4.0],
            angular_velocity: [0.0, 1.5, 0.0],
            sleeping: false,
        };
        assert!(game_loop
            .physics
            .as_mut()
            .unwrap()
            .restore_runtime_body_state(target, &source_state));
        register_damage_test_prefab(&mut game_loop, "crate-fracture");

        let diagnostics = game_loop
            .runtime
            .apply_script_gameplay_commands(vec![damage_command(
                "camera-main",
                "cube-01",
                10.0,
                [6.0, 0.0, 0.0],
            )]);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        game_loop.process_script_damage_requests();

        let fragment = game_loop
            .runtime
            .with_world(|world| {
                assert!(world.entity_by_persistent_id("cube-01").is_none());
                let fragment = world
                    .entity_by_persistent_id("crate-fracture")
                    .expect("replacement prefab root");
                assert_eq!(
                    world
                        .get::<engine_scene::components::Transform>(fragment)
                        .unwrap()
                        .translation,
                    Vec3::new(3.0, 4.0, 5.0)
                );
                fragment
            })
            .unwrap();
        let delivered = &game_loop.runtime.script_damage_events["camera-main"];
        assert_eq!(delivered.len(), 1);
        assert!(delivered[0].broke);
        assert_eq!(
            delivered[0].spawned_entity_ids,
            vec!["crate-fracture".to_string()]
        );

        {
            let physics = game_loop.physics.as_mut().unwrap();
            game_loop
                .runtime
                .with_world_mut(|world| physics.step(0.0, world))
                .unwrap();
        }
        let fragment_state = game_loop
            .physics
            .as_ref()
            .unwrap()
            .runtime_body_states()
            .into_iter()
            .find_map(|(entity, state)| (entity == fragment).then_some(state))
            .expect("fracture piece body state");
        assert!(fragment_state.linear_velocity[0] > source_state.linear_velocity[0]);
        assert_eq!(
            fragment_state.linear_velocity[1],
            source_state.linear_velocity[1]
        );
        assert_eq!(
            fragment_state.linear_velocity[2],
            source_state.linear_velocity[2]
        );
        assert_eq!(
            fragment_state.angular_velocity,
            source_state.angular_velocity
        );
    }

    #[test]
    fn failed_fracture_prefab_does_not_delete_the_broken_source() {
        let mut game_loop = GameLoop::new(EngineConfig::default());
        game_loop.load_scene(engine_scene::sample_scene()).unwrap();
        game_loop
            .runtime
            .with_world_mut(|world| {
                let target = world.entity_by_persistent_id("cube-01").unwrap();
                world.add_component(
                    target,
                    engine_physics::Destructible {
                        max_health: 1.0,
                        health: 1.0,
                        replacement_prefab: Some(engine_serialize::AssetId::new(
                            "missing-fracture",
                        )),
                        ..Default::default()
                    },
                );
            })
            .unwrap();

        assert!(game_loop
            .runtime
            .apply_script_gameplay_commands(vec![damage_command(
                "camera-main",
                "cube-01",
                1.0,
                [0.0; 3],
            )])
            .is_empty());
        game_loop.process_script_damage_requests();

        game_loop
            .runtime
            .with_world(|world| {
                let target = world
                    .entity_by_persistent_id("cube-01")
                    .expect("failed replacement keeps source entity");
                let destructible = world.get::<engine_physics::Destructible>(target).unwrap();
                assert!(destructible.broken);
                assert_eq!(destructible.health, 0.0);
            })
            .unwrap();
        assert!(game_loop
            .runtime
            .diagnostics_collector()
            .script_diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SCRIPT_PREFAB_UNKNOWN"));
    }

    /// A static box entity at `translation` with a collider on
    /// `collision_group`, optionally a sensor.
    fn physics_box_entity(
        persistent_id: &str,
        translation: [f32; 3],
        collision_group: u32,
        is_trigger: bool,
    ) -> engine_scene::EntityRecord {
        engine_scene::EntityRecord {
            persistent_id: persistent_id.into(),
            parent: None,
            name: None,
            enabled: true,
            components: BTreeMap::from([
                (
                    "engine.transform".into(),
                    component_record(BTreeMap::from([(
                        "translation".into(),
                        Value::Vec3(translation),
                    )])),
                ),
                (
                    "engine.physics.rigid_body".into(),
                    component_record(BTreeMap::from([(
                        "body_type".into(),
                        Value::Enum("Static".into()),
                    )])),
                ),
                (
                    "engine.physics.collider".into(),
                    component_record(BTreeMap::from([
                        (
                            "collision_group".into(),
                            Value::UInt(u64::from(collision_group)),
                        ),
                        ("is_trigger".into(), Value::Bool(is_trigger)),
                    ])),
                ),
            ]),
        }
    }

    /// A game loop whose script entity owns a layer-1 collider, alongside a
    /// layer-2 box (`cube-02` at y = -4) and a sensor (`sensor-01` at
    /// y = -8). `commands` are issued by the script on the first frame.
    fn filtered_query_game_loop(
        contexts: &Arc<std::sync::Mutex<Vec<GameplayContext>>>,
        commands: Vec<GameplayCommand>,
    ) -> GameLoop {
        let mut game_loop = GameLoop::new(EngineConfig::default());
        game_loop
            .runtime
            .register_script_host(Box::new(ScriptedQueriesHost {
                contexts: Arc::clone(contexts),
                commands,
            }));
        game_loop.runtime.set_script_host_name("scripted-queries");
        game_loop
            .runtime
            .load_script_assembly("game", "scripted-queries", b"test")
            .unwrap();

        let mut scene = engine_scene::sample_scene();
        let target = scene
            .entities
            .iter_mut()
            .find(|entity| entity.persistent_id == "cube-01")
            .unwrap();
        target
            .components
            .insert("engine.transform".into(), component_record(BTreeMap::new()));
        target.components.insert(
            "engine.physics.rigid_body".into(),
            component_record(BTreeMap::from([(
                "body_type".into(),
                Value::Enum("Static".into()),
            )])),
        );
        target.components.insert(
            "engine.physics.collider".into(),
            component_record(BTreeMap::from([("collision_group".into(), Value::UInt(1))])),
        );
        target.components.insert(
            "engine.script".into(),
            component_record(BTreeMap::from([
                ("assembly_id".into(), Value::Str("game".into())),
                ("class_name".into(), Value::Str("Probe".into())),
            ])),
        );
        scene
            .entities
            .push(physics_box_entity("cube-02", [0.0, -4.0, 0.0], 2, false));
        scene.entities.push(physics_box_entity(
            "sensor-01",
            [0.0, -8.0, 0.0],
            0xFFFF_FFFF,
            true,
        ));
        game_loop.load_scene(scene).unwrap();
        game_loop
    }

    /// Run two frames and return the contexts from each.
    fn two_frames(
        game_loop: &mut GameLoop,
        contexts: &Arc<std::sync::Mutex<Vec<GameplayContext>>>,
    ) -> (GameplayContext, GameplayContext) {
        game_loop.update(1.0 / 60.0);
        let first = contexts.lock().unwrap().last().unwrap().clone();
        game_loop.update(1.0 / 60.0);
        let second = contexts.lock().unwrap().last().unwrap().clone();
        (first, second)
    }

    #[test]
    fn sphere_cast_reports_hit_miss_and_normal_in_the_next_frame() {
        use engine_script::GameplayPhysicsQueryResult;

        let contexts = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sphere_cast = |query_id, direction| GameplayCommand::PhysicsQuery {
            query: engine_script::GameplayPhysicsQuery::SphereCast {
                query_id,
                origin: [0.0, 5.0, 0.0],
                radius: 0.5,
                direction,
                max_distance: 10.0,
                filter: None,
            },
        };
        let mut game_loop = filtered_query_game_loop(
            &contexts,
            vec![
                sphere_cast(21, [0.0, -1.0, 0.0]),
                sphere_cast(22, [0.0, 1.0, 0.0]),
            ],
        );

        let (first, second) = two_frames(&mut game_loop, &contexts);
        assert!(first.physics_query_results.is_empty());
        assert_eq!(second.physics_query_results.len(), 2);

        let hit = second
            .physics_query_results
            .iter()
            .find_map(|result| match result {
                GameplayPhysicsQueryResult::SphereCastHit {
                    query_id: 21,
                    entity_id,
                    point,
                    normal,
                    distance,
                    ..
                } => Some((entity_id.clone(), *point, *normal, *distance)),
                _ => None,
            })
            .expect("sphere cast hit result for query 21");
        assert_eq!(hit.0, "cube-01");
        // The sphere surface touches the cube's top face once its centre
        // reaches y = 1.0: 4.0 units of travel from y = 5.
        assert!((hit.3 - 4.0).abs() < 1.0e-4, "hit distance: {}", hit.3);
        assert!(
            (hit.1[1] - 0.5).abs() < 5.0e-3,
            "contact point should sit on the top face (GJK/EPA tolerance): {:?}",
            hit.1
        );
        assert!(
            (hit.2[1] - 1.0).abs() < 1.0e-4 && hit.2[0].abs() < 1.0e-4 && hit.2[2].abs() < 1.0e-4,
            "hit normal: {:?}",
            hit.2
        );

        assert!(second.physics_query_results.iter().any(|result| matches!(
            result,
            GameplayPhysicsQueryResult::SphereCastMiss { query_id: 22 }
        )));
        assert!(game_loop
            .runtime
            .diagnostics_collector()
            .script_diagnostics
            .is_empty());
    }

    #[test]
    fn physics_queries_respect_layer_masks() {
        use engine_script::GameplayPhysicsQueryResult;

        let layer_filter = |mask| {
            Some(engine_script::GameplayPhysicsQueryFilter {
                layer_mask: Some(mask),
                include_sensors: false,
                exclude_entity: None,
            })
        };
        let raycast = |query_id, mask| GameplayCommand::PhysicsQuery {
            query: engine_script::GameplayPhysicsQuery::Raycast {
                query_id,
                origin: [0.0, 5.0, 0.0],
                direction: [0.0, -1.0, 0.0],
                max_distance: 20.0,
                filter: layer_filter(mask),
            },
        };
        let overlap = |query_id, mask| GameplayCommand::PhysicsQuery {
            query: engine_script::GameplayPhysicsQuery::OverlapSphere {
                query_id,
                center: [0.0, -4.0, 0.0],
                radius: 1.0,
                filter: layer_filter(mask),
            },
        };

        let contexts = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut game_loop = filtered_query_game_loop(
            &contexts,
            vec![
                raycast(31, 1),
                raycast(32, 2),
                overlap(33, 1),
                overlap(34, 2),
            ],
        );
        let (_, second) = two_frames(&mut game_loop, &contexts);

        let hit_entity = |query_id| {
            second
                .physics_query_results
                .iter()
                .find_map(|result| match result {
                    GameplayPhysicsQueryResult::RaycastHit {
                        query_id: id,
                        entity_id,
                        ..
                    } if *id == query_id => Some(entity_id.clone()),
                    _ => None,
                })
        };
        // cube-01 sits on layer bit 1, cube-02 on layer bit 2.
        assert_eq!(hit_entity(31).as_deref(), Some("cube-01"));
        assert_eq!(hit_entity(32).as_deref(), Some("cube-02"));

        let overlap_ids = |query_id| {
            second
                .physics_query_results
                .iter()
                .find_map(|result| match result {
                    GameplayPhysicsQueryResult::OverlapSphere {
                        query_id: id,
                        entity_ids,
                    } if *id == query_id => Some(entity_ids.clone()),
                    _ => None,
                })
                .expect("overlap result")
        };
        assert!(overlap_ids(33).is_empty());
        assert_eq!(overlap_ids(34), vec!["cube-02".to_string()]);
    }

    #[test]
    fn physics_queries_exclude_sensors_unless_opted_in() {
        use engine_script::GameplayPhysicsQueryResult;

        let overlap = |query_id, include_sensors| GameplayCommand::PhysicsQuery {
            query: engine_script::GameplayPhysicsQuery::OverlapSphere {
                query_id,
                center: [0.0, -8.0, 0.0],
                radius: 1.0,
                filter: Some(engine_script::GameplayPhysicsQueryFilter {
                    layer_mask: None,
                    include_sensors,
                    exclude_entity: None,
                }),
            },
        };
        let raycast = |query_id, include_sensors| GameplayCommand::PhysicsQuery {
            query: engine_script::GameplayPhysicsQuery::Raycast {
                query_id,
                origin: [0.0, -6.0, 0.0],
                direction: [0.0, -1.0, 0.0],
                max_distance: 4.0,
                filter: Some(engine_script::GameplayPhysicsQueryFilter {
                    layer_mask: None,
                    include_sensors,
                    exclude_entity: None,
                }),
            },
        };

        let contexts = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut game_loop = filtered_query_game_loop(
            &contexts,
            vec![
                overlap(41, false),
                overlap(42, true),
                raycast(43, false),
                raycast(44, true),
            ],
        );
        let (_, second) = two_frames(&mut game_loop, &contexts);

        let overlap_ids = |query_id| {
            second
                .physics_query_results
                .iter()
                .find_map(|result| match result {
                    GameplayPhysicsQueryResult::OverlapSphere {
                        query_id: id,
                        entity_ids,
                    } if *id == query_id => Some(entity_ids.clone()),
                    _ => None,
                })
                .expect("overlap result")
        };
        assert!(
            overlap_ids(41).is_empty(),
            "sensors stay invisible by default"
        );
        assert_eq!(overlap_ids(42), vec!["sensor-01".to_string()]);

        assert!(second.physics_query_results.iter().any(|result| matches!(
            result,
            GameplayPhysicsQueryResult::RaycastMiss { query_id: 43 }
        )));
        assert!(second.physics_query_results.iter().any(|result| matches!(
            result,
            GameplayPhysicsQueryResult::RaycastHit { query_id: 44, entity_id, .. }
                if entity_id == "sensor-01"
        )));
    }

    #[test]
    fn physics_queries_respect_exclude_entity() {
        use engine_script::GameplayPhysicsQueryResult;

        let self_filter = || {
            Some(engine_script::GameplayPhysicsQueryFilter {
                layer_mask: None,
                include_sensors: false,
                exclude_entity: Some("cube-01".into()),
            })
        };
        let commands = vec![
            GameplayCommand::PhysicsQuery {
                query: engine_script::GameplayPhysicsQuery::Raycast {
                    query_id: 51,
                    origin: [0.0, 5.0, 0.0],
                    direction: [0.0, -1.0, 0.0],
                    max_distance: 20.0,
                    filter: self_filter(),
                },
            },
            GameplayCommand::PhysicsQuery {
                query: engine_script::GameplayPhysicsQuery::OverlapSphere {
                    query_id: 52,
                    center: [0.0, 0.0, 0.0],
                    radius: 6.0,
                    filter: self_filter(),
                },
            },
        ];

        let contexts = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut game_loop = filtered_query_game_loop(&contexts, commands);
        let (_, second) = two_frames(&mut game_loop, &contexts);

        // With cube-01 excluded, the ray passes through to cube-02 (top face
        // at y = -3.5, 8.5 units below the origin).
        let hit = second
            .physics_query_results
            .iter()
            .find_map(|result| match result {
                GameplayPhysicsQueryResult::RaycastHit {
                    query_id: 51,
                    entity_id,
                    distance,
                    ..
                } => Some((entity_id.clone(), *distance)),
                _ => None,
            })
            .expect("raycast hit result for query 51");
        assert_eq!(hit.0, "cube-02");
        assert!((hit.1 - 8.5).abs() < 1.0e-4, "hit distance: {}", hit.1);

        assert!(second.physics_query_results.iter().any(|result| matches!(
            result,
            GameplayPhysicsQueryResult::OverlapSphere { query_id: 52, entity_ids }
                if entity_ids == &vec!["cube-02".to_string()]
        )));
        assert!(game_loop
            .runtime
            .diagnostics_collector()
            .script_diagnostics
            .is_empty());
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

    #[cfg(all(feature = "subsystem-physics", feature = "subsystem-gameplay"))]
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

    #[cfg(all(
        feature = "subsystem-animation",
        feature = "subsystem-audio",
        feature = "subsystem-navigation",
        feature = "subsystem-ui"
    ))]
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

#[cfg(test)]
mod savegame_tests {
    use std::collections::BTreeMap;

    use engine_scene::components::Transform;
    use engine_serialize::Value;

    use super::*;

    #[test]
    fn checkpoint_restores_live_scene_origin_and_project_state() {
        let mut game_loop = GameLoop::new(EngineConfig::default());
        game_loop.load_scene(engine_scene::sample_scene()).unwrap();
        game_loop
            .runtime
            .with_world_mut(|world| {
                let cube = world.entity_by_persistent_id("cube-01").unwrap();
                world.add_component(
                    cube,
                    Transform {
                        translation: Vec3::new(1010.0, 2.0, 3.0),
                        ..Transform::default()
                    },
                );
            })
            .unwrap();
        game_loop
            .shift_world_origin([1000.0, 0.0, 0.0])
            .expect("origin shift");
        let expected_relative = game_loop
            .runtime
            .with_world(|world| {
                let cube = world.entity_by_persistent_id("cube-01").unwrap();
                world.get::<Transform>(cube).unwrap().translation
            })
            .unwrap();
        let save = game_loop
            .capture_save_game(BTreeMap::from([
                ("chapter".into(), Value::UInt(4)),
                ("suit".into(), Value::Bool(true)),
            ]))
            .unwrap();

        game_loop
            .runtime
            .with_world_mut(|world| {
                let cube = world.entity_by_persistent_id("cube-01").unwrap();
                world.get_mut::<Transform>(cube).unwrap().translation = Vec3::splat(-99.0);
            })
            .unwrap();
        let report = game_loop.restore_save_game(save).unwrap();

        assert_eq!(game_loop.world_origin(), [1000.0, 0.0, 0.0]);
        assert_eq!(
            game_loop
                .runtime
                .with_world(|world| {
                    let cube = world.entity_by_persistent_id("cube-01").unwrap();
                    world.get::<Transform>(cube).unwrap().translation
                })
                .unwrap(),
            expected_relative
        );
        assert_eq!(report.custom_state["chapter"], Value::UInt(4));
        assert_eq!(report.custom_state["suit"], Value::Bool(true));
    }

    #[cfg(all(feature = "subsystem-physics", feature = "subsystem-gameplay"))]
    #[test]
    fn checkpoint_restores_transient_rigid_body_state_by_persistent_id() {
        let mut game_loop = GameLoop::new(EngineConfig::default());
        game_loop.load_scene(engine_scene::sample_scene()).unwrap();
        let cube = game_loop
            .runtime
            .with_world_mut(|world| {
                let cube = world.entity_by_persistent_id("cube-01").unwrap();
                world.add_component(cube, Transform::default());
                world.add_component(cube, engine_physics::RigidBody::default());
                world.add_component(cube, engine_physics::Collider::default());
                cube
            })
            .unwrap();
        game_loop.resync_physics_from_world();
        let expected = engine_physics::RigidBodyRuntimeState {
            position: [2.0, 3.0, 4.0],
            rotation: glam::Quat::from_rotation_y(0.5).to_array(),
            linear_velocity: [5.0, -1.0, 0.5],
            angular_velocity: [0.0, 2.0, 0.0],
            sleeping: false,
        };
        assert!(game_loop
            .physics
            .as_mut()
            .unwrap()
            .restore_runtime_body_state(cube, &expected));

        let save = game_loop.capture_save_game(BTreeMap::new()).unwrap();
        let report = game_loop.restore_save_game(save).unwrap();
        assert_eq!(report.restored_physics_bodies, 1);
        assert!(report.skipped_physics_bodies.is_empty());

        let restored = game_loop
            .physics
            .as_ref()
            .unwrap()
            .runtime_body_states()
            .into_iter()
            .find(|(entity, _)| {
                game_loop
                    .runtime
                    .with_world(|world| world.persistent_id(*entity) == Some("cube-01"))
                    == Some(true)
            })
            .expect("restored cube state")
            .1;
        assert_eq!(restored.linear_velocity, expected.linear_velocity);
        assert_eq!(restored.angular_velocity, expected.angular_velocity);
        assert_eq!(restored.position, expected.position);
    }

    #[cfg(all(feature = "subsystem-physics", feature = "subsystem-gameplay"))]
    #[test]
    fn checkpoint_rebuilds_persistent_joint_without_serializing_backend_handles() {
        use engine_physics::{BodyType, PhysicsJoint, RigidBody};

        let mut game_loop = GameLoop::new(EngineConfig::default());
        game_loop.load_scene(engine_scene::sample_scene()).unwrap();
        game_loop
            .runtime
            .with_world_mut(|world| {
                let cube = world.entity_by_persistent_id("cube-01").unwrap();
                let camera = world.entity_by_persistent_id("camera-main").unwrap();
                world.add_component(cube, Transform::default());
                world.add_component(cube, RigidBody::default());
                world.add_component(camera, Transform::default());
                world.add_component(
                    camera,
                    RigidBody {
                        body_type: BodyType::Static,
                        ..RigidBody::default()
                    },
                );
                let constraint = world.create_persistent_entity("save-tether").unwrap();
                world.add_component(
                    constraint,
                    PhysicsJoint {
                        body_a: "camera-main".into(),
                        body_b: "cube-01".into(),
                        break_force: 2500.0,
                        ..PhysicsJoint::default()
                    },
                );
            })
            .unwrap();
        game_loop.resync_physics_from_world();
        assert_eq!(game_loop.physics.as_ref().unwrap().joint_count(), 1);

        let save = game_loop.capture_save_game(BTreeMap::new()).unwrap();
        game_loop
            .runtime
            .with_world_mut(|world| {
                let constraint = world.entity_by_persistent_id("save-tether").unwrap();
                world.remove_component::<PhysicsJoint>(constraint);
            })
            .unwrap();
        game_loop.resync_physics_from_world();
        assert_eq!(game_loop.physics.as_ref().unwrap().joint_count(), 0);

        game_loop.restore_save_game(save).unwrap();
        assert_eq!(game_loop.physics.as_ref().unwrap().joint_count(), 1);
        game_loop
            .runtime
            .with_world(|world| {
                let constraint = world.entity_by_persistent_id("save-tether").unwrap();
                assert_eq!(
                    world.get::<PhysicsJoint>(constraint).unwrap().break_force,
                    2500.0
                );
            })
            .unwrap();
    }

    #[cfg(all(feature = "subsystem-physics", feature = "subsystem-gameplay"))]
    #[test]
    fn checkpoint_restores_destructible_health_and_break_state() {
        let mut game_loop = GameLoop::new(EngineConfig::default());
        game_loop.load_scene(engine_scene::sample_scene()).unwrap();
        game_loop
            .runtime
            .with_world_mut(|world| {
                let cube = world.entity_by_persistent_id("cube-01").unwrap();
                world.add_component(
                    cube,
                    engine_physics::Destructible {
                        max_health: 75.0,
                        health: 0.0,
                        minimum_damage: 4.0,
                        replacement_prefab: Some(engine_serialize::AssetId::new("crate-fracture")),
                        broken: true,
                        ..Default::default()
                    },
                );
            })
            .unwrap();

        let save = game_loop.capture_save_game(BTreeMap::new()).unwrap();
        game_loop
            .runtime
            .with_world_mut(|world| {
                let cube = world.entity_by_persistent_id("cube-01").unwrap();
                *world.get_mut::<engine_physics::Destructible>(cube).unwrap() =
                    engine_physics::Destructible::default();
            })
            .unwrap();
        game_loop.restore_save_game(save).unwrap();

        game_loop
            .runtime
            .with_world(|world| {
                let cube = world.entity_by_persistent_id("cube-01").unwrap();
                let destructible = world.get::<engine_physics::Destructible>(cube).unwrap();
                assert_eq!(destructible.max_health, 75.0);
                assert_eq!(destructible.health, 0.0);
                assert_eq!(destructible.minimum_damage, 4.0);
                assert_eq!(
                    destructible.replacement_prefab.as_ref().unwrap().id,
                    "crate-fracture"
                );
                assert!(destructible.broken);
            })
            .unwrap();
    }
}

#[cfg(test)]
mod frame_timing_tests {
    use super::*;

    /// Minimal no-op backend so a GameLoop can drive full frames in-process.
    struct NoopBackend;

    impl engine_renderer::BackendRenderer for NoopBackend {
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
            _input: &engine_renderer::RenderFrameInput,
            _pass: &engine_renderer::render_graph2::PassNode,
            _frame_stats: &mut FrameStats,
        ) -> Result<(), Vec<Diagnostic>> {
            Ok(())
        }

        fn end_frame(&mut self, _stats: &mut FrameStats) -> Result<(), Vec<Diagnostic>> {
            Ok(())
        }

        fn abort_frame(&mut self) -> Result<(), Vec<Diagnostic>> {
            Ok(())
        }

        fn upload_mesh(
            &mut self,
            _upload: engine_renderer::MeshUpload,
        ) -> Result<engine_renderer::UploadReceipt, Vec<Diagnostic>> {
            Ok(engine_renderer::UploadReceipt::new(1))
        }

        fn upload_texture(
            &mut self,
            _upload: engine_renderer::TextureUpload,
        ) -> Result<engine_renderer::UploadReceipt, Vec<Diagnostic>> {
            Ok(engine_renderer::UploadReceipt::new(1))
        }

        fn upload_material(
            &mut self,
            _upload: engine_renderer::MaterialUpload,
        ) -> Result<engine_renderer::UploadReceipt, Vec<Diagnostic>> {
            Ok(engine_renderer::UploadReceipt::new(1))
        }
    }

    #[test]
    fn game_loop_attributes_update_and_render_stages_to_one_frame() {
        let mut game_loop = GameLoop::new(EngineConfig::default());
        game_loop
            .runtime
            .set_renderer_backend(Box::new(NoopBackend));
        game_loop
            .load_scene(engine_scene::sample_scene())
            .expect("sample scene should load");

        for frame in 0..5 {
            game_loop.update(1.0 / 60.0);
            game_loop.render(frame).expect("frame should render");
        }

        let timings = game_loop
            .runtime
            .last_frame_timings()
            .expect("frame timings after five frames");
        for stage in [
            "update",
            "extraction",
            "sync_render_assets",
            "render_submit",
        ] {
            assert!(
                timings
                    .passes
                    .iter()
                    .any(|pass| pass.name == stage && pass.cpu_ms.is_some()),
                "missing CPU stage '{stage}' in {timings:?}"
            );
        }
        let stage_sum: f32 = timings.passes.iter().filter_map(|pass| pass.cpu_ms).sum();
        assert!(
            (stage_sum - timings.total_cpu_ms).abs() < f32::EPSILON,
            "CPU stage attribution must sum to the frame total"
        );

        let summary = game_loop.frame_timing_summary();
        assert_eq!(summary.window_frames, 5);
        let update = summary
            .passes
            .iter()
            .find(|pass| pass.name == "update")
            .expect("update stats");
        assert_eq!(update.cpu.expect("cpu aggregate").samples, 5);
        assert_eq!(
            summary.gpu_status,
            engine_renderer::GpuTimingStatus::Unavailable,
            "a no-op backend reports GPU timing as unavailable"
        );
    }
}
