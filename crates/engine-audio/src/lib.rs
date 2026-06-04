//! Engine audio system using cpal for output and symphonia for decoding.
//!
//! Per FD-035 policy, this crate is excepted from `#![forbid(unsafe_code)]`
//! because cpal's audio callback requires unsafe for FFI with platform audio APIs.
//! Every `unsafe` block carries a `// SAFETY:` comment.
//!
//! # Architecture
//!
//! - **AudioEngine**: Main-thread manager. Sends commands to the audio callback via a
//!   lock-free SPSC channel (`crossbeam-channel`).
//! - **Audio thread** (cpal callback): Owns the mixer state (pre-allocated voice pool),
//!   processes commands, mixes stereo output. Never allocates, never acquires locks.
//!
//! # Feature flags
//!
//! - `subsystem-audio-cpal`: Enables the cpal output path. Without it, `AudioEngine::new()`
//!   returns `Err(AudioError::DeviceUnavailable)`.
//! - Core types (`AudioClip`, `AudioSource`, `AudioHandle`, `AudioEmitter`, `AudioListener`,
//!   `AudioError`) are always available.

// No #![forbid(unsafe_code)] — excepted per FD-035 policy (cpal audio callback requires unsafe)

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use glam::Vec3;

// ---------------------------------------------------------------------------
// Module declarations
// ---------------------------------------------------------------------------

pub mod clip;
pub mod components;
pub mod engine;
pub mod handle;
pub(crate) mod mixer;
pub mod source;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum number of simultaneous voices (pre-allocated pool, never exceeded).
pub(crate) const _MAX_VOICES: usize = 32;

/// Volume ramp duration in seconds (linear, avoids clicks on parameter changes).
pub(crate) const _VOLUME_RAMP_SECS: f32 = 0.005;

/// Number of output channels (always stereo).
pub(crate) const _OUTPUT_CHANNELS: u16 = 2;

// ---------------------------------------------------------------------------
// Spatial audio helpers
// ---------------------------------------------------------------------------

/// Constant-power stereo pan from emitter position relative to listener.
///
/// Returns `(left_gain, right_gain)` where both sum in power to 1.0 at centre.
pub(crate) fn _compute_stereo_pan(
    emitter: Vec3,
    listener_pos: Vec3,
    listener_forward: Vec3,
    listener_up: Vec3,
) -> (f32, f32) {
    let to_emitter = (emitter - listener_pos).normalize_or_zero();
    if to_emitter == Vec3::ZERO {
        return (
            std::f32::consts::FRAC_1_SQRT_2,
            std::f32::consts::FRAC_1_SQRT_2,
        );
    }

    let forward = listener_forward.normalize_or_zero();
    let up = listener_up.normalize_or_zero();
    let right = forward.cross(up).normalize_or_zero();

    // Dot with right vector: -1 = full left, +1 = full right.
    let pan = to_emitter.dot(right).clamp(-1.0, 1.0);

    // Constant-power pan law.
    let theta = (pan + 1.0) * std::f32::consts::FRAC_PI_4; // 0 … π/2
    let left_gain = theta.cos();
    let right_gain = theta.sin();

    (left_gain, right_gain)
}

/// Inverse-distance attenuation with rolloff factor.
pub(crate) fn _distance_attenuation(distance: f32, max_distance: f32, rolloff: f32) -> f32 {
    if distance <= 0.0 {
        return 1.0;
    }
    let d = distance.min(max_distance);
    1.0 / (1.0 + rolloff * d)
}

pub use clip::AudioClip;
pub use components::{register_audio_extensions, AudioListenerComponent, AudioSourceComponent};
pub use engine::AudioEngine;
pub use handle::AudioHandle;
pub use source::AudioSource;

pub use MixerGroup::*;

// ---------------------------------------------------------------------------
// AudioError
// ---------------------------------------------------------------------------

/// Errors produced by the audio system.
#[derive(thiserror::Error, Debug)]
pub enum AudioError {
    /// No audio output device is present on the system.
    #[error("no audio device available")]
    NoDevice,

    /// The requested audio device could not be opened or has been unplugged.
    #[error("audio device unavailable: {0}")]
    DeviceUnavailable(String),

    /// An error occurred while decoding audio data.
    #[error("decode error: {detail}")]
    DecodeError {
        /// Human-readable description of the decode failure.
        detail: String,
    },

    /// An `AudioHandle` or `AudioSource` referred to a voice that no longer exists.
    #[error("invalid audio handle")]
    InvalidHandle,

    /// The audio format or sample format is not supported.
    #[error("unsupported audio format: {0}")]
    UnsupportedFormat(String),

