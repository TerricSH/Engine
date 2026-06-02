//! IK effector and target types.
//!
//! An [`IkEffector`] is a **control point** attached to a specific bone. It defines
//! a target position (and optionally rotation) that the IK solver tries to make the
//! bone reach. Multiple effectors can target the same bone with blended weights.
//!
//! # Control points
//!
//! Each effector acts as a humanoid control point — such as a hand IK target, foot
//! IK target, or head look-at target. The effector's `weight` field lets you blend
//! between the animated pose and the IK-solved pose for smooth transitions.

use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::BoneIndex;

/// Coordinate space for an IK effector's target transform.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub enum IkEffectorSpace {
    /// Target is in world space (absolute position in the scene).
    #[default]
    World,
    /// Target is in the local space of the effector's parent bone.
    Local,
}

/// An IK effector — a **control point** that drives a bone toward a target.
///
/// # Fields
///
/// | Field          | Type            | Description                                    |
/// |----------------|-----------------|------------------------------------------------|
/// | `name`         | `String`        | Human-readable label (e.g. "hand_ik_r")        |
/// | `bone`         | `BoneIndex`     | The bone this effector controls                |
/// | `position`     | `Vec3`          | Target position (in `space`)                   |
/// | `rotation`     | `Quat`          | Target rotation (optional, identity = ignored) |
/// | `weight`       | `f32`           | Blend weight 0..1 (1.0 = full IK)              |
/// | `pole_vector`  | `Option<Vec3>`  | Pole direction for elbow/knee hints            |
/// | `space`        | `IkEffectorSpace` | Coordinate space of position/rotation        |
/// | `influence_chain` | `Option<usize>` | If set, restricts to a specific IK chain index |
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IkEffector {
    /// Human-readable label (used for editor identification).
    pub name: String,

    /// The bone this effector is attached to.
    pub bone: BoneIndex,

    /// Target position (in the space specified by [`space`](Self::space)).
    pub position: Vec3,

    /// Target rotation. When `IDENTITY` the rotation is ignored by the solver.
    pub rotation: Quat,

    /// Blend weight in `[0, 1]`. `0.0` = no effect, `1.0` = full IK solve.
    pub weight: f32,

    /// Optional pole vector that hints the direction of intermediate joints
    /// (e.g. for elbow or knee orientation in a 3-bone chain).
    pub pole_vector: Option<Vec3>,

    /// Coordinate space for `position` and `rotation`.
    #[serde(default)]
    pub space: IkEffectorSpace,

    /// If `Some(chain_index)`, this effector only applies to that IK chain.
    /// `None` means it applies to all chains that contain its bone.
    pub influence_chain: Option<usize>,

    /// Whether this effector is active.
    pub enabled: bool,
}

impl IkEffector {
    /// Create a new effector targeting `bone` at `position`.
    ///
    /// Defaults: rotation = identity, weight = 1.0, world space, enabled.
    pub fn new(name: impl Into<String>, bone: BoneIndex, position: Vec3) -> Self {
        Self {
            name: name.into(),
            bone,
            position,
            rotation: Quat::IDENTITY,
            weight: 1.0,
            pole_vector: None,
            space: IkEffectorSpace::World,
            influence_chain: None,
            enabled: true,
        }
    }

    /// Builder: set the target rotation.
    pub fn with_rotation(mut self, rotation: Quat) -> Self {
        self.rotation = rotation;
        self
    }

    /// Builder: set the blend weight.
    pub fn with_weight(mut self, weight: f32) -> Self {
        self.weight = weight.clamp(0.0, 1.0);
        self
    }

    /// Builder: set the pole vector hint.
    pub fn with_pole(mut self, pole: Vec3) -> Self {
        self.pole_vector = Some(pole);
        self
    }

    /// Builder: set the coordinate space.
    pub fn with_space(mut self, space: IkEffectorSpace) -> Self {
        self.space = space;
        self
    }

    /// Builder: restrict to a specific IK chain.
    pub fn with_chain(mut self, chain_index: usize) -> Self {
        self.influence_chain = Some(chain_index);
        self
    }

    /// Returns `true` if this effector has an active non-zero weight.
    pub fn is_active(&self) -> bool {
        self.enabled && self.weight > 0.0
    }
}

// ---------------------------------------------------------------------------
// Pre-defined humanoid control point helpers
// ---------------------------------------------------------------------------

/// Common humanoid IK control point labels.
pub mod humanoid {
    /// Right hand IK target.
    pub const HAND_R: &str = "hand_ik_r";
    /// Left hand IK target.
    pub const HAND_L: &str = "hand_ik_l";
    /// Right foot IK target.
    pub const FOOT_R: &str = "foot_ik_r";
    /// Left foot IK target.
    pub const FOOT_L: &str = "foot_ik_l";
    /// Head look-at target.
    pub const HEAD: &str = "head_lookat";
    /// Pelvis / hip target.
    pub const PELVIS: &str = "pelvis_ik";
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    #[test]
    fn effector_new_defaults() {
        let e = IkEffector::new("foot_ik_r", BoneIndex(5), Vec3::new(1.0, 0.0, 0.0));
        assert_eq!(e.name, "foot_ik_r");
        assert_eq!(e.bone, BoneIndex(5));
        assert_eq!(e.position, Vec3::new(1.0, 0.0, 0.0));
        assert_eq!(e.rotation, Quat::IDENTITY);
        assert!((e.weight - 1.0).abs() < 1e-6);
        assert!(e.pole_vector.is_none());
        assert_eq!(e.space, IkEffectorSpace::World);
        assert!(e.enabled);
        assert!(e.is_active());
    }

    #[test]
    fn effector_builder_methods() {
        let e = IkEffector::new("hand_l", BoneIndex(3), Vec3::ZERO)
            .with_rotation(Quat::from_rotation_z(1.0))
            .with_weight(0.5)
            .with_pole(Vec3::Y)
            .with_space(IkEffectorSpace::Local)
            .with_chain(1);

        assert!((e.weight - 0.5).abs() < 1e-6);
        assert!(e.pole_vector == Some(Vec3::Y));
        assert_eq!(e.space, IkEffectorSpace::Local);
        assert_eq!(e.influence_chain, Some(1));
    }

    #[test]
    fn effector_weight_zero_is_inactive() {
        let e = IkEffector::new("test", BoneIndex(0), Vec3::ZERO).with_weight(0.0);
        assert!(!e.is_active());
    }

    #[test]
    fn effector_disabled_is_inactive() {
        let mut e = IkEffector::new("test", BoneIndex(0), Vec3::ZERO);
        e.enabled = false;
        assert!(!e.is_active());
    }

    #[test]
    fn effector_humanoid_constants() {
        assert_eq!(humanoid::HAND_R, "hand_ik_r");
        assert_eq!(humanoid::HAND_L, "hand_ik_l");
        assert_eq!(humanoid::FOOT_R, "foot_ik_r");
        assert_eq!(humanoid::FOOT_L, "foot_ik_l");
        assert_eq!(humanoid::HEAD, "head_lookat");
        assert_eq!(humanoid::PELVIS, "pelvis_ik");
    }

    #[test]
    fn effector_serialize_roundtrip() {
        let e = IkEffector::new("hip", BoneIndex(2), Vec3::new(0.0, 1.0, 0.0)).with_weight(0.8);
        let bytes = bincode::serialize(&e).unwrap();
        let restored: IkEffector = bincode::deserialize(&bytes).unwrap();
        assert_eq!(restored.name, "hip");
        assert_eq!(restored.bone, BoneIndex(2));
        assert!((restored.weight - 0.8).abs() < 1e-6);
    }
}
