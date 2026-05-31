//! IK debug visualization.
//!
//! Provides debug drawing for IK effectors (target markers) and IK chains
//! (bone chain lines) following the same pattern as [`SkeletonDebugDraw`].
//!
//! The [`IkDebugDraw`] struct collects IK debug info each frame and renders
//! it through the engine's [`DebugDrawProvider`] trait.

use std::sync::Mutex;

use engine_renderer::{DebugDrawBuffer, DebugDrawProvider};
use glam::{Mat4, Quat, Vec3};

use crate::ik::chain::IkChain;
use crate::ik::effector::IkEffector;

/// Per-frame IK debug info for a single character.
#[derive(Clone, Debug)]
pub struct IkDebugInfo {
    /// Name of the character / entity.
    pub name: String,
    /// Effector targets to visualise.
    pub effectors: Vec<IkEffector>,
    /// Chains to visualise (lines).
    pub chains: Vec<IkChain>,
    /// Current global bone positions for drawing chain lines.
    /// Indexed by BoneIndex.0.
    pub bone_positions: Vec<Vec3>,
}

/// Debug draw provider for IK systems.
///
/// Draws:
/// - Target markers (sphere + axis) at each effector position.
/// - Chain lines showing the bone chain from base to tip.
/// - Labels with effector names.
pub struct IkDebugDraw {
    infos: Mutex<Vec<IkDebugInfo>>,
}

impl IkDebugDraw {
    /// Create a new empty IK debug drawer.
    pub fn new() -> Self {
        Self {
            infos: Mutex::new(Vec::new()),
        }
    }

    /// Push IK debug info for the current frame.
    pub fn push(&self, info: IkDebugInfo) {
        if let Ok(mut guard) = self.infos.lock() {
            guard.push(info);
        }
    }

    /// Clear all pending IK debug info.
    pub fn clear(&self) {
        if let Ok(mut guard) = self.infos.lock() {
            guard.clear();
        }
    }
}

impl Default for IkDebugDraw {
    fn default() -> Self {
        Self::new()
    }
}

impl DebugDrawProvider for IkDebugDraw {
    fn name(&self) -> &str {
        "ik_debug"
    }

    fn populate(&self, buffer: &mut DebugDrawBuffer, _view: &Mat4, _proj: &Mat4) {
        let infos = match self.infos.lock() {
            Ok(mut guard) => std::mem::take(&mut *guard),
            Err(_) => return,
        };

        for info in &infos {
            // ── Draw effector targets ────────────────────────────────────
            let effector_color = [1.0, 0.6, 0.0, 1.0]; // orange
            let effector_axis_color = [0.8, 0.4, 0.0, 0.8];

            for effector in &info.effectors {
                if !effector.is_active() {
                    continue;
                }

                let pos = effector.position;
                let color = effector_color;

                // Draw target sphere.
                buffer.sphere_wireframe(pos, 0.06, color);

                // Draw local axes if rotation is not identity.
                if effector.rotation != Quat::IDENTITY {
                    let x_axis = effector.rotation * Vec3::X * 0.15;
                    let y_axis = effector.rotation * Vec3::Y * 0.15;
                    let z_axis = effector.rotation * Vec3::Z * 0.15;

                    buffer.arrow(pos, pos + x_axis, [1.0, 0.0, 0.0, 0.8]);
                    buffer.arrow(pos, pos + y_axis, [0.0, 1.0, 0.0, 0.8]);
                    buffer.arrow(pos, pos + z_axis, [0.0, 0.0, 1.0, 0.8]);
                }

                // Label.
                buffer.label(pos + Vec3::Y * 0.1, &effector.name, effector_axis_color);
            }

            // ── Draw chain bone lines ────────────────────────────────────
            let chain_color = [0.3, 0.8, 1.0, 0.6]; // light blue

            for chain in &info.chains {
                if !chain.is_active() {
                    continue;
                }

                // Draw lines connecting consecutive bones in the chain.
                for window in chain.bones.windows(2) {
                    let from_idx = window[0].0 as usize;
                    let to_idx = window[1].0 as usize;

                    if from_idx < info.bone_positions.len() && to_idx < info.bone_positions.len() {
                        let from = info.bone_positions[from_idx];
                        let to = info.bone_positions[to_idx];
                        buffer.arrow(from, to, chain_color);
                    }
                }

                // Label the chain tip and base.
                if let Some(tip) = chain.tip() {
                    let tip_idx = tip.0 as usize;
                    if tip_idx < info.bone_positions.len() {
                        buffer.label(
                            info.bone_positions[tip_idx] + Vec3::Y * 0.08,
                            format!("{}_tip", chain.name),
                            [0.5, 0.9, 1.0, 0.8],
                        );
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BoneIndex;

    #[test]
    fn ik_debug_draw_empty_no_crash() {
        let drawer = IkDebugDraw::new();
        let mut buf = DebugDrawBuffer::new();
        let view = Mat4::IDENTITY;
        let proj = Mat4::IDENTITY;
        drawer.populate(&mut buf, &view, &proj);
        assert!(buf.is_empty());
    }

    #[test]
    fn ik_debug_draw_with_effector() {
        let drawer = IkDebugDraw::new();
        let effector = IkEffector::new("hand_r", BoneIndex(5), Vec3::new(1.0, 0.0, 0.0));
        let chain = IkChain::new("arm_r", vec![BoneIndex(5), BoneIndex(4), BoneIndex(3)]);

        drawer.push(IkDebugInfo {
            name: "test".into(),
            effectors: vec![effector],
            chains: vec![chain],
            bone_positions: vec![Vec3::ZERO; 10],
        });

        let mut buf = DebugDrawBuffer::new();
        let view = Mat4::IDENTITY;
        let proj = Mat4::IDENTITY;
        drawer.populate(&mut buf, &view, &proj);

        // Should have at least some shapes.
        assert!(!buf.shapes.is_empty());
    }

    #[test]
    fn ik_debug_draw_multiple_frames() {
        let drawer = IkDebugDraw::new();
        drawer.push(IkDebugInfo {
            name: "a".into(),
            effectors: vec![],
            chains: vec![],
            bone_positions: vec![],
        });
        drawer.push(IkDebugInfo {
            name: "b".into(),
            effectors: vec![],
            chains: vec![],
            bone_positions: vec![],
        });

        // Populate should drain all.
        let mut buf = DebugDrawBuffer::new();
        drawer.populate(&mut buf, &Mat4::IDENTITY, &Mat4::IDENTITY);
        assert!(buf.is_empty()); // no effectors/chains → nothing drawn

        // Second populate should be empty.
        let mut buf2 = DebugDrawBuffer::new();
        drawer.populate(&mut buf2, &Mat4::IDENTITY, &Mat4::IDENTITY);
        assert!(buf2.is_empty());
    }
}
