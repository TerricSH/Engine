//! Joint constraints for IK solving.
//!
//! Constraints limit how much individual bones can deviate from their rest
//! pose during IK solving. This prevents unnatural joint configurations
//! such as hyper-extension of elbows or knees.

use serde::{Deserialize, Serialize};

use crate::BoneIndex;

/// Per-bone joint constraint applied after IK solves.
///
/// Constraints are specified as angular limits around the bone's rest-pose
/// local axes. They work by clamping the solved bone rotation so it stays
/// within a valid cone.
///
/// # Fields
///
/// | Field      | Type    | Description                                        |
/// |------------|---------|----------------------------------------------------|
/// | `bone`     | `BoneIndex` | The bone this constraint applies to             |
/// | `twist_min`  | `f32` | Min twist angle in degrees (default: -45)          |
/// | `twist_max`  | `f32` | Max twist angle in degrees (default: +45)          |
/// | `swing_min`  | `f32` | Min swing angle in degrees (default: -45)          |
/// | `swing_max`  | `f32` | Max swing angle in degrees (default: +45)          |
/// | `softness`   | `f32` | Softness `0..1`; 0 = hard clamp, 1 = very soft    |
/// | `stiffness`  | `f32` | Resistance to deviation from rest `0..1`           |
/// | `rest_angle` | `[f32; 4]` | Optional rest-pose quaternion (x,y,z,w)       |
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IkConstraint {
    /// Bone index this constraint applies to.
    pub bone: BoneIndex,

    // ── Twist limits (rotation around the bone's local forward axis) ────
    /// Minimum twist angle in degrees (negative = allow rotation both ways).
    #[serde(default = "default_twist_min")]
    pub twist_min: f32,
    /// Maximum twist angle in degrees.
    #[serde(default = "default_twist_max")]
    pub twist_max: f32,

    // ── Swing limits (rotation around lateral axes) ─────────────────────
    /// Minimum swing angle in degrees.
    #[serde(default = "default_swing_min")]
    pub swing_min: f32,
    /// Maximum swing angle in degrees.
    #[serde(default = "default_swing_max")]
    pub swing_max: f32,

    // ── Softness & stiffness ────────────────────────────────────────────
    /// Softness factor in `[0, 1]`. At 0.0 the constraint is a hard clamp;
    /// at 1.0 it's a very gradual spring.
    #[serde(default = "default_softness")]
    pub softness: f32,

    /// Stiffness factor in `[0, 1]`. At 0.0 the joint is free; at 1.0 it
    /// tries to stay at the rest angle.
    #[serde(default = "default_stiffness")]
    pub stiffness: f32,

    /// Rest-pose quaternion `(x, y, z, w)` for this bone in parent space.
    /// If not set, the constraint uses the skeleton's rest pose.
    #[serde(default)]
    pub rest_angle: Option<[f32; 4]>,
}

const fn default_twist_min() -> f32 {
    -45.0
}
const fn default_twist_max() -> f32 {
    45.0
}
const fn default_swing_min() -> f32 {
    -45.0
}
const fn default_swing_max() -> f32 {
    45.0
}
const fn default_softness() -> f32 {
    0.0
}
const fn default_stiffness() -> f32 {
    0.0
}

impl IkConstraint {
    /// Create a new constraint for the given bone with default angle limits.
    pub fn new(bone: BoneIndex) -> Self {
        Self {
            bone,
            twist_min: default_twist_min(),
            twist_max: default_twist_max(),
            swing_min: default_swing_min(),
            swing_max: default_swing_max(),
            softness: default_softness(),
            stiffness: default_stiffness(),
            rest_angle: None,
        }
    }

    /// Builder: set twist limits (degrees).
    pub fn with_twist(mut self, min: f32, max: f32) -> Self {
        self.twist_min = min;
        self.twist_max = max;
        self
    }

    /// Builder: set swing limits (degrees).
    pub fn with_swing(mut self, min: f32, max: f32) -> Self {
        self.swing_min = min;
        self.swing_max = max;
        self
    }

    /// Builder: set softness.
    pub fn with_softness(mut self, softness: f32) -> Self {
        self.softness = softness.clamp(0.0, 1.0);
        self
    }

    /// Builder: set stiffness.
    pub fn with_stiffness(mut self, stiffness: f32) -> Self {
        self.stiffness = stiffness.clamp(0.0, 1.0);
        self
    }

    /// Builder: set rest angle.
    pub fn with_rest_angle(mut self, quat: [f32; 4]) -> Self {
        self.rest_angle = Some(quat);
        self
    }

    /// Check whether this constraint has any limiting effect.
    pub fn is_active(&self) -> bool {
        self.twist_min > -180.0
            || self.twist_max < 180.0
            || self.swing_min > -180.0
            || self.swing_max < 180.0
            || self.stiffness > 0.0
    }
}

/// Parameters for configuring a constraint set across multiple bones.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IkConstraintSet {
    /// Per-bone constraints.
    pub constraints: Vec<IkConstraint>,
    /// Whether constraints are enabled globally.
    pub enabled: bool,
}

impl IkConstraintSet {
    pub fn new() -> Self {
        Self {
            constraints: Vec::new(),
            enabled: true,
        }
    }

    /// Add a constraint.
    pub fn add(&mut self, constraint: IkConstraint) {
        self.constraints.push(constraint);
    }

