use glam::{Mat4, Quat, Vec3};

use crate::assets::{AnimationClip, JointTransform, Keyframe, Skeleton};
use crate::components::AnimationPlayer;

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

            let t = Self::sample_channel(&channel.translations, time);
            let r = Self::sample_channel(&channel.rotations, time);
            let s = Self::sample_channel(&channel.scales, time);

            result[joint_idx] = JointTransform {
                translation: t.unwrap_or([0.0, 0.0, 0.0]),
                rotation: r.unwrap_or([0.0, 0.0, 0.0, 1.0]),
                scale: s.unwrap_or([1.0, 1.0, 1.0]),
            };
        }

        result
    }

    /// Sample a keyframe track at a given time, returning interpolated value.
    fn sample_channel<T>(keyframes: &[Keyframe<T>], time: f32) -> Option<T>
    where
        T: Copy + Lerp,
    {
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

        Some(Lerp::lerp(prev.value, next.value, t))
    }

    /// Linear interpolation between two [f32; 3] translation values.
    pub fn lerp_translation(a: &[f32; 3], b: &[f32; 3], t: f32) -> [f32; 3] {
        Lerp::lerp(*a, *b, t)
    }

    /// Spherical linear interpolation between two quaternion [f32; 4] values.
    pub fn lerp_rotation(a: &[f32; 4], b: &[f32; 4], t: f32) -> [f32; 4] {
        Lerp::lerp(*a, *b, t)
    }

    /// Linear interpolation between two [f32; 3] scale values.
    pub fn lerp_scale(a: &[f32; 3], b: &[f32; 3], t: f32) -> [f32; 3] {
        Lerp::lerp(*a, *b, t)
    }

    /// Convert local joint transforms to global (world-space) matrices by
    /// walking the skeleton hierarchy.
    ///
    /// Returns one 4×4 matrix per joint in skeleton order.
    pub fn solve_hierarchy(
        local: &[JointTransform],
        skeleton: &Skeleton,
    ) -> Vec<[[f32; 4]; 4]> {
        let count = skeleton.joint_count();
        let joints = skeleton.joints();
        // Use glam Mat4 for correct multiplication, then convert back.
        let mut global_glam: Vec<Mat4> = Vec::with_capacity(count);

        for i in 0..count {
            let local_glam = joint_transform_to_glam(&local[i]);
            let global_i = match joints[i].parent_index {
                Some(parent_idx) => global_glam[parent_idx as usize] * local_glam,
                None => local_glam,
            };
            global_glam.push(global_i);
        }

        global_glam.iter().map(|m| m.to_cols_array_2d()).collect()
    }
}

// ---------------------------------------------------------------------------
// Lerp trait (internal)
// ---------------------------------------------------------------------------

/// Trait for linear interpolation of keyframe values.
trait Lerp: Copy {
    fn lerp(a: Self, b: Self, t: f32) -> Self;
}

impl Lerp for [f32; 3] {
    fn lerp(a: Self, b: Self, t: f32) -> Self {
        [
            a[0] + (b[0] - a[0]) * t,
            a[1] + (b[1] - a[1]) * t,
            a[2] + (b[2] - a[2]) * t,
        ]
    }
}

impl Lerp for [f32; 4] {
    fn lerp(a: Self, b: Self, t: f32) -> Self {
        // Normalise quaternions for safe slerp.
        let qa = Quat::from_array(a).normalize();
        let qb = Quat::from_array(b).normalize();
        qa.slerp(qb, t).to_array()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Convert a [`JointTransform`] to a glam [`Mat4`].
fn joint_transform_to_glam(jt: &JointTransform) -> Mat4 {
    let t = Vec3::from(jt.translation);
    let r = Quat::from_array(jt.rotation);
    let s = Vec3::from(jt.scale);
    Mat4::from_scale_rotation_translation(s, r, t)
}

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
    skeleton: Option<&Skeleton>,
    dt: f32,
) -> Vec<[[f32; 4]; 4]> {
    if !player.playing {
        // Still evaluate at current_time if there's a clip and skeleton.
        if let (Some(clip), Some(skeleton)) = (clip, skeleton) {
            let local = AnimationEvaluator::evaluate(clip, player.current_time, skeleton);
            return AnimationEvaluator::solve_hierarchy(&local, skeleton);
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
    match (clip, skeleton) {
        (Some(clip), Some(skeleton)) => {
            let local = AnimationEvaluator::evaluate(clip, player.current_time, skeleton);
            AnimationEvaluator::solve_hierarchy(&local, skeleton)
        }
        _ => Vec::new(),
    }
}
