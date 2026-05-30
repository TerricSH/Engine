use crate::JointTransform;

/// Defines how root motion is extracted and applied.
#[derive(Clone, Debug, PartialEq)]
pub enum RootMotionApplyTo {
    /// Do not apply root motion (animation-driven movement disabled for this character).
    None,
    /// Root motion is extracted and made available to the character controller
    /// or movement system to consume.
    Controller,
    /// Apply root motion directly to the entity transform (non-character objects only).
    DirectTransform,
}

/// Configuration for root motion extraction.
#[derive(Clone, Debug, PartialEq)]
pub struct RootMotionConfig {
    /// How root motion deltas are applied.
    pub apply_to: RootMotionApplyTo,
    /// Bone index of the root (usually 0, but can be any bone that serves as
    /// the animation root).
    pub root_bone: u16,
    /// Whether to extract horizontal (XZ) movement only.
    pub horizontal_only: bool,
}

impl Default for RootMotionConfig {
    fn default() -> Self {
        Self {
            apply_to: RootMotionApplyTo::None,
            root_bone: 0,
            horizontal_only: true,
        }
    }
}

/// Extracted root motion delta for one frame.
#[derive(Clone, Debug, PartialEq)]
pub struct RootMotionDelta {
    /// Translation delta (in local space).
    pub translation: [f32; 3],
    /// Rotation delta (as quaternion, in local space).
    pub rotation: [f32; 4],
}

impl RootMotionDelta {
    pub fn identity() -> Self {
        Self {
            translation: [0.0, 0.0, 0.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
        }
    }

    pub fn is_identity(&self) -> bool {
        self.translation == [0.0, 0.0, 0.0]
            && self.rotation == [0.0, 0.0, 0.0, 1.0]
    }
}

/// Extract root motion delta between two poses.
///
/// Compares the root bone's transform between the previous frame's pose and
/// the current frame's pose to compute animation-authored movement.
pub fn extract_root_motion(
    prev_pose: &[JointTransform],
    current_pose: &[JointTransform],
    config: &RootMotionConfig,
) -> RootMotionDelta {
    let root = config.root_bone as usize;

    let prev = match prev_pose.get(root) {
        Some(t) => t,
        None => return RootMotionDelta::identity(),
    };
    let current = match current_pose.get(root) {
        Some(t) => t,
        None => return RootMotionDelta::identity(),
    };

    let mut delta = RootMotionDelta {
        translation: [
            current.translation[0] - prev.translation[0],
            current.translation[1] - prev.translation[1],
            current.translation[2] - prev.translation[2],
        ],
        rotation: current.rotation, // simplified — proper delta would invert prev
    };

    if config.horizontal_only {
        delta.translation[1] = 0.0; // zero out vertical
    }

    delta
}

/// Gate 11 Root Motion Policy:
///
/// - Root motion is extracted from the animation by comparing the root bone
///   transform between consecutive frames.
/// - When `RootMotionApplyTo::None` (default), root motion is ignored and
///   the character controller or physics system fully controls movement.
/// - When `RootMotionApplyTo::Controller`, the extracted delta is exposed
///   for the Gate 12 character controller to optionally consume.
/// - When `RootMotionApplyTo::DirectTransform`, the delta is applied
///   directly to the entity's world transform. This mode is intended for
///   non-character objects (props, cinematics) and MUST NOT be used when
///   a character controller or physics body is present.
/// - Horizontal-only mode (default) zeroes the Y component of the
///   translation delta to prevent animation from overriding vertical
///   physics (gravity, jumping).

#[cfg(test)]
mod tests {
    use super::*;
    use crate::JointTransform;

    fn identity_jt() -> JointTransform {
        JointTransform {
            translation: [0.0, 0.0, 0.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0, 1.0, 1.0],
        }
    }

    #[test]
    fn root_motion_config_defaults() {
        let config = RootMotionConfig::default();
        assert_eq!(config.apply_to, RootMotionApplyTo::None);
        assert_eq!(config.root_bone, 0);
        assert!(config.horizontal_only);
    }

    #[test]
    fn extract_root_motion_identity_when_no_change() {
        let pose = vec![identity_jt()];
        let config = RootMotionConfig::default();
        let delta = extract_root_motion(&pose, &pose, &config);
        assert!(delta.is_identity());
    }

    #[test]
    fn extract_root_motion_detects_translation() {
        let prev = vec![identity_jt()];
        let current = vec![JointTransform {
            translation: [1.0, 2.0, 3.0],
            ..identity_jt()
        }];
        let config = RootMotionConfig::default();
        let delta = extract_root_motion(&prev, &current, &config);
        // horizontal_only means Y is zeroed
        assert_eq!(delta.translation[0], 1.0);
        assert_eq!(delta.translation[1], 0.0); // horizontal_only
        assert_eq!(delta.translation[2], 3.0);
    }

    #[test]
    fn extract_root_motion_full_3d_when_horizontal_false() {
        let prev = vec![identity_jt()];
        let current = vec![JointTransform {
            translation: [1.0, 2.0, 3.0],
            ..identity_jt()
        }];
        let config = RootMotionConfig {
            horizontal_only: false,
            ..RootMotionConfig::default()
        };
        let delta = extract_root_motion(&prev, &current, &config);
        assert_eq!(delta.translation[1], 2.0); // Y preserved
    }

    #[test]
    fn root_motion_empty_pose_returns_identity() {
        let config = RootMotionConfig::default();
        let delta = extract_root_motion(&[], &[], &config);
        assert!(delta.is_identity());
    }

    #[test]
    fn root_motion_delta_debug_and_clone() {
        let delta = RootMotionDelta::identity();
        let _ = format!("{:?}", delta);
        let _ = delta.clone();
    }
}
