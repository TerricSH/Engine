use glam::Quat;

use crate::assets::{AnimationClip, JointTransform, Keyframe, Skeleton};
use crate::blend_space::BlendSpace1D;
use crate::components::AnimationPlayer;
use crate::components::IkTargetComponent;
use crate::ik::solve_pose_multi;
use crate::pose::Pose;
use crate::skeleton;
use crate::state_machine::{AnimParamValue, AnimStateMachineInstance};

// ---------------------------------------------------------------------------
// AnimationEvaluator
// ---------------------------------------------------------------------------

/// Evaluates animation clips against a skeleton, producing bone palette data.
pub struct AnimationEvaluator;

impl AnimationEvaluator {
    /// Evaluate a single clip at the given time, producing local joint transforms.
    ///
    /// Returns a vector of [`JointTransform`] in skeleton joint order.
    /// Non-animated joints receive [`JointTransform::IDENTITY`].
    pub fn evaluate(clip: &AnimationClip, time: f32, skeleton: &Skeleton) -> Vec<JointTransform> {
        let count = skeleton.joint_count();
        let mut result = vec![JointTransform::IDENTITY; count];

        for channel in &clip.channels {
            let joint_idx = channel.joint_index as usize;
            if joint_idx >= count {
                continue;
            }

            let t = Self::sample_channel(&channel.translations, time, lerp_f32x3);
            let r = Self::sample_channel(&channel.rotations, time, slerp_f32x4);
            let s = Self::sample_channel(&channel.scales, time, lerp_f32x3);

            result[joint_idx] = JointTransform {
                translation: t.unwrap_or([0.0, 0.0, 0.0]),
                rotation: r.unwrap_or([0.0, 0.0, 0.0, 1.0]),
                scale: s.unwrap_or([1.0, 1.0, 1.0]),
            };
        }

        result
    }

    /// Evaluate a single clip at the given time, producing a runtime [`Pose`].
    ///
    /// Starts from the skeleton's rest pose and overrides each animated channel.
    /// Non-animated bones retain their rest-pose transform.
    pub fn evaluate_pose(clip: &AnimationClip, time: f32, skeleton: &skeleton::Skeleton) -> Pose {
        let mut pose = skeleton.rest_pose();
        for channel in &clip.channels {
            let joint_idx = channel.joint_index as usize;
            if joint_idx >= pose.local.len() {
                continue;
            }

            let t = Self::sample_channel(&channel.translations, time, lerp_f32x3);
            let r = Self::sample_channel(&channel.rotations, time, slerp_f32x4);
            let s = Self::sample_channel(&channel.scales, time, lerp_f32x3);

            let jt = JointTransform {
                translation: t.unwrap_or([0.0, 0.0, 0.0]),
                rotation: r.unwrap_or([0.0, 0.0, 0.0, 1.0]),
                scale: s.unwrap_or([1.0, 1.0, 1.0]),
            };
            pose.local[joint_idx] = jt.into();
        }
        pose
    }

    /// Sample a keyframe track at a given time, returning interpolated value.
    fn sample_channel<T: Copy>(
        keyframes: &[Keyframe<T>],
        time: f32,
        lerp: fn(T, T, f32) -> T,
    ) -> Option<T> {
        match keyframes.len() {
            0 => return None,
            1 => return Some(keyframes[0].value),
            _ => {}
        }

        // Clamp / hold.
        if time <= keyframes[0].time {
            return Some(keyframes[0].value);
        }
        let last_idx = keyframes.len() - 1;
        if time >= keyframes[last_idx].time {
            return Some(keyframes[last_idx].value);
        }

        // Binary search for surrounding pair.
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

        Some(lerp(prev.value, next.value, t))
    }

    /// Linear interpolation between two [f32; 3] translation values.
    pub fn lerp_translation(a: &[f32; 3], b: &[f32; 3], t: f32) -> [f32; 3] {
        lerp_f32x3(*a, *b, t)
    }

    /// Spherical linear interpolation between two quaternion [f32; 4] values.
    pub fn lerp_rotation(a: &[f32; 4], b: &[f32; 4], t: f32) -> [f32; 4] {
        slerp_f32x4(*a, *b, t)
    }

    /// Linear interpolation between two [f32; 3] scale values.
    pub fn lerp_scale(a: &[f32; 3], b: &[f32; 3], t: f32) -> [f32; 3] {
        lerp_f32x3(*a, *b, t)
    }
}

// ---------------------------------------------------------------------------
// Private interpolation helpers (replacing the old Lerp trait)
// ---------------------------------------------------------------------------

