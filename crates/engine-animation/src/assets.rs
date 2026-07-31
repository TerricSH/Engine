use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Joint
// ---------------------------------------------------------------------------

/// A single joint in a skeleton hierarchy.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Joint {
    pub name: String,
    pub parent_index: Option<u32>,
    pub local_transform: JointTransform,
}

// ---------------------------------------------------------------------------
// JointTransform
// ---------------------------------------------------------------------------

/// Local-space SRT transform for a joint.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JointTransform {
    pub translation: [f32; 3],
    pub rotation: [f32; 4], // quaternion (x, y, z, w)
    pub scale: [f32; 3],
}

impl JointTransform {
    pub const IDENTITY: Self = Self {
        translation: [0.0, 0.0, 0.0],
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale: [1.0, 1.0, 1.0],
    };

    fn validate(&self, context: &str, require_invertible_scale: bool) -> Result<(), String> {
        if !self.translation.iter().all(|value| value.is_finite()) {
            return Err(format!("{context} has a non-finite translation"));
        }
        if !self.rotation.iter().all(|value| value.is_finite()) {
            return Err(format!("{context} has a non-finite rotation"));
        }
        let rotation_length_squared: f32 = self.rotation.iter().map(|value| value * value).sum();
        if !rotation_length_squared.is_finite() || rotation_length_squared <= f32::EPSILON {
            return Err(format!("{context} has a zero-length rotation"));
        }
        if !self.scale.iter().all(|value| value.is_finite()) {
            return Err(format!("{context} has a non-finite scale"));
        }
        if require_invertible_scale && self.scale.iter().any(|value| value.abs() <= f32::EPSILON) {
            return Err(format!("{context} has a non-invertible rest scale"));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Skeleton (asset)
// ---------------------------------------------------------------------------

/// A skeleton asset — joints in hierarchy order plus inverse bind matrices.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SkeletonAsset {
    pub joints: Vec<Joint>,
    pub inverse_bind_matrices: Vec<[[f32; 4]; 4]>,
}

/// Backwards-compatible asset name for [`SkeletonAsset`].
pub type Skeleton = SkeletonAsset;

impl SkeletonAsset {
    /// Number of joints in this skeleton.
    pub fn joint_count(&self) -> usize {
        self.joints.len()
    }

    /// Iterate joints in a parent-before-child order suitable for hierarchical
    /// solves.  This is guaranteed by construction (parents have lower indices).
    pub fn joints(&self) -> &[Joint] {
        &self.joints
    }

    /// Validate hierarchy ordering and bind-pose data before runtime conversion.
    pub fn validate(&self) -> Result<(), String> {
        if self.joints.len() > u16::MAX as usize + 1 {
            return Err(format!(
                "skeleton has {} joints; maximum supported is {}",
                self.joints.len(),
                u16::MAX as usize + 1
            ));
        }
        if self.inverse_bind_matrices.len() != self.joints.len() {
            return Err(format!(
                "skeleton has {} joints but {} inverse bind matrices",
                self.joints.len(),
                self.inverse_bind_matrices.len()
            ));
        }

        for (joint_index, joint) in self.joints.iter().enumerate() {
            if let Some(parent_index) = joint.parent_index {
                if parent_index as usize >= joint_index {
                    return Err(format!(
                        "joint {joint_index} has invalid parent {parent_index}; parents must precede children"
                    ));
                }
            }
            joint
                .local_transform
                .validate(&format!("joint {joint_index}"), true)?;
        }
        for (matrix_index, matrix) in self.inverse_bind_matrices.iter().enumerate() {
            if !matrix.iter().flatten().all(|value| value.is_finite()) {
                return Err(format!(
                    "inverse bind matrix {matrix_index} contains a non-finite value"
                ));
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Keyframe
// ---------------------------------------------------------------------------

/// A single keyframe storing a value at a specific time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AssetKeyframe<T> {
    pub time: f32,
    pub value: T,
}

/// Backwards-compatible asset name for [`AssetKeyframe`].
pub type Keyframe<T> = AssetKeyframe<T>;

// ---------------------------------------------------------------------------
// AnimationChannel
// ---------------------------------------------------------------------------

/// A channel animating a single joint's translation, rotation, and scale.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnimationChannel {
    pub joint_index: u32,
    pub translations: Vec<AssetKeyframe<[f32; 3]>>,
    pub rotations: Vec<AssetKeyframe<[f32; 4]>>,
    pub scales: Vec<AssetKeyframe<[f32; 3]>>,
}

// ---------------------------------------------------------------------------
// AnimationClip (asset)
// ---------------------------------------------------------------------------

/// An animation clip asset — a named collection of channels plus duration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnimationClipAsset {
    pub name: String,
    pub duration: f32,
    pub channels: Vec<AnimationChannel>,
    /// Maps each channel to a skeleton joint index.
    pub joint_indices: Vec<u32>,
}

/// Backwards-compatible asset name for [`AnimationClipAsset`].
pub type AnimationClip = AnimationClipAsset;

impl AnimationClipAsset {
    /// Total duration in seconds.
    pub fn duration(&self) -> f32 {
        self.duration
    }

    /// Human-readable name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Validate timing and transform values before the clip reaches evaluation.
    pub fn validate(&self) -> Result<(), String> {
        if !self.duration.is_finite() || self.duration < 0.0 {
            return Err(format!(
                "animation clip '{}' has invalid duration {}",
                self.name, self.duration
            ));
        }

        for (channel_index, channel) in self.channels.iter().enumerate() {
            validate_vec3_track(
                &channel.translations,
                self.duration,
                channel_index,
                "translation",
            )?;
            validate_quat_track(&channel.rotations, self.duration, channel_index, "rotation")?;
            validate_vec3_track(&channel.scales, self.duration, channel_index, "scale")?;
        }
        Ok(())
    }
}

fn validate_keyframe_times<T>(
    keyframes: &[AssetKeyframe<T>],
    duration: f32,
    channel_index: usize,
    track_name: &str,
) -> Result<(), String> {
    let mut previous_time = None;
    for (keyframe_index, keyframe) in keyframes.iter().enumerate() {
        if !keyframe.time.is_finite() || keyframe.time < 0.0 || keyframe.time > duration {
            return Err(format!(
                "channel {channel_index} {track_name} keyframe {keyframe_index} has invalid time {}",
                keyframe.time
            ));
        }
        if previous_time.is_some_and(|previous| keyframe.time < previous) {
            return Err(format!(
                "channel {channel_index} {track_name} keyframes are not sorted"
            ));
        }
        previous_time = Some(keyframe.time);
    }
    Ok(())
}

fn validate_vec3_track(
    keyframes: &[AssetKeyframe<[f32; 3]>],
    duration: f32,
    channel_index: usize,
    track_name: &str,
) -> Result<(), String> {
    validate_keyframe_times(keyframes, duration, channel_index, track_name)?;
    if keyframes
        .iter()
        .any(|keyframe| !keyframe.value.iter().all(|value| value.is_finite()))
    {
        return Err(format!(
            "channel {channel_index} {track_name} track contains a non-finite value"
        ));
    }
    Ok(())
}

fn validate_quat_track(
    keyframes: &[AssetKeyframe<[f32; 4]>],
    duration: f32,
    channel_index: usize,
    track_name: &str,
) -> Result<(), String> {
    validate_keyframe_times(keyframes, duration, channel_index, track_name)?;
    for keyframe in keyframes {
        let length_squared: f32 = keyframe.value.iter().map(|value| value * value).sum();
        if !keyframe.value.iter().all(|value| value.is_finite())
            || !length_squared.is_finite()
            || length_squared <= f32::EPSILON
        {
            return Err(format!(
                "channel {channel_index} {track_name} track contains an invalid quaternion"
            ));
        }
    }
    Ok(())
}
