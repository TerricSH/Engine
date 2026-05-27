use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::{AudioCommand, AudioError};

// ---------------------------------------------------------------------------
// AudioHandle
// ---------------------------------------------------------------------------

/// A lightweight, `Send` handle for controlling playback from other systems.
///
/// Unlike [`AudioSource`], `AudioHandle` is cheap to clone and can be sent to
/// other threads. However, it provides a restricted API (no play/pause/seek).
pub struct AudioHandle {
    /// Voice id that this handle refers to.
    id: u64,
    /// Sender to the audio engine's command channel.
    cmd_tx: crossbeam_channel::Sender<AudioCommand>,
    /// Shared finished flag updated by the mixer.
    finished: Arc<AtomicBool>,
}

// SAFETY: AudioHandle contains only Send types (u64, crossbeam sender, Arc<AtomicBool>).
unsafe impl Send for AudioHandle {}
// SAFETY: All methods take &self or &mut self, no interior mutability beyond `finished`.
unsafe impl Sync for AudioHandle {}

impl AudioHandle {
    /// Create a new handle.
    pub(crate) fn new(
        id: u64,
        cmd_tx: crossbeam_channel::Sender<AudioCommand>,
        finished: Arc<AtomicBool>,
    ) -> Self {
        Self { id, cmd_tx, finished }
    }

    /// Stop playback. Returns `InvalidHandle` if the command channel is closed.
    pub fn stop(&mut self) -> Result<(), AudioError> {
        self.finished.store(true, Ordering::Release);
        self.cmd_tx
            .send(AudioCommand::Stop { id: self.id })
            .map_err(|_| AudioError::InvalidHandle)
    }

    /// Set volume (0.0–1.0).
    pub fn set_volume(&mut self, volume: f32) -> Result<(), AudioError> {
        let vol = volume.clamp(0.0, 1.0);
        self.cmd_tx
            .send(AudioCommand::SetVolume {
                id: self.id,
                volume: vol,
            })
            .map_err(|_| AudioError::InvalidHandle)
    }

    /// Enable or disable looping.
    pub fn set_loop(&mut self, enabled: bool) -> Result<(), AudioError> {
        self.cmd_tx
            .send(AudioCommand::SetLoop {
                id: self.id,
                enabled,
            })
            .map_err(|_| AudioError::InvalidHandle)
    }

    /// Whether the voice has finished playing (EOF and not looping).
    pub fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }
}

impl Clone for AudioHandle {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            cmd_tx: self.cmd_tx.clone(),
            finished: self.finished.clone(),
        }
    }
}
