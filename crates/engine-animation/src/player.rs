use glam::{Quat, Vec3};

use crate::assets::{AnimationClip, JointTransform, Keyframe, Skeleton};
use crate::blend_space::BlendSpace1D;
use crate::components::AnimationPlayer;
use crate::components::IkTargetComponent;
use crate::ik::solve_pose_multi;
use crate::layers::{AnimLayer, LayerBlendMode};
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

            let transform = &mut pose.local[joint_idx];
            if let Some(translation) = Self::sample_channel(&channel.translations, time, lerp_f32x3)
            {
                transform.translation = Vec3::from(translation);
            }
            if let Some(rotation) = Self::sample_channel(&channel.rotations, time, slerp_f32x4) {
                transform.rotation = quat_or_identity(rotation);
            }
            if let Some(scale) = Self::sample_channel(&channel.scales, time, lerp_f32x3) {
                transform.scale = Vec3::from(scale);
            }
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
    let qa = quat_or_identity(a);
    let qb = quat_or_identity(b);
    qa.slerp(qb, t).to_array()
}

fn quat_or_identity(value: [f32; 4]) -> Quat {
    let quat = Quat::from_array(value);
    let length_squared = quat.length_squared();
    if quat.is_finite() && length_squared.is_finite() && length_squared > f32::EPSILON {
        quat / length_squared.sqrt()
    } else {
        Quat::IDENTITY
    }
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
    let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };

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
    if sm.state_machine.states.is_empty() {
        return None;
    }

    // Advance the state machine and get the active state + blend weight.
    let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
    let speed = if player.speed.is_finite() {
        player.speed
    } else {
        1.0
    };
    let blend_weight = if player.playing {
        sm.update(dt * speed).1
    } else if sm.transitioning {
        sm.transition_progress
    } else {
        1.0
    };

    // Resolve the current state.
    let state = sm.state_machine.find_state(&sm.current_state)?;

    // Evaluate the current pose: either via blend space or single clip.
    let current_pose = if let Some(ref bs) = state.blend_space_1d {
        evaluate_blend_space_1d(bs, sm, clips, skel, sm.current_time, state.looping)
    } else {
        let clip = match clips.iter().find(|(id, _)| *id == state.clip_asset) {
            Some((_, c)) => c,
            None => return None,
        };
        evaluate_clip_at(clip, sm.current_time, state.looping, skel)
    };

    let final_pose = if sm.transitioning && blend_weight < 1.0 {
        // Resolve the from-state for crossfade blending.
        let from_state = match sm.state_machine.find_state(&sm.transition_from) {
            Some(s) => s,
            None => return Some(current_pose),
        };
        let from_pose = if let Some(ref bs) = from_state.blend_space_1d {
            evaluate_blend_space_1d(
                bs,
                sm,
                clips,
                skel,
                sm.transition_from_time,
                from_state.looping,
            )
        } else {
            match clips.iter().find(|(id, _)| *id == from_state.clip_asset) {
                Some((_, clip)) => {
                    evaluate_clip_at(clip, sm.transition_from_time, from_state.looping, skel)
                }
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

fn evaluate_clip_at(
    clip: &AnimationClip,
    time: f32,
    looping: bool,
    skel: &skeleton::Skeleton,
) -> Pose {
    let duration = clip.duration();
    let time = if !time.is_finite() || !duration.is_finite() || duration <= 0.0 {
        0.0
    } else if looping {
        time.rem_euclid(duration)
    } else {
        time.clamp(0.0, duration)
    };
    AnimationEvaluator::evaluate_pose(clip, time, skel)
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
    looping: bool,
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
                evaluate_clip_at(clip, time, looping, skel)
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
            evaluate_clip_at(clip, time, looping, skel)
        } else {
            skel.rest_pose()
        };
    }

    let Some(first) = bs.clips.first() else {
        return skel.rest_pose();
    };
    let Some(last) = bs.clips.last() else {
        return skel.rest_pose();
    };

    if param <= first.threshold {
        // Below range → sample first clip.
        return if let Some((_, clip)) = clips.iter().find(|(id, _)| *id == first.clip_asset) {
            evaluate_clip_at(clip, time, looping, skel)
        } else {
            skel.rest_pose()
        };
    }

    if param >= last.threshold {
        // Above range → sample last clip.
        return if let Some((_, clip)) = clips.iter().find(|(id, _)| *id == last.clip_asset) {
            evaluate_clip_at(clip, time, looping, skel)
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
        .map(|(_, clip)| evaluate_clip_at(clip, time, looping, skel))
        .unwrap_or_else(|| skel.rest_pose());

    let upper_pose = clips
        .iter()
        .find(|(id, _)| *id == upper.clip_asset)
        .map(|(_, clip)| evaluate_clip_at(clip, time, looping, skel))
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
    player: &mut AnimationPlayer,
    clip: &AnimationClip,
    skel: &skeleton::Skeleton,
    dt: f32,
) -> Pose {
    let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
    if !player.current_time.is_finite() {
        player.current_time = 0.0;
    }
    if player.playing {
        let speed = if player.speed.is_finite() {
            player.speed
        } else {
            1.0
        };
        player.current_time += dt * speed;
        if clip.duration.is_finite() && clip.duration > 0.0 {
            if player.looping {
                player.current_time = player.current_time.rem_euclid(clip.duration);
            } else {
                player.current_time = player.current_time.clamp(0.0, clip.duration);
                if (speed >= 0.0 && player.current_time >= clip.duration)
                    || (speed < 0.0 && player.current_time <= 0.0)
                {
                    player.playing = false;
                }
            }
        } else {
            player.current_time = 0.0;
        }
    }

    AnimationEvaluator::evaluate_pose(clip, player.current_time, skel)
}

fn advance_layer_time(layer: &mut AnimLayer, clip: &AnimationClip, dt: f32, playing: bool) {
    if !layer.current_time.is_finite() {
        layer.current_time = 0.0;
    }
    if !playing {
        return;
    }

    let speed = if layer.speed.is_finite() {
        layer.speed
    } else {
        1.0
    };
    layer.current_time += dt * speed;

    if clip.duration.is_finite() && clip.duration > 0.0 {
        if layer.looping {
            layer.current_time = layer.current_time.rem_euclid(clip.duration);
        } else {
            layer.current_time = layer.current_time.clamp(0.0, clip.duration);
        }
    } else {
        layer.current_time = 0.0;
    }
}

fn blend_animation_layer(base: &mut Pose, layer_pose: &Pose, rest_pose: &Pose, layer: &AnimLayer) {
    let weight = if layer.weight.is_finite() {
        layer.weight.clamp(0.0, 1.0)
    } else {
        0.0
    };
    if weight <= 0.0 {
        return;
    }

    let count = base
        .local_transforms()
        .len()
        .min(layer_pose.local_transforms().len())
        .min(rest_pose.local_transforms().len());
    let base_local = base.local_transforms_mut();
    let layer_local = layer_pose.local_transforms();
    let rest_local = rest_pose.local_transforms();

    for bone_index in 0..count {
        let selected = layer.bone_mask.is_empty()
            || u16::try_from(bone_index)
                .ok()
                .is_some_and(|index| layer.bone_mask.contains(&index));
        if !selected {
            continue;
        }

        let current = &mut base_local[bone_index];
        let sampled = layer_local[bone_index];
        let rest = rest_local[bone_index];
        match layer.blend_mode {
            LayerBlendMode::Overwrite => {
                current.translation = current.translation.lerp(sampled.translation, weight);
                current.rotation = current.rotation.slerp(sampled.rotation, weight);
                current.scale = current.scale.lerp(sampled.scale, weight);
            }
            LayerBlendMode::Additive => {
                current.translation += (sampled.translation - rest.translation) * weight;

                let rotation_delta = rest.rotation.inverse() * sampled.rotation;
                let weighted_delta = Quat::IDENTITY.slerp(rotation_delta, weight);
                current.rotation = (current.rotation * weighted_delta).normalize();

                let scale_ratio = Vec3::new(
                    safe_scale_ratio(sampled.scale.x, rest.scale.x),
                    safe_scale_ratio(sampled.scale.y, rest.scale.y),
                    safe_scale_ratio(sampled.scale.z, rest.scale.z),
                );
                current.scale *= Vec3::ONE.lerp(scale_ratio, weight);
            }
        }
    }
}

fn safe_scale_ratio(sampled: f32, rest: f32) -> f32 {
    if sampled.is_finite() && rest.is_finite() && rest.abs() > f32::EPSILON {
        sampled / rest
    } else {
        1.0
    }
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
    let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
    let direct_clip_asset = player.clip_asset.clone();

    // ── 1. Evaluate base pose ────────────────────────────────────────────
    let mut pose = if let Some(sm_inner) = sm
        .as_mut()
        .filter(|instance| !instance.state_machine.states.is_empty())
    {
        evaluate_sm_to_pose(player, sm_inner, clips, skel, dt).unwrap_or_else(|| skel.rest_pose())
    } else if let Some(clip_asset) = direct_clip_asset.as_deref() {
        clips
            .iter()
            .find(|(id, _)| *id == clip_asset)
            .map(|(_, clip)| evaluate_clip_to_pose(player, clip, skel, dt))
            .unwrap_or_else(|| skel.rest_pose())
    } else {
        skel.rest_pose()
    };

    // ── 2. Apply animation layers (simple blend for v1) ──────────────────
    if player.layers.len() > 1 {
        let rest_pose = skel.rest_pose();
        let playing = player.playing;
        for layer in player.layers.iter_mut().skip(1) {
            let Some(clip_asset) = layer.clip_asset.clone() else {
                continue;
            };
            let Some((_, clip)) = clips.iter().find(|(id, _)| *id == clip_asset) else {
                continue;
            };

            advance_layer_time(layer, clip, dt, playing);
            let layer_pose = AnimationEvaluator::evaluate_pose(clip, layer.current_time, skel);
            blend_animation_layer(&mut pose, &layer_pose, &rest_pose, layer);
        }
    }

    // ── 3. Apply IK post-processing ──────────────────────────────────────
    let pre_ik = pose.clone();
    let mut pose = if let Some(ik_comp) = ik {
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

    // ── 4. Apply external pose ownership (ragdoll / physical animation) ──
    if let Some(override_pose) = player.external_pose_override.as_ref() {
        if override_pose.local_transforms.len() == skel.bone_count()
            && override_pose.weight.is_finite()
            && override_pose.weight > 0.0
        {
            let external = Pose::from_local_transforms(override_pose.local_transforms.clone());
            pose = Pose::blend(&pose, &external, override_pose.weight);
        }
    }

    // ── 5. Cache bone world positions for C# query ───────────────────────
    let global = pose.global_transforms(skel);
    player.set_cached_bone_positions(&global);

    // ── 6. Compute skin matrices ─────────────────────────────────────────
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
    let (Some(clip), Some(skel)) = (clip, skel) else {
        return Vec::new();
    };

    let pose = evaluate_clip_to_pose(player, clip, skel, dt);
    let global = pose.global_transforms(skel);
    player.set_cached_bone_positions(&global);
    pose.skin_matrices(skel)
        .iter()
        .map(|m| m.to_cols_array_2d())
        .collect()
}
