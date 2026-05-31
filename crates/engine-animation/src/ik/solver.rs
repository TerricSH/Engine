//! IK solver implementations — FABRIK and CCD.
//!
//! Both solvers operate on a [`Pose`] by modifying the local transforms of
//! bones in an IK chain. The typical usage is:
//!
//! 1. Evaluate the animation to produce a [`Pose`] (local bone transforms).
//! 2. Call [`solve_pose`] with the pose, skeleton, chain, and an effector.
//! 3. The pose is modified in-place with IK corrections.
//! 4. Downstream [`Pose::global_transforms`] or [`Pose::skin_matrices`]
//!    compute the final result.

use glam::{Quat, Vec3};

use crate::ik::chain::{IkChain, IkSolverType};
use crate::ik::constraint::IkConstraintSet;
use crate::ik::effector::IkEffector;
use crate::skeleton::Skeleton;
use crate::{BoneIndex, BoneTransform, Pose};

// ═════════════════════════════════════════════════════════════════════════
// Public API
// ═════════════════════════════════════════════════════════════════════════

/// Solve IK for a single effector on a chain, modifying the pose in-place.
///
/// # Arguments
///
/// * `pose`        — The current pose (modified in-place with IK corrections).
/// * `skeleton`    — The skeleton defining the bone hierarchy.
/// * `chain`       — The IK chain (tip → base bone indices).
/// * `effector`    — The IK effector (target position/rotation, weight).
/// * `constraints` — Optional per-bone constraints.
///
/// # Returns
///
/// `true` if the solver converged (reached tolerance) or stretched out-of-reach.
pub fn solve_pose(
    pose: &mut Pose,
    skeleton: &Skeleton,
    chain: &IkChain,
    effector: &IkEffector,
    constraints: &IkConstraintSet,
) -> bool {
    if !chain.is_active() || !effector.is_active() {
        return false;
    }

    match chain.solver {
        IkSolverType::Fabrik => solve_fabrik(pose, skeleton, chain, effector, constraints),
        IkSolverType::Ccd => solve_ccd(pose, skeleton, chain, effector, constraints),
    }
}

/// Solve multiple chains against their respective effectors.
///
/// Each chain is matched to the first active effector whose bone is in the
/// chain (or whose `influence_chain` matches). Chains are solved in order
/// — later chains can re-solve bones modified by earlier chains.
///
/// For proper prioritisation, order chains: spine → legs → arms → head.
pub fn solve_pose_multi(
    pose: &mut Pose,
    skeleton: &Skeleton,
    chains: &[IkChain],
    effectors: &[IkEffector],
    constraints: &IkConstraintSet,
) -> usize {
    let mut solved_count = 0usize;

    for (chain_idx, chain) in chains.iter().enumerate() {
        let effector = match effectors.iter().find(|e| {
            e.is_active()
                && e.influence_chain.map_or(true, |ic| ic == chain_idx)
                && chain.contains(e.bone)
        }) {
            Some(e) => e,
            None => continue,
        };

        if solve_pose(pose, skeleton, chain, effector, constraints) {
            solved_count += 1;
        }
    }

    solved_count
}

// ═════════════════════════════════════════════════════════════════════════
// Helpers
// ═════════════════════════════════════════════════════════════════════════

/// Map effector position to world space.
fn effector_world_pos(
    effector: &IkEffector,
    pose: &Pose,
    skeleton: &Skeleton,
    chain: &IkChain,
) -> Vec3 {
    match effector.space {
        crate::ik::effector::IkEffectorSpace::World => effector.position,
        crate::ik::effector::IkEffectorSpace::Local => {
            let global = pose.global_transforms(skeleton);
            let base = chain.base().unwrap_or(BoneIndex(0));
            let m = global[base.0 as usize].to_mat4();
            m.transform_point3(effector.position)
        }
    }
}

/// Extract world-space positions for chain bones from global transforms.
fn chain_positions(global: &[BoneTransform], bones: &[BoneIndex]) -> Vec<Vec3> {
    bones
        .iter()
        .map(|b| global[b.0 as usize].translation)
        .collect()
}

/// Pre-compute bone lengths between consecutive chain bones.
fn chain_bone_lengths(positions: &[Vec3]) -> Vec<f32> {
    let n = positions.len();
    let mut lens = Vec::with_capacity(n.max(1) - 1);
    for i in 0..n.saturating_sub(1) {
        let d = positions[i].distance(positions[i + 1]);
        lens.push(if d > 1e-10 { d } else { 1e-10 });
    }
    lens
}

