// Some items are kept for future spatial-audio integration
// and are suppressed from dead-code warnings explicitly.
#![allow(dead_code, reason = "spatial audio scaffolding")]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use glam::Vec3;

use crate::clip::AudioClip;
use crate::{
    AudioCommand, AudioListener, MixerGroup, _compute_stereo_pan, _distance_attenuation,
    _MAX_VOICES, _VOLUME_RAMP_SECS,
};

// ---------------------------------------------------------------------------
// Mixer state  (owned by the audio-callback thread)
// ---------------------------------------------------------------------------

/// Per-voice state, pre-allocated in a fixed-size pool.
pub(crate) struct MixerVoice {
    /// Unique voice identifier (0 = slot unused).
    id: u64,
    /// The clip being played, if any.
    clip: Option<Arc<AudioClip>>,
    /// Current read position in frames (interleaved frame index).
    read_frame: usize,
    /// Current instantaneous volume (ramped).
    current_volume: f32,
    /// Target volume for the ramp.
    target_volume: f32,
    /// Volume delta per output sample during the active ramp.
    volume_ramp_delta: f32,
    /// Number of samples remaining in the current ramp.
    ramp_samples_remaining: u32,
    /// Whether to loop when the clip ends.
    loop_enabled: bool,
    /// Whether the voice is paused.
    paused: bool,
    /// Shared finished flag (read by main thread via `AudioHandle::is_finished`).
    finished: Arc<AtomicBool>,
    /// Spatial audio configuration, if any.
    spatial: bool,
    emitter_position: Vec3,
    emitter_max_distance: f32,
    emitter_rolloff: f32,
    /// Pre-clip volume for this voice (before master volume).
    voice_volume: f32,
    /// Mixer group this voice belongs to.
    group: MixerGroup,
}

/// Mixer state: everything the audio callback touches.
///
/// **Must not allocate inside the callback.** All storage is pre-allocated
/// during construction (on the main thread).
pub(crate) struct MixerState {
    /// Pre-allocated voice pool.
    voices: Vec<MixerVoice>,
    /// Current master volume.
    master_volume: f32,
    /// Current listener for spatial audio.
    listener: AudioListener,
    /// Output sample rate (from cpal config).
    sample_rate: u32,
    /// Per-group volume multipliers, indexed by [`MixerGroup::index()`].
    group_volumes: [f32; MixerGroup::COUNT],
}

impl MixerState {
    pub(crate) fn new(sample_rate: u32) -> Self {
        // Pre-allocate the voice pool.
        let mut voices = Vec::with_capacity(_MAX_VOICES);
        for _ in 0.._MAX_VOICES {
            voices.push(MixerVoice::new());
        }

        Self {
            voices,
            master_volume: 1.0,
            listener: AudioListener::default(),
            sample_rate,
            group_volumes: [1.0; MixerGroup::COUNT],
        }
    }

    /// Drain the command channel, processing every pending command.
    pub(crate) fn process_commands(&mut self, rx: &crossbeam_channel::Receiver<AudioCommand>) {
        // The channel may hold multiple commands; drain them all.
        while let Ok(cmd) = rx.try_recv() {
            match cmd {
                AudioCommand::Play {
                    id,
                    clip,
                    volume,
                    loop_enabled,
                    emitter,
                    finished,
                    group,
                } => {
                    let sr = self.sample_rate;
                    if let Some(slot) = self.find_free_slot() {
                        slot.id = id;
                        slot.clip = Some(clip);
                        slot.read_frame = 0;
                        slot.target_volume = volume;
                        slot.current_volume = 0.0;
                        slot.voice_volume = volume;
                        slot.volume_ramp_delta = volume / (sr as f32 * _VOLUME_RAMP_SECS).max(1.0);
                        slot.ramp_samples_remaining = (sr as f32 * _VOLUME_RAMP_SECS) as u32;
                        slot.loop_enabled = loop_enabled;
                        slot.paused = false;
                        slot.finished = finished;
                        slot.finished.store(false, Ordering::Release);
                        if let Some(em) = emitter {
                            slot.spatial = true;
                            slot.emitter_position = em.position;
                            slot.emitter_max_distance = em.max_distance;
                            slot.emitter_rolloff = em.rolloff_factor;
                        } else {
                            slot.spatial = false;
                        }
                        slot.group = group;
                    }
                }
                AudioCommand::Stop { id } => {
                    if let Some(slot) = self.find_voice(id) {
                        slot.stop();
                    }
                }
                AudioCommand::Pause { id } => {
                    if let Some(slot) = self.find_voice(id) {
                        slot.paused = true;
                    }
                }
                AudioCommand::Resume { id } => {
                    if let Some(slot) = self.find_voice(id) {
                        slot.paused = false;
                    }
                }
                AudioCommand::SetVolume { id, volume } => {
                    let sr = self.sample_rate;
                    if let Some(slot) = self.find_voice(id) {
                        slot.set_target_volume(volume, sr);
                    }
                }
                AudioCommand::SetLoop { id, enabled } => {
                    if let Some(slot) = self.find_voice(id) {
                        slot.loop_enabled = enabled;
                    }
                }
                AudioCommand::SetPosition { id, pos_frame } => {
                    if let Some(slot) = self.find_voice(id) {
                        if let Some(ref clip) = slot.clip {
                            let max_frame = clip.samples().len() / clip.channels() as usize;
                            slot.read_frame = pos_frame.min(max_frame.saturating_sub(1));
                        }
                    }
                }
                AudioCommand::SetEmitterPosition { id, position } => {
                    if let Some(slot) = self.find_voice(id) {
                        slot.spatial = true;
                        slot.emitter_position = position;
                    }
                }
                AudioCommand::SetMasterVolume(vol) => {
                    self.master_volume = vol.clamp(0.0, 1.0);
                }
                AudioCommand::SetListener(listener) => {
                    self.listener = listener;
                }
                AudioCommand::SetGroupVolume(group, vol) => {
                    let idx = group.index();
                    if idx < self.group_volumes.len() {
                        self.group_volumes[idx] = vol.clamp(0.0, 1.0);
                    }
                }
                AudioCommand::StopAll => {
                    for v in &mut self.voices {
                        if v.id != 0 {
                            v.stop();
                        }
                    }
                }
            }
        }
    }

