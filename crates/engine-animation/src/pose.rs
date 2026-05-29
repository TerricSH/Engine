use glam::Mat4;

use crate::skeleton::Skeleton;
use crate::{BoneIndex, BoneTransform};

// ---------------------------------------------------------------------------
// Pose — per-bone local-space transforms, with helpers to compute global
//        transforms, skin matrices, and blends.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Pose {
    pub(crate) local: Vec<BoneTransform>,
}

impl Pose {
    /// Create a `Pose` pre-populated with the skeleton's rest pose.
    pub fn new(skeleton: &Skeleton) -> Self {
        skeleton.rest_pose()
    }

    /// Read-only access to per-bone local-space transforms.
    pub fn local_transforms(&self) -> &[BoneTransform] {
        &self.local
    }

    /// Mutable access to per-bone local-space transforms.
    pub fn local_transforms_mut(&mut self) -> &mut [BoneTransform] {
        &mut self.local
    }

    /// Compute world-space (global) transforms by walking the skeleton
    /// hierarchy top-down.  Root bones use their local transform directly;
    /// children are composed as `parent_global * child_local`.
    pub fn global_transforms(&self, skeleton: &Skeleton) -> Vec<BoneTransform> {
        let count = self.local.len();
        let mut global = Vec::with_capacity(count);

        for i in 0..count {
            let local = self.local[i];
            match skeleton.parent_of(BoneIndex(i as u16)) {
                Some(parent_idx) => {
                    // SAFETY: parent_idx < i because we add bones in order, so
                    // the parent's global entry is already computed.
                    let parent_global = global[parent_idx.0 as usize];
                    global.push(parent_global * local);
                }
                None => {
                    global.push(local);
                }
            }
        }

        global
    }

    /// Compute bone palette matrices suitable for GPU skinning.
    ///
    /// Each entry is `current_global[i] * inverse(rest_global[i])` — i.e. the
    /// transform that maps a vertex from the bind (rest) pose into the current
    /// animated pose.  The renderer uploads these as `bone_palette: Vec<Mat4>`.
    pub fn skin_matrices(&self, skeleton: &Skeleton) -> Vec<Mat4> {
        let rest_global = skeleton.rest_pose().global_transforms(skeleton);
        let current_global = self.global_transforms(skeleton);

        let count = self.local.len();
        let mut matrices = Vec::with_capacity(count);

        for i in 0..count {
            let current = current_global[i].to_mat4();
            let inverse_rest = rest_global[i].to_mat4().inverse();
            matrices.push(current * inverse_rest);
        }

        matrices
    }

    /// Linearly blend two poses.  Uses LERP for translation and scale, SLERP
    /// for rotation.  `t` is clamped to `[0, 1]`.
    ///
    /// If the pose bone counts differ the function blends up to
    /// `min(a.len, b.len)` bones and logs a warning.
    pub fn blend(a: &Pose, b: &Pose, t: f32) -> Pose {
        let t = t.clamp(0.0, 1.0);
        let count = a.local.len().min(b.local.len());

        if a.local.len() != b.local.len() {
            tracing::warn!(
                a_bones = a.local.len(),
                b_bones = b.local.len(),
                blended = count,
                "Pose::blend bone count mismatch"
            );
        }

        let mut local = Vec::with_capacity(count);
        for i in 0..count {
            let at = a.local[i];
            let bt = b.local[i];
            local.push(BoneTransform {
                translation: at.translation.lerp(bt.translation, t),
                rotation: at.rotation.slerp(bt.rotation, t),
                scale: at.scale.lerp(bt.scale, t),
            });
        }

        Pose { local }
    }
}