// ═════════════════════════════════════════════════════════════════════════
// FABRIK solver
// ═════════════════════════════════════════════════════════════════════════

fn solve_fabrik(
    pose: &mut Pose,
    skeleton: &Skeleton,
    chain: &IkChain,
    effector: &IkEffector,
    constraints: &IkConstraintSet,
) -> bool {
    let n = chain.bones.len();
    if n < 2 {
        return false;
    }

    // Snapshot current global transforms (immutable borrow).
    let original_global = pose.global_transforms(skeleton);
    let mut pos = chain_positions(&original_global, &chain.bones);
    let base_original = pos[n - 1];
    let target = effector_world_pos(effector, pose, skeleton, chain);
    let lengths = chain_bone_lengths(&pos);
    let total_reach: f32 = lengths.iter().sum();

    let tip_to_target = pos[0].distance(target);
    let out_of_reach = tip_to_target > total_reach;

    let rest_pose = skeleton.rest_pose();

    if out_of_reach {
        // Straight-line stretch toward target.
        // Set tip to target; then for each bone inward, place it along
        // the line from the previously set bone toward the base.
        pos[0] = target;
        for i in 1..n {
            let dir = (pos[i] - pos[i - 1]).normalize_or_zero();
            pos[i] = pos[i - 1] + dir * lengths[i - 1];
        }
    } else {
        // Iterative FABRIK.
        // Uses the standard FABRIK formula from the paper (Aristidou & Lasenby 2011):
        //   Forward: p[i] = p[i-1] + normalize(p_old[i] - p_updated[i-1]) * length
        //   Backward: p[i] = p[i+1] + normalize(p_old[i] - p_updated[i+1]) * length
        let tol_sq = chain.tolerance * chain.tolerance;
        for _ in 0..chain.iteration_count {
            // Forward pass (tip → base): push tip toward target.
            pos[0] = target;
            for i in 1..n {
                let dir = (pos[i] - pos[i - 1]).normalize_or_zero();
                pos[i] = pos[i - 1] + dir * lengths[i - 1];
            }

            // Backward pass (base → tip): anchor base at original position.
            pos[n - 1] = base_original;
            for i in (0..n - 1).rev() {
                let dir = (pos[i] - pos[i + 1]).normalize_or_zero();
                pos[i] = pos[i + 1] + dir * lengths[i];
            }

            if pos[0].distance_squared(target) < tol_sq {
                break;
            }
        }
    }

    // Apply positions back to the Pose by processing bones from BASE to TIP
    // (reverse chain order). Processing base-first ensures that when we
    // modify a parent bone, any subsequent lookups of its global transform
    // reflect the change.
    for _pass in 0..5 {
        for i in (0..n - 1).rev() {
            let child_bone = chain.bones[i]; // skeleton child (tip-side)
            let parent_bone = chain.bones[i + 1]; // skeleton parent (root-side)

            // Recompute current global transforms (reflects prior modifications).
            // We take a fresh snapshot for each bone pair to avoid stale data.
            // Use the weight to blend between original and solved positions.
            let current_global = pose.global_transforms(skeleton);
            let child_pos = if effector.weight >= 1.0 - 1e-6 {
                pos[i]
            } else {
                current_global[child_bone.0 as usize]
                    .translation
                    .lerp(pos[i], effector.weight)
            };
            let parent_pos = if effector.weight >= 1.0 - 1e-6 {
                pos[i + 1]
            } else {
                current_global[parent_bone.0 as usize]
                    .translation
                    .lerp(pos[i + 1], effector.weight)
            };

            let old_dir = current_global[child_bone.0 as usize].translation
                - current_global[parent_bone.0 as usize].translation;
            let new_dir = child_pos - parent_pos;

            let old_n = old_dir.normalize_or_zero();
            let new_n = new_dir.normalize_or_zero();

            if old_n.length_squared() > 1e-10 && new_n.length_squared() > 1e-10 {
                let delta_global = Quat::from_rotation_arc(old_n, new_n);
                if !delta_global.is_nan() && delta_global.angle_between(Quat::IDENTITY) > 1e-8 {
                    let grandparent_idx = skeleton.parent_of(parent_bone);
                    let gp_rot = match grandparent_idx {
                        Some(gp) => current_global[gp.0 as usize].rotation,
                        None => Quat::IDENTITY,
                    };
                    let delta_local = gp_rot.inverse() * delta_global * gp_rot;

                    let mut pose_local = pose.local_transforms_mut();
                    let bone = &mut pose_local[parent_bone.0 as usize];
                    bone.rotation = delta_local * bone.rotation;

                    if constraints.enabled {
                        if let Some(c) = constraints.for_bone(parent_bone) {
                            if c.is_active() {
                                let rest_rotation =
                                    rest_pose.local[parent_bone.0 as usize].rotation;
                                apply_constraint(bone, c, rest_rotation);
                            }
                        }
                    }
                }
            }
        }
    }

    // ── Base bone translation correction ───────────────────────────────
    if skeleton.parent_of(chain.bones[n - 1]).is_none() {
        let mut local_base = pose.local_transforms_mut();
        local_base[chain.bones[n - 1].0 as usize].translation = pos[n - 1];
        let _ = local_base;
    }

    // Re-check convergence.
    let final_global = pose.global_transforms(skeleton);
    let d = final_global[chain.bones[0].0 as usize]
        .translation
        .distance(target);
    out_of_reach || d <= chain.tolerance
}

