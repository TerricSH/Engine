use serde::{Deserialize, Serialize};

use crate::{BoneIndex, BoneTransform, Pose, Skeleton};

// ---------------------------------------------------------------------------
// Keyframe
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Keyframe {
    pub time: f32,
    pub transform: BoneTransform,
}

// ---------------------------------------------------------------------------
// AnimationClip — set of channels (one per animated bone) plus duration.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Channel {
    bone: BoneIndex,
    keyframes: Vec<Keyframe>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnimationClip {
    name: String,
    duration_seconds: f32,
    channels: Vec<Channel>,
}

impl AnimationClip {
    /// Create a new clip with the given name and duration (in seconds).
    pub fn new(name: String, duration_seconds: f32) -> Self {
        tracing::debug!(
            clip = %name,
            duration = duration_seconds,
            "AnimationClip created"
        );
        Self {
            name,
            duration_seconds,
            channels: Vec::new(),
        }
    }

    /// Add a channel that animates `bone` with the provided sorted keyframes.
    /// Keyframes should be sorted by time in ascending order.
    pub fn add_channel(&mut self, bone: BoneIndex, keyframes: Vec<Keyframe>) {
        self.channels.push(Channel { bone, keyframes });
    }

    /// Evaluate the clip at `time` seconds and produce a pose.
    ///
    /// Starts with the skeleton's rest pose, then overrides each animated bone
    /// with the interpolated channel value.
    pub fn sample(&self, time: f32, skeleton: &Skeleton) -> Pose {
        let mut pose = Pose::new(skeleton);
        for channel in &self.channels {
            let bone_idx = channel.bone.0 as usize;
            if bone_idx >= pose.local.len() {
                continue;
            }
            pose.local[bone_idx] = sample_keyframes(&channel.keyframes, time);
        }
        pose
    }

    /// Total duration of the clip in seconds.
    pub fn duration(&self) -> f32 {
        self.duration_seconds
    }

    /// Human-readable clip name.
    pub fn name(&self) -> &str {
        &self.name
    }
}

// ---------------------------------------------------------------------------
// Keyframe sampling
// ---------------------------------------------------------------------------

/// Interpolate between keyframes at `time`.
///
/// - Before the first keyframe: held constant (first keyframe value).
/// - After the last keyframe: held constant (last keyframe value).
/// - Between two keyframes: LERP for translation/scale, SLERP for rotation.
fn sample_keyframes(keyframes: &[Keyframe], time: f32) -> BoneTransform {
    match keyframes.len() {
        0 => return BoneTransform::IDENTITY,
        1 => return keyframes[0].transform,
        _ => {}
    }

    // Clamp / hold.
    if time <= keyframes[0].time {
        return keyframes[0].transform;
    }
    let last_idx = keyframes.len() - 1;
    if time >= keyframes[last_idx].time {
        return keyframes[last_idx].transform;
    }

    // Binary search for the surrounding pair.
    let mut lo = 0usize;
    let mut hi = last_idx;
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        if time < keyframes[mid].time {
            hi = mid;
        } else {
            lo = mid;
        }
    }

    let prev = &keyframes[lo];
    let next = &keyframes[hi];
    let segment_dt = next.time - prev.time;
    let t = if segment_dt > 0.0 {
        ((time - prev.time) / segment_dt).clamp(0.0, 1.0)
    } else {
        0.0
    };

    BoneTransform {
        translation: prev.transform.translation.lerp(next.transform.translation, t),
        rotation: prev.transform.rotation.slerp(next.transform.rotation, t),
        scale: prev.transform.scale.lerp(next.transform.scale, t),
    }
}
