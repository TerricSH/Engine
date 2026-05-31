//! Foot IK grounding — corrects foot positions so they contact the ground
//! during locomotion.
//!
//! This module provides a standalone [`apply_foot_ik`] function that uses the
//! existing IK solver (FABRIK/CCD) to adjust a [`Pose`] so that configured
//! foot bones rest on the ground surface.
//!
//! # Usage
//!
//! ```ignore
//! use engine_animation::foot_ik::{FootIkConfig, apply_foot_ik};
//!
//! let mut config = FootIkConfig::default();
//! config.foot_bones = vec![left_foot_bone, right_foot_bone];
//! config.foot_chains = vec![left_leg_chain, right_leg_chain];
//!
//! let corrected = apply_foot_ik(&mut pose, &skeleton, &config, &|x, z| {
//!     Some(ground_height_at(x, z))
//! });
//! ```

use glam::Vec3;

use crate::ik::{
    solve_pose_multi, IkChain, IkConstraintSet, IkEffector,
};
use crate::skeleton::Skeleton;
use crate::{BoneIndex, Pose};

// ---------------------------------------------------------------------------
// FootIkConfig
// ---------------------------------------------------------------------------

/// Configuration for foot IK grounding.
///
/// # Fields
///
/// | Field               | Type              | Description                                       |
/// |---------------------|-------------------|---------------------------------------------------|
/// | `foot_bones`        | `Vec<BoneIndex>`  | Bone indices for the feet (typically left/right)  |
/// | `foot_chains`       | `Vec<IkChain>`    | IK chain per foot (tip→base: foot→shin→thigh→hip) |
/// | `ray_origin_offset` | `f32`             | Y-offset above foot bone for raycast origin       |
/// | `ray_max_distance`  | `f32`             | Max downward raycast distance from offset origin  |
/// | `blend_weight`      | `f32`             | IK correction blend weight `[0, 1]`               |
/// | `bone_mask`         | `Vec<u16>`        | If non-empty, only these bone indices are modified |
pub struct FootIkConfig {
    /// Bone indices for the feet (typically left foot, right foot).
    pub foot_bones: Vec<BoneIndex>,
    /// IK chain for each foot (tip→base order, e.g. foot → shin → thigh → hip).
    pub foot_chains: Vec<IkChain>,
    /// Raycast origin offset above the foot bone (in model space Y units).
    pub ray_origin_offset: f32,
    /// Maximum raycast distance downward from the offset origin.
    pub ray_max_distance: f32,
    /// Blend weight for IK correction `[0, 1]`.
    pub blend_weight: f32,
    /// Bone mask — only these bones are modified by foot IK.
    /// If empty, all chain bones are affected.
    pub bone_mask: Vec<u16>,
}

