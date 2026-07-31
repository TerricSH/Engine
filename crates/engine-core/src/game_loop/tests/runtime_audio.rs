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
