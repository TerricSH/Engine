use glam::{Quat, Vec3};

use crate::assets::{self, JointTransform};
use tracing;
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
            rotation: Quat::from_array(jt.rotation),
            scale: Vec3::from(jt.scale),
        }
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

    for joint in &asset_skel.joints {
        // Convert parent index: asset uses u32, runtime uses u16 via BoneIndex.
        // Clamp to u16::MAX as a safe fallback if asset data is corrupted.
        let parent = joint.parent_index.map(|p| {
            BoneIndex(p.min(u16::MAX as u32) as u16)
        });
        let bone_idx = runtime.add_bone(
            parent,
            joint.name.clone(),
            BoneTransform::from(joint.local_transform.clone()),
        );
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
        // Bounds check: if the asset references a joint beyond the skeleton,
        // fall back to bone 0 (root) to avoid a panic on malformed data.
        let bone = if joint_idx < joint_map.len() {
            joint_map[joint_idx]
        } else {
            tracing::warn!(
                "clip '{}' references joint index {} but skeleton has {} joints",
                asset_clip.name,
                joint_idx,
                joint_map.len()
            );
            joint_map[0]
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
                    rotation: Quat::from_array(r.value),
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
    pose
        .local_transforms()
        .iter()
        .map(|bt| JointTransform::from(bt))
        .collect()
}
