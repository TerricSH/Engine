use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::clip::AudioClip;
use crate::handle::AudioHandle;
#[cfg(feature = "subsystem-audio-cpal")]
use crate::mixer::MixerState;
use crate::{AudioCommand, AudioEmitter, AudioError, AudioListener};

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
        let device = host
            .default_output_device()
            .ok_or(AudioError::NoDevice)?;

        let device_name = device
            .name()
            .unwrap_or_else(|_| "<unknown>".to_string());
        tracing::info!("audio output device: {}", device_name);

        let config = device
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
            cpal::SampleFormat::F32 => {
                device.build_output_stream::<f32, _, _>(
                    &config.config(),
                    move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                        mixer.process_commands(&cmd_rx);
                        mixer.mix_f32(data, output_channels);
                    },
                    |err| {
                        tracing::error!("audio stream error: {}", err);
                    },
                    None,
                )
            }
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

        stream.map_err(|e: cpal::BuildStreamError| {
            AudioError::DeviceUnavailable(e.to_string())
        })
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
                volume: self.master_volume,
                loop_enabled: false,
                emitter: None,
                finished: finished.clone(),
            })
            .map_err(|_| AudioError::StreamError("command channel closed".to_string()))?;

        Ok(AudioHandle::new(id, self.cmd_tx.clone(), finished))
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
            })
            .map_err(|_| AudioError::StreamError("command channel closed".to_string()))?;

        Ok(AudioHandle::new(id, self.cmd_tx.clone(), finished))
    }

    /// Set the global listener.
    pub fn set_listener(&mut self, listener: AudioListener) {
        self.listener = listener.clone();
        let _ = self
            .cmd_tx
            .send(AudioCommand::SetListener(listener));
    }

    /// Per-frame update. Processes any pending state synchronisation.
    ///
    /// `dt` is the frame delta in seconds (reserved for future use such as
    /// main-thread positional interpolation).
    pub fn update(&mut self, _dt: f32) {
        // Currently the command queue is drained by the audio callback itself.
        // This method is reserved for future main-thread work such as
        // streaming-chunk submission or statistics gathering.
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
