use std::sync::Mutex;

use engine_renderer::{DebugDrawBuffer, DebugDrawProvider};
use glam::{Mat4, Vec3};

/// Info about one skeleton instance for debug rendering.
///
/// Pushed by the animation system each frame.
#[derive(Clone, Debug)]
pub struct SkeletonDebugInfo {
    /// World-space positions of each joint.
    pub world_positions: Vec<[f32; 3]>,
    /// Parent index for each joint (None = root).
    pub parents: Vec<Option<u32>>,
    /// Human-readable joint names.
    pub joint_names: Vec<String>,
}

/// Debug draw provider that renders skeletons as wireframe bones.
///
/// Draws a small sphere at each joint position and a line from each joint
/// to its parent.
pub struct SkeletonDebugDraw {
    skeletons: Mutex<Vec<SkeletonDebugInfo>>,
}

impl SkeletonDebugDraw {
    /// Create a new empty skeleton debug drawer.
    pub fn new() -> Self {
        Self {
            skeletons: Mutex::new(Vec::new()),
        }
    }

    /// Push skeleton debug info for the current frame.
    ///
    /// Called by the animation system during the update phase.
    pub fn push(&self, info: SkeletonDebugInfo) {
        if let Ok(mut guard) = self.skeletons.lock() {
            guard.push(info);
        }
    }

    /// Clear all pending skeleton debug info.
    pub fn clear(&self) {
        if let Ok(mut guard) = self.skeletons.lock() {
            guard.clear();
        }
    }
}

impl Default for SkeletonDebugDraw {
    fn default() -> Self {
        Self::new()
    }
}

impl DebugDrawProvider for SkeletonDebugDraw {
    fn name(&self) -> &str {
        "animation_skeleton"
    }

    fn populate(&self, buffer: &mut DebugDrawBuffer, _view: &Mat4, _proj: &Mat4) {
        let infos = match self.skeletons.lock() {
            Ok(mut guard) => std::mem::take(&mut *guard),
            Err(_) => return,
        };

        let joint_color = [0.3, 0.8, 1.0, 1.0]; // cyan
        let bone_color = [0.6, 0.9, 1.0, 0.6]; // light blue / translucent

        for info in &infos {
            // Draw joints as small spheres.
            for pos_3 in &info.world_positions {
                let center = Vec3::from(*pos_3);
                buffer.sphere_wireframe(center, 0.04, joint_color);
            }

            // Draw bones as arrows/lines from child to parent.
            for (i, parent_opt) in info.parents.iter().enumerate() {
                if let Some(parent_idx) = parent_opt {
                    let idx = *parent_idx as usize;
                    if idx < info.world_positions.len() && i < info.world_positions.len() {
                        let from = Vec3::from(info.world_positions[idx]);
                        let to = Vec3::from(info.world_positions[i]);
                        buffer.arrow(from, to, bone_color);
                    }
                }
            }
        }
    }
}
