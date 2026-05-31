use glam::{Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};
use std::ops::Mul;
use thiserror::Error;

use crate::Pose;

// ---------------------------------------------------------------------------
// BoneIndex — opaque handle into a Skeleton's bone array
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct BoneIndex(pub u16);

// ---------------------------------------------------------------------------
// BoneTransform — scale, rotation, translation (SRT), identity constant,
//                 multiplication (parent * child composition), and Mat4
//                 conversion.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BoneTransform {
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl BoneTransform {
    /// Identity transform: no translation, no rotation, unit scale.
    pub const IDENTITY: Self = Self {
        translation: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale: Vec3::ONE,
    };

    /// Convert to a 4×4 column-major affine matrix in SRT order.
    /// Equivalent to `Mat4::from_scale_rotation_translation(scale, rotation, translation)`.
    #[inline]
    pub fn to_mat4(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }
}

/// Compose two SRT transforms: `self` (parent) applied first, then `rhs` (child).
///
/// The result is the combined affine transform:
/// - `translation = parent.t + parent.r * (parent.s * child.t)`
/// - `rotation    = parent.r * child.r`
/// - `scale       = parent.s * child.s`
impl Mul for BoneTransform {
    type Output = Self;

    #[inline]
    fn mul(self, rhs: Self) -> Self {
        Self {
            translation: self.translation + self.rotation * (self.scale * rhs.translation),
            rotation: self.rotation * rhs.rotation,
            scale: self.scale * rhs.scale,
        }
    }
}

// ---------------------------------------------------------------------------
// AnimationError — typed error enum (thiserror, per FD-032)
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum AnimationError {
    #[error("bone not found: {0}")]
    BoneNotFound(String),

    #[error("clip not found: {0}")]
    ClipNotFound(String),

    #[error("invalid blend weight: {0} (expected 0.0–1.0)")]
    InvalidBlendWeight(f32),

    #[error("pose bone count mismatch: {0} vs {1}")]
    PoseBoneCountMismatch(usize, usize),
}

// ---------------------------------------------------------------------------
// Skeleton — bone hierarchy stored in AOS order; children adjacency is
//            maintained on insert.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BoneData {
    name: String,
    parent: Option<BoneIndex>,
    rest_transform: BoneTransform,
    children: Vec<BoneIndex>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skeleton {
    name: String,
    bones: Vec<BoneData>,
}

impl Skeleton {
    /// Create an empty skeleton with a human-readable name.
    pub fn new(name: String) -> Self {
        tracing::debug!(skeleton = %name, "Skeleton created");
        Self {
            name,
            bones: Vec::new(),
        }
    }

    /// Add a bone to the hierarchy.
    ///
    /// - `parent`: `None` for a root bone, `Some(index)` to attach to an existing bone.
    /// - `name`: human-readable bone name.
    /// - `rest_transform`: the bone's local-space rest (bind) pose.
    ///
    /// Returns the newly allocated `BoneIndex`.
    pub fn add_bone(
        &mut self,
        parent: Option<BoneIndex>,
        name: String,
        rest_transform: BoneTransform,
    ) -> BoneIndex {
        let idx = BoneIndex(self.bones.len() as u16);

        // Register this bone as a child of its parent.
        if let Some(p) = parent {
            if let Some(parent_data) = self.bones.get_mut(p.0 as usize) {
                parent_data.children.push(idx);
            }
        }

        self.bones.push(BoneData {
            name,
            parent,
            rest_transform,
            children: Vec::new(),
        });

        tracing::debug!(
            skeleton = %self.name,
            bone = idx.0,
            "Bone added to skeleton"
        );

        idx
    }

    /// Number of bones in this skeleton.
    pub fn bone_count(&self) -> usize {
        self.bones.len()
    }

    /// Human-readable name of the bone at `index`, or `None` if out of range.
    pub fn bone_name(&self, index: BoneIndex) -> Option<&str> {
        self.bones.get(index.0 as usize).map(|b| b.name.as_str())
    }

    /// The parent of `index`, or `None` if it is a root bone.
    pub fn parent_of(&self, index: BoneIndex) -> Option<BoneIndex> {
        self.bones.get(index.0 as usize).and_then(|b| b.parent)
    }

    /// Slice of immediate children of `index`. Returns an empty slice if the
    /// bone is out of range or has no children.
    pub fn children_of(&self, index: BoneIndex) -> &[BoneIndex] {
        self.bones
            .get(index.0 as usize)
            .map(|b| b.children.as_slice())
            .unwrap_or(&[])
    }

    /// Build a `Pose` initialised with every bone's rest (bind) transform.
    pub fn rest_pose(&self) -> Pose {
        let local: Vec<BoneTransform> = self.bones.iter().map(|b| b.rest_transform).collect();
        Pose { local }
    }

    /// Create a runtime `Skeleton` from an `assets::Skeleton` by converting
    /// each joint into a bone with the correct parent hierarchy.
    pub fn from_asset(asset: &crate::assets::Skeleton) -> Self {
        use glam::{Quat, Vec3};
        let mut skel = Self::new("imported".into());
        for joint in &asset.joints {
            let parent = joint.parent_index.map(|p| BoneIndex(p as u16));
            let t = joint.local_transform.translation;
            let r = joint.local_transform.rotation;
            let s = joint.local_transform.scale;
            skel.add_bone(
                parent,
                joint.name.clone(),
                BoneTransform {
                    translation: Vec3::from(t),
                    rotation: Quat::from_array(r),
                    scale: Vec3::from(s),
                },
            );
        }
        skel
    }
}
