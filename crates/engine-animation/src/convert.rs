use glam::{Quat, Vec3};

use crate::assets::{self, JointTransform};
use crate::clip;
use crate::pose::Pose;
use crate::skeleton::{self, BoneIndex, BoneTransform};

// ---------------------------------------------------------------------------
// JointTransform ↔ BoneTransform
// ---------------------------------------------------------------------------

impl From<assets::JointTransform> for BoneTransform {
    fn from(jt: assets::JointTransform) -> Self {
        Self {
            translation: Vec3::from(jt.translation),
            rotation: safe_quat(jt.rotation),
            scale: Vec3::from(jt.scale),
        }
    }
}

fn safe_quat(value: [f32; 4]) -> Quat {
    let quat = Quat::from_array(value);
    let length_squared = quat.length_squared();
    if quat.is_finite() && length_squared.is_finite() && length_squared > f32::EPSILON {
        quat / length_squared.sqrt()
    } else {
        Quat::IDENTITY
    }
}

impl From<&BoneTransform> for assets::JointTransform {
    fn from(bt: &BoneTransform) -> Self {
        Self {
            translation: bt.translation.into(),
            rotation: bt.rotation.into(),
            scale: bt.scale.into(),
        }
    }
}

// ---------------------------------------------------------------------------
// Skeleton conversion: asset → runtime
// ---------------------------------------------------------------------------

/// Convert an asset [`Skeleton`] into a runtime [`skeleton::Skeleton`].
///
/// Returns the runtime skeleton and a mapping from asset joint index (0-based
/// position in [`assets::Skeleton::joints`]) to [`BoneIndex`].
pub fn skeleton_asset_to_runtime(
    asset_skel: &assets::Skeleton,
) -> (skeleton::Skeleton, Vec<BoneIndex>) {
    let mut runtime = skeleton::Skeleton::new("converted".into());
    let mut joint_map = Vec::with_capacity(asset_skel.joints.len());
    if asset_skel.joints.len() > u16::MAX as usize + 1 {
        tracing::warn!(
            joints = asset_skel.joints.len(),
            maximum = u16::MAX as usize + 1,
            "truncating skeleton that exceeds BoneIndex capacity"
        );
    }

    for (joint_index, joint) in asset_skel
        .joints
        .iter()
        .take(u16::MAX as usize + 1)
        .enumerate()
    {
        let parent = match joint.parent_index {
            Some(parent_index) if (parent_index as usize) < joint_map.len() => {
                Some(joint_map[parent_index as usize])
            }
            Some(parent_index) => {
                tracing::warn!(
                    joint = joint_index,
                    parent = parent_index,
                    "ignoring invalid skeleton parent; parents must precede children"
                );
                None
            }
            None => None,
        };
        let bone_idx = runtime.add_bone(
            parent,
            joint.name.clone(),
            BoneTransform::from(joint.local_transform.clone()),
        );
        if let Some(inverse_bind_matrix) =
            asset_skel.inverse_bind_matrices.get(joint_index).copied()
        {
            runtime.set_inverse_bind_matrix(bone_idx, inverse_bind_matrix);
        }
        joint_map.push(bone_idx);
    }

    (runtime, joint_map)
}

// ---------------------------------------------------------------------------
// AnimationClip conversion: asset → runtime
// ---------------------------------------------------------------------------

/// Convert an asset [`AnimationClip`] into a runtime [`clip::AnimationClip`].
///
/// `joint_map` is the index mapping returned by [`skeleton_asset_to_runtime`]
/// — it maps asset joint indices to the corresponding [`BoneIndex`] values.
///
/// Each asset channel stores translations, rotations, and scales as three
/// parallel keyframe arrays.  This function zips them by index into a single
/// [`clip::Keyframe`] stream where each keyframe holds a complete
/// [`BoneTransform`].
pub fn clip_asset_to_runtime(
    asset_clip: &assets::AnimationClip,
    joint_map: &[BoneIndex],
) -> clip::AnimationClip {
    let mut runtime = clip::AnimationClip::new(asset_clip.name.clone(), asset_clip.duration);

    for channel in &asset_clip.channels {
        // Map asset joint index through the joint map.
        let joint_idx = channel.joint_index as usize;
        let Some(&bone) = joint_map.get(joint_idx) else {
            tracing::warn!(
                "clip '{}' references joint index {} but skeleton has {} joints",
                asset_clip.name,
                joint_idx,
                joint_map.len()
            );
            continue;
        };

        // Zip the three parallel SRT tracks together by index.
        // All three tracks should have the same number of keyframes with
        // matching times at corresponding positions.
        let count = channel
            .translations
            .len()
            .min(channel.rotations.len())
            .min(channel.scales.len());

        let mut keyframes = Vec::with_capacity(count);
        for i in 0..count {
            let t = &channel.translations[i];
            let r = &channel.rotations[i];
            let s = &channel.scales[i];

            keyframes.push(clip::Keyframe {
                time: t.time,
                transform: BoneTransform {
                    translation: Vec3::from(t.value),
                    rotation: safe_quat(r.value),
                    scale: Vec3::from(s.value),
                },
            });
        }

        runtime.add_channel(bone, keyframes);
    }

    runtime
}

// ---------------------------------------------------------------------------
// Pose ↔ Vec<JointTransform>
// ---------------------------------------------------------------------------

/// Build a [`Pose`] from a slice of asset [`JointTransform`]s.
///
/// The transforms are mapped in order — index 0 becomes bone 0, etc.  This is
/// appropriate when the transforms are already in skeleton-major order.
pub fn pose_from_joint_transforms(transforms: &[JointTransform]) -> Pose {
    Pose {
        local: transforms
            .iter()
            .map(|jt| BoneTransform::from(jt.clone()))
            .collect(),
    }
}

/// Decompose a [`Pose`] back into a [`Vec<JointTransform>`].
///
/// The order matches the skeleton's bone ordering (index 0 → bone 0, etc.).
pub fn joint_transforms_from_pose(pose: &Pose) -> Vec<JointTransform> {
    pose.local_transforms()
        .iter()
        .map(JointTransform::from)
        .collect()
}