    /// Mix interleaved stereo output directly into a `f32` buffer.
    pub(crate) fn mix_f32(&mut self, data: &mut [f32], channels: u16) {
        let num_frames = data.len() / channels as usize;

        for frame in 0..num_frames {
            let mut left_sum: f32 = 0.0;
            let mut right_sum: f32 = 0.0;

            for v in &mut self.voices {
                if v.id == 0 || v.paused {
                    continue;
                }
                let Some(ref clip) = v.clip else {
                    continue;
                };

                let clip_channels = (clip.channels() as usize).max(1);
                let clip_frames = clip.samples().len() / clip_channels;

                // Check for end-of-clip.
                if v.read_frame >= clip_frames {
                    if v.loop_enabled {
                        v.read_frame = 0;
                    } else {
                        v.stop();
                        continue;
                    }
                }

                // Read one interleaved frame from the clip.
                let base = v.read_frame * clip_channels;
                let (s_left, s_right) = if clip_channels >= 2 {
                    (clip.samples()[base], clip.samples()[base + 1])
                } else {
                    let m = clip.samples()[base];
                    (m, m) // mono → stereo
                };

                // Apply volume ramp.
                if v.ramp_samples_remaining > 0 {
                    v.current_volume += v.volume_ramp_delta;
                    v.ramp_samples_remaining -= 1;
                } else {
                    v.current_volume = v.target_volume;
                }
                let vol = v.current_volume;

                // Advance read position.
                v.read_frame += 1;

                // Apply per-voice volume * group (bus) volume.
                let g_vol = self.group_volumes[v.group.index()];
                let left_sample = s_left * vol * g_vol;
                let right_sample = s_right * vol * g_vol;

                // Spatial audio panning.
                if v.spatial {
                    let (pan_l, pan_r) = _compute_stereo_pan(
                        v.emitter_position,
                        self.listener.position,
                        self.listener.forward,
                        self.listener.up,
                    );
                    let dist = v.emitter_position.distance(self.listener.position);
                    let atten =
                        _distance_attenuation(dist, v.emitter_max_distance, v.emitter_rolloff);
                    left_sum += left_sample * pan_l * atten;
                    right_sum += right_sample * pan_r * atten;
                } else {
                    left_sum += left_sample;
                    right_sum += right_sample;
                }
            }

            // Apply master volume.
            left_sum *= self.master_volume;
            right_sum *= self.master_volume;

            // Clamp to avoid overflow in output.
            let out_idx = frame * channels as usize;
            data[out_idx] = left_sum.clamp(-1.0, 1.0);
            if channels > 1 {
                data[out_idx + 1] = right_sum.clamp(-1.0, 1.0);
            }
        }
    }

    /// Find a free voice slot (id == 0).
    fn find_free_slot(&mut self) -> Option<&mut MixerVoice> {
        self.voices.iter_mut().find(|v| v.id == 0)
    }

    /// Find a voice by id.
    fn find_voice(&mut self, id: u64) -> Option<&mut MixerVoice> {
        self.voices.iter_mut().find(|v| v.id == id)
    }
}

impl MixerVoice {
    fn new() -> Self {
        Self {
            id: 0,
            clip: None,
            read_frame: 0,
            current_volume: 0.0,
            target_volume: 0.0,
            volume_ramp_delta: 0.0,
            ramp_samples_remaining: 0,
            loop_enabled: false,
            paused: false,
            finished: Arc::new(AtomicBool::new(true)),
            spatial: false,
            emitter_position: Vec3::ZERO,
            emitter_max_distance: 10.0,
            emitter_rolloff: 1.0,
            voice_volume: 1.0,
            group: MixerGroup::default(),
        }
    }

    fn stop(&mut self) {
        self.finished.store(true, Ordering::Release);
        self.id = 0;
        self.clip = None;
        self.read_frame = 0;
        self.current_volume = 0.0;
        self.target_volume = 0.0;
        self.paused = false;
    }

    fn set_target_volume(&mut self, volume: f32, sample_rate: u32) {
        let clamped = volume.clamp(0.0, 1.0);
        self.target_volume = clamped;
        self.voice_volume = clamped;
        let ramp_frames = (sample_rate as f32 * _VOLUME_RAMP_SECS) as u32;
        if ramp_frames > 0 {
            self.volume_ramp_delta = (clamped - self.current_volume) / ramp_frames as f32;
            self.ramp_samples_remaining = ramp_frames;
        } else {
            self.current_volume = clamped;
            self.volume_ramp_delta = 0.0;
            self.ramp_samples_remaining = 0;
        }
    }
}
