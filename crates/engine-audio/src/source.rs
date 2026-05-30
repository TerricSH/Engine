use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::clip::AudioClip;
use crate::AudioCommand;

// ---------------------------------------------------------------------------
// AudioSource
// ---------------------------------------------------------------------------

/// A playable instance of an [`AudioClip`].
///
/// `AudioSource` manages playback state locally and sends commands to the
/// audio engine via a shared channel. It is single-threaded (the main thread).
///
/// For a lightweight handle that can be passed to other systems, see
/// [`AudioHandle`].
pub struct AudioSource {
    /// The underlying clip.
    clip: Arc<AudioClip>,
    /// Sender to the audio engine's command channel.
    cmd_tx: crossbeam_channel::Sender<AudioCommand>,
    /// Unique voice id assigned by the engine.
    id: u64,
    /// Current volume (cached on main thread).
    volume: f32,
    /// Whether looping is enabled.
    loop_enabled: bool,
    /// Whether `play()` has been called (optimistic — may not match mixer).
    playing: bool,
    /// Whether `pause()` was called.
    paused: bool,
    /// Current playback position in seconds (best-effort, tracked on main thread).
    position_secs: f32,
    /// Shared finished flag from the mixer.
    finished: Arc<AtomicBool>,
}

impl AudioSource {
    /// Create a new `AudioSource` bound to the given engine.
    pub(crate) fn _new(
        clip: Arc<AudioClip>,
        cmd_tx: crossbeam_channel::Sender<AudioCommand>,
        id: u64,
        finished: Arc<AtomicBool>,
    ) -> Self {
        Self {
            clip,
            cmd_tx,
            id,
            volume: 1.0,
            loop_enabled: false,
            playing: false,
            paused: false,
            position_secs: 0.0,
            finished,
        }
    }

    /// Start or resume playback.
    pub fn play(&mut self) {
        if self.paused {
            // Resume.
            self.paused = false;
            self.playing = true;
            let _ = self.cmd_tx.send(AudioCommand::Resume { id: self.id });
        } else {
            // Fresh start.
            self.playing = true;
            self.paused = false;
            self.position_secs = 0.0;
            self.finished.store(false, Ordering::Release);
            let _ = self.cmd_tx.send(AudioCommand::Play {
                id: self.id,
                clip: self.clip.clone(),
                volume: self.volume,
                loop_enabled: self.loop_enabled,
                emitter: None,
                finished: self.finished.clone(),
            });
        }
    }

    /// Stop playback and reset position.
    pub fn stop(&mut self) {
        self.playing = false;
        self.paused = false;
        self.position_secs = 0.0;
        let _ = self.cmd_tx.send(AudioCommand::Stop { id: self.id });
    }

    /// Pause playback (position is preserved).
    pub fn pause(&mut self) {
        self.paused = true;
        self.playing = false;
        let _ = self.cmd_tx.send(AudioCommand::Pause { id: self.id });
    }

    /// Set volume (0.0–1.0).
    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume.clamp(0.0, 1.0);
        let _ = self.cmd_tx.send(AudioCommand::SetVolume {
            id: self.id,
            volume: self.volume,
        });
    }

    /// Enable or disable looping.
    pub fn set_loop(&mut self, loop_enabled: bool) {
        self.loop_enabled = loop_enabled;
        let _ = self.cmd_tx.send(AudioCommand::SetLoop {
            id: self.id,
            enabled: loop_enabled,
        });
    }

    /// Whether the source is currently playing (optimistic — may briefly diverge
    /// from the mixer state).
    pub fn is_playing(&self) -> bool {
        self.playing && !self.finished.load(Ordering::Acquire)
    }

    /// Get the current playback position in seconds (best-effort).
    pub fn position_seconds(&self) -> f32 {
        self.position_secs
    }

    /// Set the playback position (seek) in seconds.
    pub fn set_position_seconds(&mut self, pos: f32) {
        let clamped = pos.max(0.0);
        self.position_secs = clamped;
        let sample_rate = self.clip.sample_rate();
        let channels = self.clip.channels() as usize;
        let pos_frame = (clamped * sample_rate as f32) as usize;
        let _ = self.cmd_tx.send(AudioCommand::SetPosition {
            id: self.id,
            pos_frame: pos_frame.min(self.clip.samples().len() / channels),
        });
    }
}