    /// The internal audio command channel has been closed (engine was dropped).
    #[error("audio stream error: {0}")]
    StreamError(String),
}

// ---------------------------------------------------------------------------
// Internal command queue
// ---------------------------------------------------------------------------

/// Commands sent from the main thread to the audio callback thread.
pub(crate) enum AudioCommand {
    Play {
        id: u64,
        clip: Arc<AudioClip>,
        volume: f32,
        loop_enabled: bool,
        emitter: Option<AudioEmitter>,
        finished: Arc<AtomicBool>,
        group: MixerGroup,
    },
    Stop {
        id: u64,
    },
    Pause {
        id: u64,
    },
    Resume {
        id: u64,
    },
    SetVolume {
        id: u64,
        volume: f32,
    },
    SetLoop {
        id: u64,
        enabled: bool,
    },
    SetPosition {
        id: u64,
        pos_frame: usize,
    },
    /// Update the world-space position of a spatial emitter so the mixer
    /// can recalculate pan and attenuation at mix time.
    SetEmitterPosition {
        id: u64,
        position: glam::Vec3,
    },
    SetMasterVolume(f32),
    SetListener(AudioListener),
    SetGroupVolume(MixerGroup, f32),
    StopAll,
}

// ---------------------------------------------------------------------------
// MixerGroup
// ---------------------------------------------------------------------------

/// Category groups for mixer bus routing.
///
/// Each voice is assigned a group at creation time.  Group volumes are
/// multiplied with per-voice and master volume during mixing, allowing
/// category-wide volume control (e.g. mute SFX while keeping music).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MixerGroup {
    /// Background music.
    Music,
    /// Sound effects.
    Sfx,
    /// User-interface sounds.
    Ui,
    /// Ambient / environmental sounds.
    Ambience,
}

#[allow(clippy::derivable_impls)]
impl Default for MixerGroup {
    fn default() -> Self {
        Self::Sfx
    }
}

impl MixerGroup {
    /// The number of mixer group variants.
    pub const COUNT: usize = 4;

    /// Return the index of this group for array lookups.
    pub fn index(self) -> usize {
        match self {
            MixerGroup::Music => 0,
            MixerGroup::Sfx => 1,
            MixerGroup::Ui => 2,
            MixerGroup::Ambience => 3,
        }
    }
}

// ---------------------------------------------------------------------------
// AudioEmitter
// ---------------------------------------------------------------------------

/// 3D position and volume controls for spatial audio.
#[derive(Clone, Debug)]
pub struct AudioEmitter {
    pub(crate) position: Vec3,
    pub(crate) max_distance: f32,
    pub(crate) rolloff_factor: f32,
}

impl AudioEmitter {
    /// Create a new emitter at the given position.
    pub fn new(position: Vec3) -> Self {
        Self {
            position,
            max_distance: 10.0,
            rolloff_factor: 1.0,
        }
    }

    /// Set the 3D position of the emitter.
    pub fn set_position(&mut self, position: Vec3) {
        self.position = position;
    }

    /// Get the 3D position.
    pub fn position(&self) -> Vec3 {
        self.position
    }

    /// Set the maximum distance for attenuation (clamped).
    pub fn set_max_distance(&mut self, max_distance: f32) {
        self.max_distance = max_distance.max(0.0);
    }

    /// Set the rolloff factor (>= 0.0). 0.0 = no attenuation.
    pub fn set_rolloff_factor(&mut self, factor: f32) {
        self.rolloff_factor = factor.max(0.0);
    }
}

// ---------------------------------------------------------------------------
// AudioListener
// ---------------------------------------------------------------------------

/// Listener position and orientation for spatial audio.
#[derive(Clone, Debug)]
pub struct AudioListener {
    pub(crate) position: Vec3,
    pub(crate) forward: Vec3,
    pub(crate) up: Vec3,
}

impl Default for AudioListener {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            // Per FD-031: forward = -Z, up = +Y
            forward: -Vec3::Z,
            up: Vec3::Y,
        }
    }
}

impl AudioListener {
    /// Create a new listener at the origin looking down -Z.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the listener's world-space position.
    pub fn set_position(&mut self, position: Vec3) {
        self.position = position;
    }

    /// Set the listener's forward and up vectors (should be normalized).
    pub fn set_orientation(&mut self, forward: Vec3, up: Vec3) {
        let fwd = forward.normalize_or_zero();
        let u = up.normalize_or_zero();
        if fwd != Vec3::ZERO && u != Vec3::ZERO {
            self.forward = fwd;
            self.up = u;
        }
    }
}

// ---------------------------------------------------------------------------

