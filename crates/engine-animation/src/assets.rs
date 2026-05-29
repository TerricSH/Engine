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
}

// ---------------------------------------------------------------------------
// Skeleton (asset)
// ---------------------------------------------------------------------------

/// A skeleton asset — joints in hierarchy order plus inverse bind matrices.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Skeleton {
    pub joints: Vec<Joint>,
    pub inverse_bind_matrices: Vec<[[f32; 4]; 4]>,
}

impl Skeleton {
    /// Number of joints in this skeleton.
    pub fn joint_count(&self) -> usize {
        self.joints.len()
    }

    /// Iterate joints in a parent-before-child order suitable for hierarchical
    /// solves.  This is guaranteed by construction (parents have lower indices).
    pub fn joints(&self) -> &[Joint] {
        &self.joints
    }
}

// ---------------------------------------------------------------------------
// Keyframe
// ---------------------------------------------------------------------------

/// A single keyframe storing a value at a specific time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Keyframe<T> {
    pub time: f32,
    pub value: T,
}

// ---------------------------------------------------------------------------
// AnimationChannel
// ---------------------------------------------------------------------------

/// A channel animating a single joint's translation, rotation, and scale.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnimationChannel {
    pub joint_index: u32,
    pub translations: Vec<Keyframe<[f32; 3]>>,
    pub rotations: Vec<Keyframe<[f32; 4]>>,
    pub scales: Vec<Keyframe<[f32; 3]>>,
}

// ---------------------------------------------------------------------------
// AnimationClip (asset)
// ---------------------------------------------------------------------------

/// An animation clip asset — a named collection of channels plus duration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnimationClip {
    pub name: String,
    pub duration: f32,
    pub channels: Vec<AnimationChannel>,
    /// Maps each channel to a skeleton joint index.
    pub joint_indices: Vec<u32>,
}

impl AnimationClip {
    /// Total duration in seconds.
    pub fn duration(&self) -> f32 {
        self.duration
    }

    /// Human-readable name.
    pub fn name(&self) -> &str {
        &self.name
    }
}