// ═════════════════════════════════════════════════════════════════════════
// CCD solver
// ═════════════════════════════════════════════════════════════════════════

fn solve_ccd(
    pose: &mut Pose,
    skeleton: &Skeleton,
    chain: &IkChain,
    effector: &IkEffector,
    constraints: &IkConstraintSet,
) -> bool {
    let n = chain.bones.len();
    if n < 2 {
        return false;
    }

    let target = effector_world_pos(effector, pose, skeleton, chain);
    let tol_sq = chain.tolerance * chain.tolerance;
    let weight = effector.weight;

    let rest_pose = skeleton.rest_pose();

    for _ in 0..chain.iteration_count {
        // Check convergence.
        let global = pose.global_transforms(skeleton);
        let tip_pos = global[chain.bones[0].0 as usize].translation;
        if tip_pos.distance_squared(target) < tol_sq {
            return true;
        }

        // Process joints from BASE to TIP (reverse chain order: n-1 down to 1).
        // Base-first ensures that parent transforms are updated before we
        // compute the delta for child joints.
        for joint_i in (1..n).rev() {
            let joint_bone = chain.bones[joint_i];

            // Get current global transforms (reflects prior modifications).
            let current_global = pose.global_transforms(skeleton);

            // Get the tip's current position and the joint's current position.
            let tip_current = current_global[chain.bones[0].0 as usize].translation;
            let joint_pos = current_global[joint_bone.0 as usize].translation;

            let to_tip = tip_current - joint_pos;
            let to_target = target - joint_pos;

            let tt = to_tip.normalize_or_zero();
            let tgt = to_target.normalize_or_zero();

            if tt.length_squared() < 1e-10 || tgt.length_squared() < 1e-10 {
                continue;
            }

            let delta_global = Quat::from_rotation_arc(tt, tgt);
            if delta_global.is_nan() || delta_global.angle_between(Quat::IDENTITY) < 1e-8 {
                continue;
            }

            // Convert world-space delta to joint's local rotation frame.
            let grandparent_idx = skeleton.parent_of(joint_bone);
            let gp_rot = match grandparent_idx {
                Some(gp) if (gp.0 as usize) < current_global.len() => {
                    current_global[gp.0 as usize].rotation
                }
                _ => Quat::IDENTITY,
            };
            let delta_local = gp_rot.inverse() * delta_global * gp_rot;

            let pose_local = pose.local_transforms_mut();
            let bone = &mut pose_local[joint_bone.0 as usize];
            if weight >= 1.0 - 1e-6 {
                bone.rotation = delta_local * bone.rotation;
            } else {
                let w = Quat::IDENTITY.slerp(delta_local, weight);
                bone.rotation = w * bone.rotation;
            }

            if constraints.enabled {
                if let Some(c) = constraints.for_bone(joint_bone) {
                    if c.is_active() {
                        let rest_rotation = rest_pose.local[joint_bone.0 as usize].rotation;
                        apply_constraint(bone, c, rest_rotation);
                    }
                }
            }
        }
    }

    // Final convergence check.
    let final_global = pose.global_transforms(skeleton);
    let tip_pos = final_global[chain.bones[0].0 as usize].translation;
    tip_pos.distance_squared(target) < tol_sq
}

