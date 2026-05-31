use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::clip::AudioClip;
use crate::handle::AudioHandle;
#[cfg(feature = "subsystem-audio-cpal")]
use crate::mixer::MixerState;
use crate::{AudioCommand, AudioEmitter, AudioError, AudioListener, MixerGroup};

#[cfg(feature = "subsystem-audio-cpal")]
use cpal::traits::{DeviceTrait, HostTrait};

// ---------------------------------------------------------------------------
// AudioEngine
// ---------------------------------------------------------------------------

/// Main audio system manager.
///
/// Owns the cpal output stream and communicates with the audio callback thread
/// via a lock-free SPSC command channel.
///
/// # Usage
///
/// ```ignore
/// let mut engine = AudioEngine::new()?;
/// let clip = AudioClip::decode(&Path::new("music.wav"))?;
/// let handle = engine.play(Arc::new(clip))?;
/// // ...
/// engine.update(0.016); // call every frame
/// ```
pub struct AudioEngine {
    /// Command sender to the audio callback.
    cmd_tx: crossbeam_channel::Sender<AudioCommand>,
    /// Keep the cpal stream alive.
    #[cfg(feature = "subsystem-audio-cpal")]
    _stream: Option<cpal::Stream>,
    /// Monotonically increasing source id counter.
    next_id: u64,
    /// Cached master volume.
    master_volume: f32,
    /// Current listener state.
    listener: AudioListener,
    /// Active sound handles keyed by handle ID.
    /// Used by the FFI layer to stop / set volume on individual sounds.
    active_handles: HashMap<u64, AudioHandle>,
}

// SAFETY: AudioEngine is Send because all fields are Send.
// The cpal Stream is Send, the channel sender is Send.
unsafe impl Send for AudioEngine {}

impl AudioEngine {
    /// Create a new audio engine.
    ///
    /// Initializes the cpal output device and starts the audio callback thread.
    /// Without the `subsystem-audio-cpal` feature, returns
    /// `Err(AudioError::DeviceUnavailable)`.
    #[cfg(feature = "subsystem-audio-cpal")]
    pub fn new() -> Result<Self, AudioError> {
        let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded::<AudioCommand>();

        let stream = Self::build_stream(cmd_rx)?;

        tracing::info!("audio engine initialized");

        Ok(Self {
            cmd_tx,
            _stream: Some(stream),
            next_id: 1,
            master_volume: 1.0,
            listener: AudioListener::default(),
            active_handles: HashMap::new(),
        })
    }

    /// Create a new audio engine when the cpal feature is not enabled.
    ///
    /// Returns an error so that callers can still compile and test non-audio
    /// code paths.
    #[cfg(not(feature = "subsystem-audio-cpal"))]
    pub fn new() -> Result<Self, AudioError> {
        Err(AudioError::DeviceUnavailable(
            "audio subsystem not enabled (feature `subsystem-audio-cpal` is required)".into(),
        ))
    }