/// Linear interpolation for [f32; 3] (translations and scales).
fn lerp_f32x3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

/// Spherical linear interpolation for quaternion [f32; 4] (rotations).
fn slerp_f32x4(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    let qa = Quat::from_array(a).normalize();
    let qb = Quat::from_array(b).normalize();
    qa.slerp(qb, t).to_array()
}

// ---------------------------------------------------------------------------
// update_animation_sm — state-machine-driven evaluation
// ---------------------------------------------------------------------------

/// Advance an [`AnimStateMachineInstance`] by `dt` seconds and produce a bone
/// palette (global joint matrices) for GPU skinning.
///
/// `clips` is a slice of `(asset_id, AnimationClip)` pairs used to resolve the
/// clip references inside each state of the state machine.
///
/// Returns the bone palette — one 4×4 matrix per skeleton joint.
/// The palette is empty if the player is not playing or the clip cannot be
/// resolved.
pub fn update_animation_sm(
    player: &AnimationPlayer,
    sm: &mut AnimStateMachineInstance,
    clips: &[(&str, AnimationClip)],
    skel: &skeleton::Skeleton,
    dt: f32,
) -> Vec<[[f32; 4]; 4]> {
    match evaluate_sm_to_pose(player, sm, clips, skel, dt) {
        Some(pose) => {
            let matrices = pose.skin_matrices(skel);
            matrices.iter().map(|m| m.to_cols_array_2d()).collect()
        }
        None => Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// evaluate_sm_to_pose — internal helper for state machine → Pose
// ---------------------------------------------------------------------------

/// Evaluate the state machine to a [`Pose`] (internal helper).
/// Returns `Some(pose)` if evaluating, `None` if player is stopped or no clips.
fn evaluate_sm_to_pose(
    player: &AnimationPlayer,
    sm: &mut AnimStateMachineInstance,
    clips: &[(&str, AnimationClip)],
    skel: &skeleton::Skeleton,
    dt: f32,
) -> Option<Pose> {
    if !player.playing || sm.state_machine.states.is_empty() {
        return None;
    }

    // Advance the state machine and get the active state + blend weight.
    let (_state_name, blend_weight) = sm.update(dt);

    // Resolve the current state.
    let state = sm.state_machine.find_state(&sm.current_state)?;

    // Evaluate the current pose: either via blend space or single clip.
    let current_pose = if let Some(ref bs) = state.blend_space_1d {
        evaluate_blend_space_1d(bs, sm, clips, skel, sm.current_time)
    } else {
        let clip = match clips.iter().find(|(id, _)| *id == state.clip_asset) {
            Some((_, c)) => c,
            None => return None,
        };
        let clip_time = if state.looping && clip.duration() > 0.0 {
            sm.current_time % clip.duration()
        } else {
            sm.current_time.min(clip.duration())
        };
        AnimationEvaluator::evaluate_pose(clip, clip_time, skel)
    };

    let final_pose = if sm.transitioning && blend_weight < 1.0 {
        // Resolve the from-state for crossfade blending.
        let from_state = match sm.state_machine.find_state(&sm.transition_from) {
            Some(s) => s,
            None => return Some(current_pose),
        };
        let from_pose = if let Some(ref bs) = from_state.blend_space_1d {
            evaluate_blend_space_1d(bs, sm, clips, skel, sm.current_time)
        } else {
            match clips.iter().find(|(id, _)| *id == from_state.clip_asset) {
                Some((_, c)) => AnimationEvaluator::evaluate_pose(c, sm.current_time, skel),
                None => return Some(current_pose),
            }
        };

        // Crossfade using Pose::blend.
        Pose::blend(&from_pose, &current_pose, blend_weight)
    } else {
        current_pose
    };

    Some(final_pose)
}

// ---------------------------------------------------------------------------
// evaluate_blend_space_1d — 1D blend space evaluation
// ---------------------------------------------------------------------------

/// Evaluate a [`BlendSpace1D`] by sampling between the two surrounding clips
/// based on the current parameter value.  If the parameter falls outside the
/// sample range the closest clip is used directly.
fn evaluate_blend_space_1d(
    bs: &BlendSpace1D,
    sm: &AnimStateMachineInstance,
    clips: &[(&str, AnimationClip)],
    skel: &skeleton::Skeleton,
    time: f32,
) -> Pose {
    // Get the parameter value that drives the blend.
    let param = match sm.get_param(&bs.parameter_name) {
        Some(AnimParamValue::Float(v)) => *v,
        _ => {
            return if bs.clips.is_empty() {
                skel.rest_pose()
            } else if let Some((_, clip)) =
                clips.iter().find(|(id, _)| *id == bs.clips[0].clip_asset)
            {
                AnimationEvaluator::evaluate_pose(clip, time, skel)
            } else {
                skel.rest_pose()
            };
        }
    };

    // No samples → rest pose.
    if bs.clips.is_empty() {
        return skel.rest_pose();
    }

    // Single sample → evaluate it directly.
    if bs.clips.len() == 1 {
        return if let Some((_, clip)) = clips.iter().find(|(id, _)| *id == bs.clips[0].clip_asset) {
            AnimationEvaluator::evaluate_pose(clip, time, skel)
        } else {
            skel.rest_pose()
        };
    }

    let first = &bs.clips[0];
    let last = bs.clips.last().unwrap();

    if param <= first.threshold {
        // Below range → sample first clip.
        return if let Some((_, clip)) = clips.iter().find(|(id, _)| *id == first.clip_asset) {
            AnimationEvaluator::evaluate_pose(clip, time, skel)
        } else {
            skel.rest_pose()
        };
    }

    if param >= last.threshold {
        // Above range → sample last clip.
        return if let Some((_, clip)) = clips.iter().find(|(id, _)| *id == last.clip_asset) {
            AnimationEvaluator::evaluate_pose(clip, time, skel)
        } else {
            skel.rest_pose()
        };
    }

    // Find the surrounding pair via linear scan.
    let n = bs.clips.len();
    let mut lower_idx = 0;
    for i in 0..n - 1 {
        if param >= bs.clips[i].threshold && param < bs.clips[i + 1].threshold {
            lower_idx = i;
            break;
        }
    }
    let upper_idx = lower_idx + 1;

    let lower = &bs.clips[lower_idx];
    let upper = &bs.clips[upper_idx];
    let range = upper.threshold - lower.threshold;
    let t = if range > 0.0 {
        ((param - lower.threshold) / range).clamp(0.0, 1.0)
    } else {
        0.0
    };

    let lower_pose = clips
        .iter()
        .find(|(id, _)| *id == lower.clip_asset)
        .map(|(_, clip)| AnimationEvaluator::evaluate_pose(clip, time, skel))
        .unwrap_or_else(|| skel.rest_pose());

    let upper_pose = clips
        .iter()
        .find(|(id, _)| *id == upper.clip_asset)
        .map(|(_, clip)| AnimationEvaluator::evaluate_pose(clip, time, skel))
        .unwrap_or_else(|| skel.rest_pose());

    Pose::blend(&lower_pose, &upper_pose, t)
}

// ---------------------------------------------------------------------------
// evaluate_clip_to_pose — internal helper for direct clip → Pose
// ---------------------------------------------------------------------------

/// Evaluate a single animation clip to a [`Pose`] (internal helper).
///
/// Advances time using `player.current_time + dt * player.speed` locally and
/// applies looping/clamping logic, then evaluates the clip at the resulting time.
fn evaluate_clip_to_pose(
    player: &AnimationPlayer,
    clip: &AnimationClip,
    skel: &skeleton::Skeleton,
    dt: f32,
) -> Pose {
    // Advance time locally (same logic as update_animation).
    let mut effective_time = player.current_time + dt * player.speed;

    // Handle looping / clamping.
    if clip.duration > 0.0 {
        if player.looping {
            effective_time = effective_time.rem_euclid(clip.duration);
        } else {
            effective_time = effective_time.clamp(0.0, clip.duration);
        }
    }

    AnimationEvaluator::evaluate_pose(clip, effective_time, skel)
}

// ---------------------------------------------------------------------------
// update_animation_pipeline — unified orchestration
// ---------------------------------------------------------------------------

/// Orchestrate the full animation pipeline: evaluate → blend layers → IK → skin matrices.
///
/// This is the "one-stop shop" that unifies clip evaluation, state machine crossfade,
/// animation layers, IK post-processing, and skin matrix computation into a single call.
///
/// * `player` — The [`AnimationPlayer`] component driving playback parameters.
/// * `sm` — Optional state machine instance; if `Some` and has active states the state
///   machine path is used instead of direct clip evaluation.
/// * `clips` — Slice of `(asset_id, AnimationClip)` pairs for resolving clip references.
/// * `skel` — The skeleton to evaluate against.
/// * `ik` — Optional IK target component for post-processing.
/// * `dt` — Delta time in seconds.
///
/// Returns the bone palette — one 4×4 matrix per skeleton joint, ready for GPU skinning.
pub fn update_animation_pipeline(
    player: &mut AnimationPlayer,
    sm: &mut Option<AnimStateMachineInstance>,
    clips: &[(&str, AnimationClip)],
    skel: &skeleton::Skeleton,
    ik: Option<&IkTargetComponent>,
    dt: f32,
) -> Vec<[[f32; 4]; 4]> {
    // ── 1. Evaluate base pose ────────────────────────────────────────────
    let pose = if let Some(ref mut sm_inner) = sm {
        if !sm_inner.state_machine.states.is_empty() {
            match evaluate_sm_to_pose(player, sm_inner, clips, skel, dt) {
                Some(p) => p,
                None => skel.rest_pose(),
            }
        } else if let Some(ref clip_asset) = player.clip_asset {
            match clips.iter().find(|(id, _)| *id == clip_asset.as_str()) {
                Some((_, clip)) => evaluate_clip_to_pose(player, clip, skel, dt),
                None => skel.rest_pose(),
            }
        } else {
            skel.rest_pose()
        }
    } else if let Some(ref clip_asset) = player.clip_asset {
        match clips.iter().find(|(id, _)| *id == clip_asset.as_str()) {
            Some((_, clip)) => evaluate_clip_to_pose(player, clip, skel, dt),
            None => skel.rest_pose(),
        }
    } else {
        skel.rest_pose()
    };

    // ── 2. Apply animation layers (simple blend for v1) ──────────────────
    // The base layer is already evaluated above.  Additional layers are
    // blended on top.  For now layers don't carry clip references, so this
    // is a structural placeholder for future multi-layer support.
    let pose = if player.layers.len() <= 1 {
        pose
    } else {
        // Accumulate layers on top of the base pose.
        // For v1: skip the "base" layer (already evaluated) and blend any
        // additional layers.  Since AnimLayer has no clip_asset, this is
        // a future extension point.
        pose
    };

    // ── 3. Apply IK post-processing ──────────────────────────────────────
    let pre_ik = pose.clone();
    let pose = if let Some(ik_comp) = ik {
        if ik_comp.enabled && ik_comp.blend_weight > 0.0 {
            let mut ik_pose = pose;
            solve_pose_multi(
                &mut ik_pose,
                skel,
                &ik_comp.chains,
                &ik_comp.effectors,
                &ik_comp.constraints,
            );
            if ik_comp.blend_weight < 1.0 {
                Pose::blend(&pre_ik, &ik_pose, ik_comp.blend_weight)
            } else {
                ik_pose
            }
        } else {
            pose
        }
    } else {
        pose
    };

    // ── 4. Cache bone world positions for C# query ───────────────────────
    let global = pose.global_transforms(skel);
    player.cached_bone_positions = global.iter().map(|bt| bt.translation.to_array()).collect();

    // ── 5. Compute skin matrices ─────────────────────────────────────────
    pose.skin_matrices(skel)
        .iter()
        .map(|m| m.to_cols_array_2d())
        .collect()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// update_animation
// ---------------------------------------------------------------------------

/// Advance an [`AnimationPlayer`] component by `dt` seconds and produce a
/// bone palette (global joint matrices) for GPU skinning.
///
/// Returns the bone palette — one 4×4 matrix per skeleton joint.
/// The palette is empty if no clip or skeleton is provided.
pub fn update_animation(
    player: &mut AnimationPlayer,
    clip: Option<&AnimationClip>,
    skel: Option<&skeleton::Skeleton>,
    dt: f32,
) -> Vec<[[f32; 4]; 4]> {
    if !player.playing {
        // Still evaluate at current_time if there's a clip and skeleton.
        if let (Some(clip), Some(skel)) = (clip, skel) {
            let pose = AnimationEvaluator::evaluate_pose(clip, player.current_time, skel);
            return pose
                .skin_matrices(skel)
                .iter()
                .map(|m| m.to_cols_array_2d())
                .collect();
        }
        return Vec::new();
    }

    // Advance time.
    player.current_time += dt * player.speed;

    // Handle looping / clamping.
    if let Some(clip) = clip {
        if clip.duration > 0.0 {
            if player.looping {
                player.current_time = player.current_time.rem_euclid(clip.duration);
            } else {
                player.current_time = player.current_time.clamp(0.0, clip.duration);
                if player.current_time >= clip.duration {
                    player.playing = false;
                }
            }
        }
    }

    // Evaluate and solve.
    match (clip, skel) {
        (Some(clip), Some(skel)) => {
            let pose = AnimationEvaluator::evaluate_pose(clip, player.current_time, skel);
            pose.skin_matrices(skel)
                .iter()
                .map(|m| m.to_cols_array_2d())
                .collect()
        }
        _ => Vec::new(),
    }
}