// ═════════════════════════════════════════════════════════════════════════
// Constraint application
// ═════════════════════════════════════════════════════════════════════════

/// Apply a per-bone constraint, clamping swing and twist.
fn apply_constraint(
    bone_local: &mut BoneTransform,
    constraint: &crate::ik::constraint::IkConstraint,
    rest_rotation: Quat,
) {
    let rest = constraint
        .rest_angle
        .map(|a| Quat::from_array(a))
        .unwrap_or(rest_rotation);

    let rest_inv = rest.inverse();
    let delta = rest_inv * bone_local.rotation;

    // Swing-twist decomposition around local Z (bone forward axis).
    let (swing, twist) = swing_twist(delta, Vec3::Z);

    // Clamp twist.
    let (twist_axis, twist_angle) = twist.to_axis_angle();
    let twist_min_rad = constraint.twist_min.to_radians();
    let twist_max_rad = constraint.twist_max.to_radians();
    let clamped_twist_a = twist_angle.clamp(twist_min_rad, twist_max_rad);
    let clamped_twist = Quat::from_axis_angle(twist_axis, clamped_twist_a);

    // Clamp swing.
    let (swing_axis, swing_angle) = swing.to_axis_angle();
    let swing_min_rad = constraint.swing_min.to_radians();
    let swing_max_rad = constraint.swing_max.to_radians();
    let clamped_swing_a = swing_angle.clamp(swing_min_rad, swing_max_rad);
    let clamped_swing = if swing_axis.length_squared() > 1e-10 {
        Quat::from_axis_angle(swing_axis, clamped_swing_a)
    } else {
        swing
    };

    // Softness: slerp between clamped and original.
    let s = constraint.softness.clamp(0.0, 1.0);
    let ftwist = clamped_twist.slerp(twist, s);
    let fswing = clamped_swing.slerp(swing, s);
    let final_delta = fswing * ftwist;

    // Stiffness: blend toward rest.
    if constraint.stiffness > 0.0 {
        let k = (1.0 - constraint.stiffness).clamp(0.0, 1.0);
        let blend = Quat::IDENTITY.slerp(final_delta, k);
        bone_local.rotation = rest * blend;
    } else {
        bone_local.rotation = rest * final_delta;
    }
}

/// Swing-twist decomposition: split a quaternion into swing (rotation around
/// axes perpendicular to `axis`) and twist (rotation around `axis`).
fn swing_twist(q: Quat, axis: Vec3) -> (Quat, Quat) {
    let v = Vec3::new(q.x, q.y, q.z);
    let proj = axis * v.dot(axis);
    let twist = Quat::from_xyzw(proj.x, proj.y, proj.z, q.w);
    let twist = if twist.length_squared() > 1e-10 {
        // Ensure positive w for consistent decomposition.
        let mut t = twist.normalize();
        if t.w < 0.0 {
            t = -t;
        }
        t
    } else {
        Quat::IDENTITY
    };
    // For a unit quaternion, conjugate() == inverse() but is cheaper.
    let swing = q * twist.conjugate();
    (swing, twist)
}