impl Default for FootIkConfig {
    fn default() -> Self {
        Self {
            foot_bones: Vec::new(),
            foot_chains: Vec::new(),
            ray_origin_offset: 0.3,
            ray_max_distance: 1.0,
            blend_weight: 1.0,
            bone_mask: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// apply_foot_ik
// ---------------------------------------------------------------------------

/// Apply foot IK grounding to a pose.
///
/// For each configured foot bone:
/// 1. Gets the foot's world-space position from the current pose
/// 2. Calls `sample_ground` to get the ground height at that `(x, z)` position
/// 3. If ground is within range, creates an IK effector at the corrected position
/// 4. Runs IK solve for the foot chain (saved + restored for bone_mask support)
///
/// `sample_ground` is a callback that returns the Y height at a given `(x, z)`
/// world position, or `None` if no ground is detected at that location.
///
/// Returns `true` if any foot was corrected.
pub fn apply_foot_ik(
    pose: &mut Pose,
    skel: &Skeleton,
    config: &FootIkConfig,
    sample_ground: &dyn Fn(f32, f32) -> Option<f32>,
) -> bool {
    if config.foot_bones.is_empty() || config.foot_chains.is_empty() {
        return false;
    }

    let global = pose.global_transforms(skel);
    let mut any_corrected = false;

    // Snapshot local transforms so we can restore non-masked bones later.
    let orig_local = if !config.bone_mask.is_empty() {
        Some(pose.local_transforms().to_vec())
    } else {
        None
    };

    for (foot_idx, foot_bone) in config.foot_bones.iter().enumerate() {
        let foot_pos = global[foot_bone.0 as usize].translation;

        // Raycast downward from above the foot.
        if let Some(ground_y) = sample_ground(foot_pos.x, foot_pos.z) {
            let foot_to_ground = foot_pos.y - ground_y;
            if foot_to_ground > 0.01 && foot_to_ground < config.ray_max_distance {
                if let Some(chain) = config.foot_chains.get(foot_idx) {
                    let effector = IkEffector::new(
                        format!("foot_ik_{}", foot_idx),
                        *foot_bone,
                        Vec3::new(foot_pos.x, ground_y, foot_pos.z),
                    )
                    .with_weight(config.blend_weight);

                    let constraints = IkConstraintSet::new();
                    solve_pose_multi(pose, skel, &[chain.clone()], &[effector], &constraints);
                    any_corrected = true;
                }
            }
        }
    }

    // Restore any bones that are not in the bone mask.
    if let Some(ref saved) = orig_local {
        if !config.bone_mask.is_empty() {
            let local = pose.local_transforms_mut();
            for (i, bt) in local.iter_mut().enumerate() {
                if !config.bone_mask.contains(&(i as u16)) {
                    *bt = saved[i];
                }
            }
        }
    }

    any_corrected
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ik::IkSolverType;
    use crate::skeleton::Skeleton as RuntimeSkeleton;
    use crate::{BoneIndex, BoneTransform, Pose};
    use glam::{Quat, Vec3};

    /// A simple 4-bone leg chain: root → hip → knee → foot.
    /// Bones are offset so that the knee bends forward naturally:
    ///   root: (0, 0, 0)
    ///   hip:  (0, 1, 0)   — child of root
    ///   knee: (0.2, 0.5, 0) — child of hip (forward of hip by 0.2 in X)
    ///   foot: (0, 0, 0)   — child of knee
    fn leg_skeleton() -> (RuntimeSkeleton, Vec<BoneIndex>) {
        let mut sk = RuntimeSkeleton::new("leg".into());
        let root = sk.add_bone(None, "root".into(), BoneTransform::IDENTITY);
        let hip = sk.add_bone(
            Some(root),
            "hip".into(),
            BoneTransform {
                translation: Vec3::new(0.0, 1.0, 0.0),
                rotation: Quat::IDENTITY,
                scale: Vec3::ONE,
            },
        );
        let knee = sk.add_bone(
            Some(hip),
            "knee".into(),
            BoneTransform {
                translation: Vec3::new(0.2, -0.5, 0.0),
                rotation: Quat::IDENTITY,
                scale: Vec3::ONE,
            },
        );
        let foot = sk.add_bone(
            Some(knee),
            "foot".into(),
            BoneTransform {
                translation: Vec3::new(0.0, -0.5, 0.0),
                rotation: Quat::IDENTITY,
                scale: Vec3::ONE,
            },
        );
        // Chain in tip→base order: foot → knee → hip → root
        (sk, vec![foot, knee, hip, root])
    }

    /// A mock ground sampler that returns ground at Y=0 for all positions.
    fn flat_ground(_x: f32, _z: f32) -> Option<f32> {
        Some(0.0)
    }

    /// A mock ground sampler that returns no ground.
    fn no_ground(_x: f32, _z: f32) -> Option<f32> {
        None
    }

    #[test]
    fn foot_ik_empty_config_no_crash() {
        let (sk, _) = leg_skeleton();
        let mut pose = Pose::new(&sk);
        let config = FootIkConfig::default();
        let result = apply_foot_ik(&mut pose, &sk, &config, &flat_ground);
        assert!(!result, "empty config should not correct anything");
    }

    #[test]
    fn foot_ik_no_foot_bones_no_crash() {
        let (sk, chain_bones) = leg_skeleton();
        let mut pose = Pose::new(&sk);

        let chain = IkChain::new("leg_r", chain_bones)
            .with_solver(IkSolverType::Fabrik)
            .with_iterations(20)
            .with_tolerance(0.01);

        let config = FootIkConfig {
            foot_bones: vec![],
            foot_chains: vec![chain],
            ..Default::default()
        };

        let result = apply_foot_ik(&mut pose, &sk, &config, &flat_ground);
        assert!(!result, "no foot bones should not correct anything");
    }

    #[test]
    fn foot_ik_no_ground_no_change() {
        let (sk, chain_bones) = leg_skeleton();
        let mut pose = Pose::new(&sk);
        let orig = pose.clone();

        let chain = IkChain::new("leg_r", chain_bones)
            .with_solver(IkSolverType::Fabrik)
            .with_iterations(20)
            .with_tolerance(0.01);

        let foot_bone = BoneIndex(3);
        let config = FootIkConfig {
            foot_bones: vec![foot_bone],
            foot_chains: vec![chain],
            ..Default::default()
        };

        // When ground returns None, the pose should be unchanged.
        let result = apply_foot_ik(&mut pose, &sk, &config, &no_ground);
        assert!(!result, "no ground should not correct anything");

        let og = orig.global_transforms(&sk);
        let ng = pose.global_transforms(&sk);
        for i in 0..og.len() {
            assert!(
                (og[i].translation - ng[i].translation).length() < 1e-6,
                "bone {} moved when no ground",
                i
            );
        }
    }

    #[test]
    fn foot_ik_corrects_foot_toward_ground() {
        let (sk, chain_bones) = leg_skeleton();
        let mut pose = Pose::new(&sk);

        // At rest, foot (bone 3) global Y = 0.0 (root at 0, hip at 1, knee at 0.5, foot at 0).
        // The foot is already at ground level (Y=0). To test correction we need
        // the foot above ground.  Lift the root so the foot is in the air.
        pose.local_transforms_mut()[0].translation.y = 1.0;

        let foot_bone = BoneIndex(3);
        let chain = IkChain::new("leg_r", chain_bones)
            .with_solver(IkSolverType::Fabrik)
            .with_iterations(20)
            .with_tolerance(0.01);

        let config = FootIkConfig {
            foot_bones: vec![foot_bone],
            foot_chains: vec![chain],
            ray_origin_offset: 0.3,
            ray_max_distance: 2.0,
            blend_weight: 1.0,
            bone_mask: Vec::new(),
        };

        let foot_y_before = pose.global_transforms(&sk)[foot_bone.0 as usize].translation.y;
        // foot_y_before should be 1.0 (root lifted by 1.0, foot at rest Y = 0 → global = 1.0)
        assert!(
            (foot_y_before - 1.0).abs() < 1e-5,
            "expected foot Y ≈ 1.0 before IK, got {}",
            foot_y_before
        );

        let result = apply_foot_ik(&mut pose, &sk, &config, &flat_ground);

        assert!(result, "foot IK should have corrected the pose");

        let foot_y_after = pose.global_transforms(&sk)[foot_bone.0 as usize].translation.y;
        assert!(
            foot_y_after < 0.1,
            "foot Y should be near ground (≈0.0) after IK, got {}",
            foot_y_after
        );
    }

    #[test]
    fn foot_ik_bone_mask_allows_full_chain_modification() {
        let (sk, chain_bones) = leg_skeleton();
        let mut pose = Pose::new(&sk);

        // Lift the root so foot is above ground.
        pose.local_transforms_mut()[0].translation.y = 1.0;

        let foot_bone = BoneIndex(3);

        // Chain: foot(3) → knee(2) → hip(1) → root(0)
        let chain = IkChain::new("leg_r", chain_bones)
            .with_solver(IkSolverType::Fabrik)
            .with_iterations(20)
            .with_tolerance(0.01);

        // Bone mask includes ALL chain bones — IK should work as if unmasked.
        // This verifies the mask mechanism doesn't break valid configurations.
        let config = FootIkConfig {
            foot_bones: vec![foot_bone],
            foot_chains: vec![chain],
            ray_origin_offset: 0.3,
            ray_max_distance: 2.0,
            blend_weight: 1.0,
            bone_mask: vec![0, 1, 2, 3], // all chain bones
        };

        let foot_y_before = pose.global_transforms(&sk)[foot_bone.0 as usize].translation.y;
        assert!(
            (foot_y_before - 1.0).abs() < 1e-5,
            "expected foot Y ≈ 1.0 before IK, got {}",
            foot_y_before
        );

        let result = apply_foot_ik(&mut pose, &sk, &config, &flat_ground);
        assert!(result, "foot IK should have corrected the pose");

        let foot_y_after = pose.global_transforms(&sk)[foot_bone.0 as usize].translation.y;
        assert!(
            foot_y_after < 0.1,
            "foot Y should be near ground after IK with full-chain mask, got {}",
            foot_y_after
        );
    }

    #[test]
    fn foot_ik_empty_mask_allows_all_bones_to_change() {
        let (sk, chain_bones) = leg_skeleton();
        let mut pose = Pose::new(&sk);
        pose.local_transforms_mut()[0].translation.y = 1.0;

        let foot_bone = BoneIndex(3);
        let chain = IkChain::new("leg_r", chain_bones)
            .with_solver(IkSolverType::Fabrik)
            .with_iterations(20)
            .with_tolerance(0.01);

        // Empty bone mask = all bones can be modified.
        let config = FootIkConfig {
            foot_bones: vec![foot_bone],
            foot_chains: vec![chain],
            ray_origin_offset: 0.3,
            ray_max_distance: 2.0,
            blend_weight: 1.0,
            bone_mask: Vec::new(), // empty = all bones affected
        };

        let orig = pose.clone();
        let result = apply_foot_ik(&mut pose, &sk, &config, &flat_ground);
        assert!(result, "foot IK should have corrected the pose");

        // At least one bone should have changed (the foot was lifted, IK should
        // have modified something to bring it toward ground).
        let orig_local = orig.local_transforms();
        let final_local = pose.local_transforms();
        let any_changed = (0..sk.bone_count()).any(|i| {
            (orig_local[i].translation - final_local[i].translation).length() > 1e-6
                || orig_local[i].rotation.angle_between(final_local[i].rotation) > 1e-6
        });
        assert!(any_changed, "at least one bone should have changed with empty mask");

        let foot_y = pose.global_transforms(&sk)[foot_bone.0 as usize].translation.y;
        assert!(
            foot_y < 0.1,
            "foot Y should be near ground after IK with empty mask, got {}",
            foot_y
        );
    }

    #[test]
    fn foot_ik_blend_weight_partial_correction() {
        let (sk, chain_bones) = leg_skeleton();
        let mut pose = Pose::new(&sk);
        pose.local_transforms_mut()[0].translation.y = 1.0;

        let foot_bone = BoneIndex(3);
        let chain = IkChain::new("leg_r", chain_bones)
            .with_solver(IkSolverType::Fabrik)
            .with_iterations(20)
            .with_tolerance(0.01);

        let config = FootIkConfig {
            foot_bones: vec![foot_bone],
            foot_chains: vec![chain],
            ray_origin_offset: 0.3,
            ray_max_distance: 2.0,
            blend_weight: 0.5, // 50% blend
            bone_mask: Vec::new(),
        };

        let foot_y_before = pose.global_transforms(&sk)[foot_bone.0 as usize].translation.y;
        assert!(
            (foot_y_before - 1.0).abs() < 1e-5,
            "expected foot Y ≈ 1.0 before IK, got {}",
            foot_y_before
        );

        let result = apply_foot_ik(&mut pose, &sk, &config, &flat_ground);
        assert!(result, "foot IK should have corrected the pose");

        let foot_y_after = pose.global_transforms(&sk)[foot_bone.0 as usize].translation.y;
        // With 50% blend, foot should be roughly halfway between original (1.0) and ground (0.0).
        // Due to solver dynamics, it may not be exactly 0.5, but should be significantly lower
        // than before and higher than with full weight.
        assert!(
            foot_y_after > 0.0 && foot_y_after < 0.8,
            "partial blend foot Y should be between 0 and 0.8, got {}",
            foot_y_after
        );
    }

    #[test]
    fn foot_ik_out_of_range_not_corrected() {
        let (sk, chain_bones) = leg_skeleton();
        let mut pose = Pose::new(&sk);

        // Lift foot way above ground (beyond ray_max_distance).
        pose.local_transforms_mut()[0].translation.y = 5.0;

        let foot_bone = BoneIndex(3);
        let chain = IkChain::new("leg_r", chain_bones)
            .with_solver(IkSolverType::Fabrik)
            .with_iterations(20)
            .with_tolerance(0.01);

        let config = FootIkConfig {
            foot_bones: vec![foot_bone],
            foot_chains: vec![chain],
            ray_origin_offset: 0.3,
            ray_max_distance: 1.0, // max distance = 1.0, foot is at Y ≈ 5.0
            ..Default::default()
        };

        let orig = pose.clone();
        let result = apply_foot_ik(&mut pose, &sk, &config, &flat_ground);
        assert!(!result, "foot too far should not be corrected");

        let og = orig.global_transforms(&sk);
        let ng = pose.global_transforms(&sk);
        for i in 0..og.len() {
            assert!(
                (og[i].translation - ng[i].translation).length() < 1e-6,
                "bone {} moved despite being out of range",
                i
            );
        }
    }
}