    /// Find the constraint for a given bone, if any.
    pub fn for_bone(&self, bone: BoneIndex) -> Option<&IkConstraint> {
        self.constraints.iter().find(|c| c.bone == bone)
    }

    /// Mutable access to a constraint for a bone.
    pub fn for_bone_mut(&mut self, bone: BoneIndex) -> Option<&mut IkConstraint> {
        self.constraints.iter_mut().find(|c| c.bone == bone)
    }
}

impl Default for IkConstraintSet {
    fn default() -> Self {
        Self::new()
    }
}

/// Pre-defined humanoid joint constraints following biomechanical limits.
pub mod humanoid_constraints {
    use crate::ik::constraint::IkConstraint;
    use crate::BoneIndex;

    /// Create an elbow constraint (hinge joint, ~0–150 degrees).
    pub fn elbow(bone: BoneIndex) -> IkConstraint {
        IkConstraint::new(bone)
            .with_twist(-5.0, 5.0)
            .with_swing(0.0, 150.0)
            .with_stiffness(0.1)
    }

    /// Create a knee constraint (hinge joint, ~0–140 degrees).
    pub fn knee(bone: BoneIndex) -> IkConstraint {
        IkConstraint::new(bone)
            .with_twist(-5.0, 5.0)
            .with_swing(0.0, 140.0)
            .with_stiffness(0.1)
    }

    /// Create a shoulder constraint (ball-and-socket).
    pub fn shoulder(bone: BoneIndex) -> IkConstraint {
        IkConstraint::new(bone)
            .with_twist(-60.0, 60.0)
            .with_swing(-90.0, 90.0)
            .with_stiffness(0.05)
    }

    /// Create a hip constraint (ball-and-socket).
    pub fn hip(bone: BoneIndex) -> IkConstraint {
        IkConstraint::new(bone)
            .with_twist(-30.0, 30.0)
            .with_swing(-45.0, 120.0)
            .with_stiffness(0.1)
    }

    /// Create a neck constraint (limited ball-and-socket).
    pub fn neck(bone: BoneIndex) -> IkConstraint {
        IkConstraint::new(bone)
            .with_twist(-45.0, 45.0)
            .with_swing(-30.0, 30.0)
            .with_stiffness(0.2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constraint_defaults() {
        let c = IkConstraint::new(BoneIndex(2));
        assert_eq!(c.bone, BoneIndex(2));
        assert!((c.twist_min - (-45.0)).abs() < 1e-6);
        assert!((c.twist_max - 45.0).abs() < 1e-6);
        assert!((c.softness - 0.0).abs() < 1e-6);
        assert!((c.stiffness - 0.0).abs() < 1e-6);
        assert!(c.rest_angle.is_none());
    }

    #[test]
    fn constraint_builder() {
        let c = IkConstraint::new(BoneIndex(5))
            .with_twist(-10.0, 10.0)
            .with_swing(-30.0, 80.0)
            .with_softness(0.3)
            .with_stiffness(0.5)
            .with_rest_angle([0.0, 0.0, 0.0, 1.0]);

        assert!((c.twist_min - (-10.0)).abs() < 1e-6);
        assert!((c.twist_max - 10.0).abs() < 1e-6);
        assert!((c.swing_min - (-30.0)).abs() < 1e-6);
        assert!((c.swing_max - 80.0).abs() < 1e-6);
        assert!((c.softness - 0.3).abs() < 1e-6);
        assert!((c.stiffness - 0.5).abs() < 1e-6);
        assert_eq!(c.rest_angle, Some([0.0, 0.0, 0.0, 1.0]));
    }

    #[test]
    fn constraint_active_check() {
        let c = IkConstraint::new(BoneIndex(0));
        assert!(c.is_active()); // default limits are active

        let c = IkConstraint::new(BoneIndex(0))
            .with_twist(-180.0, 180.0)
            .with_swing(-180.0, 180.0)
            .with_stiffness(0.0);
        assert!(!c.is_active()); // no limiting effect
    }

    #[test]
    fn constraint_set() {
        let mut set = IkConstraintSet::new();
        set.add(IkConstraint::new(BoneIndex(1)));
        set.add(IkConstraint::new(BoneIndex(2)));

        assert!(set.for_bone(BoneIndex(1)).is_some());
        assert!(set.for_bone(BoneIndex(3)).is_none());
        assert!(set.for_bone_mut(BoneIndex(2)).is_some());
    }

    #[test]
    fn humanoid_constraint_helpers() {
        let _ = humanoid_constraints::elbow(BoneIndex(0));
        let _ = humanoid_constraints::knee(BoneIndex(1));
        let _ = humanoid_constraints::shoulder(BoneIndex(2));
        let _ = humanoid_constraints::hip(BoneIndex(3));
        let _ = humanoid_constraints::neck(BoneIndex(4));
    }

    #[test]
    fn constraint_serialize_roundtrip() {
        let c = IkConstraint::new(BoneIndex(3))
            .with_twist(-20.0, 30.0)
            .with_softness(0.5);
        let bytes = bincode::serialize(&c).unwrap();
        let restored: IkConstraint = bincode::deserialize(&bytes).unwrap();
        assert_eq!(restored.bone, BoneIndex(3));
        assert!((restored.twist_min - (-20.0)).abs() < 1e-6);
        assert!((restored.twist_max - 30.0).abs() < 1e-6);
        assert!((restored.softness - 0.5).abs() < 1e-6);
    }
}