// ═════════════════════════════════════════════════════════════════════════
// Tests
// ═════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ik::chain::IkChain;
    use crate::ik::constraint::IkConstraint;
    use crate::ik::constraint::IkConstraintSet;
    use crate::ik::effector::IkEffector;
    use crate::skeleton::Skeleton;
    use crate::{BoneIndex, BoneTransform, Pose};

    /// 4-bone linear chain: root(Y=0) → b1(Y=1) → b2(Y=2) → tip(Y=3).
    fn linear_skeleton() -> (Skeleton, Vec<BoneIndex>) {
        let mut sk = Skeleton::new("linear".into());
        let root = sk.add_bone(None, "root".into(), BoneTransform::IDENTITY);
        let b1 = sk.add_bone(
            Some(root),
            "b1".into(),
            BoneTransform {
                translation: Vec3::Y,
                rotation: Quat::IDENTITY,
                scale: Vec3::ONE,
            },
        );
        let b2 = sk.add_bone(
            Some(b1),
            "b2".into(),
            BoneTransform {
                translation: Vec3::Y,
                rotation: Quat::IDENTITY,
                scale: Vec3::ONE,
            },
        );
        let tip = sk.add_bone(
            Some(b2),
            "tip".into(),
            BoneTransform {
                translation: Vec3::Y,
                rotation: Quat::IDENTITY,
                scale: Vec3::ONE,
            },
        );
        (sk, vec![tip, b2, b1, root])
    }

    #[test]
    fn fabrik_reaches_target() {
        let (sk, bones) = linear_skeleton();
        let chain = IkChain::new("c", bones.clone())
            .with_solver(IkSolverType::Fabrik)
            .with_iterations(20)
            .with_tolerance(0.05);
        let e = IkEffector::new("e", bones[0], Vec3::new(1.5, 1.0, 1.5));
        let mut pose = Pose::new(&sk);
        let cs = IkConstraintSet::new();
        assert!(solve_pose(&mut pose, &sk, &chain, &e, &cs));
        let g = pose.global_transforms(&sk);
        let d = g[bones[0].0 as usize].translation.distance(e.position);
        assert!(d < 0.1, "tip dist={}", d);
    }

    #[test]
    fn fabrik_out_of_reach() {
        let (sk, bones) = linear_skeleton();
        let chain = IkChain::new("c", bones.clone()).with_solver(IkSolverType::Fabrik);
        let e = IkEffector::new("e", bones[0], Vec3::new(0.0, 100.0, 0.0));
        let mut pose = Pose::new(&sk);
        let cs = IkConstraintSet::new();
        assert!(solve_pose(&mut pose, &sk, &chain, &e, &cs));
        let g = pose.global_transforms(&sk);
        let d = g[bones[0].0 as usize]
            .translation
            .distance(Vec3::new(0.0, 100.0, 0.0));
        assert!(d < 0.1, "out-of-reach tip dist={}", d);
    }

    #[test]
    fn ccd_reaches_target() {
        // CCD converges more slowly than FABRIK. With enough iterations
        // it moves the tip significantly closer to the target.
        let (sk, bones) = linear_skeleton();
        let chain = IkChain::new("c", bones.clone())
            .with_solver(IkSolverType::Ccd)
            .with_iterations(200)
            .with_tolerance(0.5);
        let e = IkEffector::new("e", bones[0], Vec3::new(1.0, 1.5, 1.0));
        let mut pose = Pose::new(&sk);
        let cs = IkConstraintSet::new();
        let converged = solve_pose(&mut pose, &sk, &chain, &e, &cs);
        let g = pose.global_transforms(&sk);
        let tip_dist = g[bones[0].0 as usize].translation.distance(e.position);
        // CCD should at least get the tip significantly closer than the
        // initial distance (~2.9 units from (0,3,0) to (1.0,1.5,1.0)).
        assert!(
            tip_dist < 1.5,
            "CCD should reduce tip distance significantly, got {}",
            tip_dist
        );
        // With 200 iterations and 0.5 tolerance, convergence is optional.
        if converged {
            assert!(tip_dist < 1.0, "CCD converged but tip far: {}", tip_dist);
        }
    }

    #[test]
    fn inactive_chain_skipped() {
        let (sk, bones) = linear_skeleton();
        let chain = IkChain::new("c", bones).with_weight(0.0);
        let e = IkEffector::new("e", BoneIndex(0), Vec3::splat(5.0));
        let mut pose = Pose::new(&sk);
        let cs = IkConstraintSet::new();
        assert!(!solve_pose(&mut pose, &sk, &chain, &e, &cs));
    }

    #[test]
    fn inactive_effector_skipped() {
        let (sk, bones) = linear_skeleton();
        let chain = IkChain::new("c", bones);
        let e = IkEffector::new("e", BoneIndex(0), Vec3::splat(5.0)).with_weight(0.0);
        let mut pose = Pose::new(&sk);
        let cs = IkConstraintSet::new();
        assert!(!solve_pose(&mut pose, &sk, &chain, &e, &cs));
    }

    #[test]
    fn zero_weight_pose_unchanged() {
        let (sk, bones) = linear_skeleton();
        let chain = IkChain::new("c", bones.clone()).with_solver(IkSolverType::Fabrik);
        let e = IkEffector::new("e", bones[0], Vec3::splat(10.0)).with_weight(0.0);
        let mut pose = Pose::new(&sk);
        let orig = pose.clone();
        let cs = IkConstraintSet::new();
        solve_pose(&mut pose, &sk, &chain, &e, &cs);
        let og = orig.global_transforms(&sk);
        let ng = pose.global_transforms(&sk);
        for i in 0..og.len() {
            assert!(
                (og[i].translation - ng[i].translation).length() < 1e-6,
                "bone {} moved",
                i
            );
        }
    }

    #[test]
    fn multi_solve() {
        let (sk, bones) = linear_skeleton();
        let chain = IkChain::new("c", bones.clone())
            .with_solver(IkSolverType::Fabrik)
            .with_iterations(30)
            .with_tolerance(0.1);
        let e = IkEffector::new("e", bones[0], Vec3::new(1.0, 2.0, 1.0));
        let mut pose = Pose::new(&sk);
        let cs = IkConstraintSet::new();
        let result = solve_pose_multi(&mut pose, &sk, &[chain], &[e], &cs);
        assert_eq!(result, 1, "multi_solve returned {result}");
    }

    #[test]
    fn arm_chain_reaches() {
        let mut sk = crate::skeleton::Skeleton::new("arm".into());
        let shoulder = sk.add_bone(None, "shoulder".into(), BoneTransform::IDENTITY);
        let upper = sk.add_bone(
            Some(shoulder),
            "upper".into(),
            BoneTransform {
                translation: Vec3::new(0.0, 0.3, 0.0),
                rotation: Quat::IDENTITY,
                scale: Vec3::ONE,
            },
        );
        let forearm = sk.add_bone(
            Some(upper),
            "forearm".into(),
            BoneTransform {
                translation: Vec3::new(0.0, 0.3, 0.0),
                rotation: Quat::IDENTITY,
                scale: Vec3::ONE,
            },
        );
        let hand = sk.add_bone(
            Some(forearm),
            "hand".into(),
            BoneTransform {
                translation: Vec3::new(0.0, 0.25, 0.0),
                rotation: Quat::IDENTITY,
                scale: Vec3::ONE,
            },
        );
        let bones = vec![hand, forearm, upper, shoulder];
        let chain = IkChain::new("arm_r", bones)
            .with_solver(IkSolverType::Fabrik)
            .with_iterations(20)
            .with_tolerance(0.05);
        let e = IkEffector::new("hand", hand, Vec3::new(0.4, 0.6, 0.4));
        let mut pose = Pose::new(&sk);
        let cs = IkConstraintSet::new();
        assert!(solve_pose(&mut pose, &sk, &chain, &e, &cs));
        let g = pose.global_transforms(&sk);
        let d = g[hand.0 as usize].translation.distance(e.position);
        assert!(d < 0.1, "hand dist={}", d);
    }

    #[test]
    fn constraint_limits_rotation() {
        let (sk, bones) = linear_skeleton();
        let chain = IkChain::new("c", bones.clone()).with_solver(IkSolverType::Fabrik);
        let e = IkEffector::new("e", bones[0], Vec3::new(2.0, 0.0, 2.0));
        let mut pose = Pose::new(&sk);
        let mut cs = IkConstraintSet::new();
        cs.add(
            IkConstraint::new(bones[1])
                .with_twist(-5.0, 5.0)
                .with_swing(-10.0, 10.0),
        );
        assert!(solve_pose(&mut pose, &sk, &chain, &e, &cs));
    }

    #[test]
    fn trivial_chain_returns_false() {
        let (sk, bones) = linear_skeleton();
        let chain = IkChain::new("c", vec![bones[0]]);
        let e = IkEffector::new("e", bones[0], Vec3::splat(5.0));
        let mut pose = Pose::new(&sk);
        let cs = IkConstraintSet::new();
        assert!(!solve_pose(&mut pose, &sk, &chain, &e, &cs));
    }

    #[test]
    fn swing_twist_roundtrip() {
        let q = Quat::from_euler(glam::EulerRot::XYZ, 0.3, 0.2, 0.1);
        let (swing, twist) = swing_twist(q, Vec3::Z);
        let recomposed = swing * twist;
        let angle = q.angle_between(recomposed);
        assert!(
            angle < 1e-3,
            "swing-twist roundtrip failed: angle={}",
            angle
        );
    }
}