    // ------------------------------------------------------------------
    // Stream construction (cpal feature only)
    // ------------------------------------------------------------------
    #[cfg(feature = "subsystem-audio-cpal")]
    fn build_stream(
        cmd_rx: crossbeam_channel::Receiver<AudioCommand>,
    ) -> Result<cpal::Stream, AudioError> {
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or(AudioError::NoDevice)?;

        let device_name = device.name().unwrap_or_else(|_| "<unknown>".to_string());
        tracing::info!("audio output device: {}", device_name);

        let config =
            device
                .default_output_config()
                .map_err(|e: cpal::DefaultStreamConfigError| {
                    AudioError::DeviceUnavailable(e.to_string())
                })?;

        let sample_rate = config.config().sample_rate.0;
        let channels = config.config().channels;

        tracing::info!(
            "audio stream config: {} Hz, {} channels, {:?}",
            sample_rate,
            channels,
            config.sample_format(),
        );

        let output_channels = channels as u16;
        let mut mixer = Box::new(MixerState::new(sample_rate));

        // Build the stream with a callback that never allocates in steady state
        // (the separate `temp` buffers for non-f32 formats grow at most once).
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => device.build_output_stream::<f32, _, _>(
                &config.config(),
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    mixer.process_commands(&cmd_rx);
                    mixer.mix_f32(data, output_channels);
                },
                |err| {
                    tracing::error!("audio stream error: {}", err);
                },
                None,
            ),
            cpal::SampleFormat::I16 => {
                // Separate scratch buffer to avoid borrow conflicts with mixer.
                // Grown at most once (cpal buffer sizes are stable after stream creation).
                let mut temp: Vec<f32> = Vec::with_capacity(4096);
                device.build_output_stream::<i16, _, _>(
                    &config.config(),
                    move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                        mixer.process_commands(&cmd_rx);

                        // Grow temp if needed (happens at most once).
                        if data.len() > temp.len() {
                            temp.resize(data.len(), 0.0);
                        }

                        mixer.mix_f32(&mut temp[..data.len()], output_channels);

                        for (i, &sample) in temp[..data.len()].iter().enumerate() {
                            data[i] = (sample.clamp(-1.0, 1.0) * 32767.0) as i16;
                        }
                    },
                    |err| {
                        tracing::error!("audio stream error: {}", err);
                    },
                    None,
                )
            }
            cpal::SampleFormat::U16 => {
                let mut temp: Vec<f32> = Vec::with_capacity(4096);
                device.build_output_stream::<u16, _, _>(
                    &config.config(),
                    move |data: &mut [u16], _: &cpal::OutputCallbackInfo| {
                        mixer.process_commands(&cmd_rx);

                        if data.len() > temp.len() {
                            temp.resize(data.len(), 0.0);
                        }

                        mixer.mix_f32(&mut temp[..data.len()], output_channels);

                        for (i, &sample) in temp[..data.len()].iter().enumerate() {
                            data[i] = ((sample.clamp(-1.0, 1.0) * 0.5 + 0.5) * 65535.0) as u16;
                        }
                    },
                    |err| {
                        tracing::error!("audio stream error: {}", err);
                    },
                    None,
                )
            }
            other => {
                return Err(AudioError::UnsupportedFormat(format!(
                    "cpal sample format {:?} is not supported (only F32, I16, U16)",
                    other
                )));
            }
        };

        stream.map_err(|e: cpal::BuildStreamError| AudioError::DeviceUnavailable(e.to_string()))
    }

    // ------------------------------------------------------------------
    // Public API
    // ------------------------------------------------------------------

    /// Play a clip and return a handle for controlling it.
    ///
    /// The clip is played once at full volume without looping and without
    /// spatialisation. Use [`AudioEngine::play_spatial`] for 3D audio.
    pub fn play(&mut self, clip: Arc<AudioClip>) -> Result<AudioHandle, AudioError> {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        let finished = Arc::new(AtomicBool::new(false));

        self.cmd_tx
            .send(AudioCommand::Play {
                id,
                clip,
                volume: 1.0,
                loop_enabled: false,
                emitter: None,
                finished: finished.clone(),
                group: MixerGroup::Sfx,
            })
            .map_err(|_| AudioError::StreamError("command channel closed".to_string()))?;

        let handle = AudioHandle::new(id, self.cmd_tx.clone(), finished);
        self.active_handles.insert(id, handle.clone());
        Ok(handle)
    }

    /// Play a clip with spatial audio and return a handle.
    pub fn play_spatial(
        &mut self,
        clip: Arc<AudioClip>,
        emitter: AudioEmitter,
    ) -> Result<AudioHandle, AudioError> {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        let finished = Arc::new(AtomicBool::new(false));

        self.cmd_tx
            .send(AudioCommand::Play {
                id,
                clip,
                volume: 1.0,
                loop_enabled: false,
                emitter: Some(emitter),
                finished: finished.clone(),
                group: MixerGroup::Sfx,
            })
            .map_err(|_| AudioError::StreamError("command channel closed".to_string()))?;

        let handle = AudioHandle::new(id, self.cmd_tx.clone(), finished);
        self.active_handles.insert(id, handle.clone());
        Ok(handle)
    }

    /// Stop a playing sound by handle ID.
    ///
    /// Returns `true` if a sound with that ID was found and stopped.
    pub fn stop(&mut self, handle_id: u64) -> bool {
        if let Some(mut handle) = self.active_handles.remove(&handle_id) {
            let _ = handle.stop();
            true
        } else {
            false
        }
    }

    /// Set the volume of a playing sound by handle ID.
    ///
    /// `volume` is clamped to `[0, 1]`.  Returns `true` if the handle was found.
    pub fn set_volume(&mut self, handle_id: u64, volume: f32) -> bool {
        if let Some(handle) = self.active_handles.get_mut(&handle_id) {
            let _ = handle.set_volume(volume.clamp(0.0, 1.0));
            true
        } else {
            false
        }
    }

    /// Set the global listener.
    pub fn set_listener(&mut self, listener: AudioListener) {
        self.listener = listener.clone();
        let _ = self.cmd_tx.send(AudioCommand::SetListener(listener));
    }

    /// Per-frame update.  Synchronises ECS-driven positional data with the
    /// audio callback thread and prunes finished handles.
    ///
    /// Call this once per frame after the ECS world has been ticked.
    /// `listener_transform` — when `Some`, updates the spatial listener
    /// position/orientation.
    /// `source_positions` — an iterator of `(source_id, position)` pairs
    /// for active spatial sources whose position changed.
    pub fn update(
        &mut self,
        _dt: f32,
        listener_transform: Option<&engine_scene::components::Transform>,
        source_positions: &[(u64, glam::Vec3)],
    ) {
        if let Some(lt) = listener_transform {
            let mut listener = AudioListener::new();
            listener.set_position(lt.translation);
            // Orientation derived from the transform's rotation.
            let fwd = lt.rotation * -glam::Vec3::Z;
            let up = lt.rotation * glam::Vec3::Y;
            listener.set_orientation(fwd, up);
            let _ = self.cmd_tx.send(AudioCommand::SetListener(listener));
        }

        for &(source_id, pos) in source_positions {
            let _ = self.cmd_tx.send(AudioCommand::SetEmitterPosition {
                id: source_id,
                position: pos,
            });
        }

        // Prune finished handles to prevent unbounded growth.
        self.active_handles
            .retain(|_id, handle| !handle.is_finished());
    }

    /// Set the master volume (0.0–1.0).
    pub fn set_master_volume(&mut self, volume: f32) {
        self.master_volume = volume.clamp(0.0, 1.0);
        let _ = self
            .cmd_tx
            .send(AudioCommand::SetMasterVolume(self.master_volume));
    }

    /// Get the current master volume.
    pub fn master_volume(&self) -> f32 {
        self.master_volume
    }

    /// Stop all currently playing sounds.
    pub fn stop_all(&mut self) {
        let _ = self.cmd_tx.send(AudioCommand::StopAll);
    }

    /// Set the volume of an entire mixer group (Music / Sfx / Ui / Ambience).
    ///
    /// `volume` is clamped to `[0, 1]`.  This is multiplied with per-voice
    /// and master volume during mixing.
    pub fn set_group_volume(&mut self, group: MixerGroup, volume: f32) {
        let vol = volume.clamp(0.0, 1.0);
        let _ = self.cmd_tx.send(AudioCommand::SetGroupVolume(group, vol));
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        // The cpal Stream is dropped, which stops the audio callback.
        // The command channel is dropped, so any future sends will error.
        tracing::info!("audio engine shutting down");
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

// TODO: Add AudioSource creation from engine (future).
// The AudioSource::new() is crate-internal; external users create sounds via
// engine.play() / play_spatial() which return AudioHandle.
//
// If a full AudioSource (with play/pause/seek) is needed, the engine will
// expose a `create_source()` method in a future pass.
