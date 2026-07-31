use super::*;

#[cfg(feature = "runtime-audio-output")]
#[derive(Clone, Debug, PartialEq)]
pub(super) struct RuntimeAudioEmitterSnapshot {
    pub(super) position: Vec3,
    pub(super) max_distance: f32,
    pub(super) rolloff_factor: f32,
}

#[cfg(feature = "runtime-audio-output")]
#[derive(Clone, Debug, PartialEq)]
pub(super) struct RuntimeAudioSourceSnapshot {
    pub(super) entity_id: String,
    pub(super) clip_asset: Option<String>,
    pub(super) clip_loaded: bool,
    pub(super) playing: bool,
    pub(super) volume: f32,
    pub(super) looping: bool,
    pub(super) emitter: Option<RuntimeAudioEmitterSnapshot>,
}

#[cfg(feature = "runtime-audio-output")]
impl RuntimeAudioSourceSnapshot {
    pub(super) fn playable_clip(&self) -> Option<&str> {
        self.playing
            .then_some(())
            .filter(|_| self.clip_loaded)
            .and(self.clip_asset.as_deref())
            .filter(|clip_asset| !clip_asset.is_empty())
    }
}

#[cfg(feature = "runtime-audio-output")]
#[derive(Clone, Debug, PartialEq)]
pub(super) struct RuntimeAudioListenerSnapshot {
    pub(super) position: Vec3,
    pub(super) forward: Vec3,
    pub(super) up: Vec3,
}

#[cfg(feature = "runtime-audio-output")]
#[derive(Default)]
pub(super) struct RuntimeAudioFrame {
    pub(super) sources: Vec<RuntimeAudioSourceSnapshot>,
    pub(super) listener: Option<RuntimeAudioListenerSnapshot>,
}

#[cfg(feature = "runtime-audio-output")]
#[derive(Clone, Debug, PartialEq)]
pub(super) enum RuntimeAudioSourceAction {
    Start(RuntimeAudioSourceSnapshot),
    Update(RuntimeAudioSourceSnapshot),
    Stop { entity_id: String },
}

#[cfg(feature = "runtime-audio-output")]
#[derive(Clone, Debug)]
pub(super) struct RuntimeAudioVoiceState {
    pub(super) clip_asset: String,
    pub(super) looping: bool,
    pub(super) completed_while_requested: bool,
}

/// Device-independent scene/audio state reconciliation.
///
/// Keeping this state machine separate from cpal makes scene playback
/// semantics testable on build agents and machines without an output device.
#[cfg(feature = "runtime-audio-output")]
#[derive(Default)]
pub(super) struct SceneAudioReconciler {
    pub(super) voices: BTreeMap<String, RuntimeAudioVoiceState>,
}

#[cfg(feature = "runtime-audio-output")]
impl SceneAudioReconciler {
    pub(super) fn reconcile(
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

    pub(super) fn reset(&mut self) -> Vec<RuntimeAudioSourceAction> {
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
pub(super) struct RuntimeAudioOutput {
    pub(super) reconciler: SceneAudioReconciler,
    pub(super) engine: Option<engine_audio::AudioEngine>,
    pub(super) handles: BTreeMap<String, engine_audio::AudioHandle>,
    pub(super) initialization_failed: bool,
}
#[cfg(feature = "runtime-audio-output")]
#[derive(Clone, Copy)]
pub(super) struct RuntimeAudioPose {
    pub(super) position: Vec3,
    pub(super) forward: Vec3,
    pub(super) up: Vec3,
}

#[cfg(feature = "runtime-audio-output")]
pub(super) fn resolved_audio_pose(
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
pub(super) fn runtime_audio_emitter(
    snapshot: &RuntimeAudioEmitterSnapshot,
) -> engine_audio::AudioEmitter {
    let mut emitter = engine_audio::AudioEmitter::new(snapshot.position);
    emitter.set_max_distance(snapshot.max_distance);
    emitter.set_rolloff_factor(snapshot.rolloff_factor);
    emitter
}

#[cfg(feature = "runtime-audio-output")]
pub(super) fn runtime_audio_listener(
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
    pub(super) fn update(&mut self, runtime: &EngineRuntime, frame: RuntimeAudioFrame, dt: f32) {
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

    pub(super) fn ensure_engine_with(
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

    pub(super) fn reset_scene(&mut self) {
        let _ = self.reconciler.reset();
        if let Some(engine) = self.engine.as_mut() {
            engine.stop_all();
        }
        self.handles.clear();
        self.initialization_failed = false;
    }
}

impl GameLoop {
    /// Synchronise scene audio components with the lazily-created desktop
    /// output device. Device failures are recoverable: the rest of the game
    /// loop continues and another scene load rearms one initialization attempt.
    #[cfg(feature = "runtime-audio-output")]
    pub(super) fn update_runtime_audio(&mut self, dt: f32) {
        let frame = self.runtime_audio_frame();
        self.audio_output.update(&self.runtime, frame, dt);
    }

    #[cfg(feature = "runtime-audio-output")]
    pub(super) fn reset_runtime_audio_scene(&mut self) {
        self.audio_output.reset_scene();
    }

    #[cfg(feature = "runtime-audio-output")]
    pub(super) fn runtime_audio_frame(&self) -> RuntimeAudioFrame {
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
}
