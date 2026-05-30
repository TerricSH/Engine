use serde::{Deserialize, Serialize};

/// Which Rapier joint type to create.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum JointType {
    Fixed,
    /// Hinge — 1 DOF rotation around local X.
    Revolute,
    /// Slider — 1 DOF translation along local X.
    Prismatic,
    /// Ball — 3 DOF rotation.
    Spherical,
}

/// Configuration limits for a joint axis.
///
/// `stiffness` and `damping` are provided here for convenience but do not
/// correspond directly to Rapier's `JointLimits` (which only carries min/max).
/// They may be used by higher-level constraint solvers in the future.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JointLimits {
    pub min: f32,
    pub max: f32,
    pub stiffness: f32,
    pub damping: f32,
}

/// Motor settings for a joint axis.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JointMotor {
    pub target_vel: f32,
    pub target_pos: f32,
    pub stiffness: f32,
    pub damping: f32,
}

/// Engine-level joint descriptor (no Rapier handles exposed).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JointDescriptor {
    /// Index of the first entity.
    pub entity_a: u32,
    /// Index of the second entity (or same as `entity_a` for world-attached).
    pub entity_b: u32,
    pub joint_type: JointType,
    /// Local anchor frame relative to body A's position.
    pub anchor_a: [f32; 3],
    /// Local anchor frame relative to body B's position.
    pub anchor_b: [f32; 3],
    /// Axis for revolute / prismatic joints (local to anchor_a).
    pub axis: [f32; 3],
    pub limits: Option<JointLimits>,
    pub motor: Option<JointMotor>,
    /// Break force before the joint detaches (0 = unbreakable).
    pub break_force: f32,
    /// Break torque before the joint detaches (0 = unbreakable).
    pub break_torque: f32,
}

/// User-facing joint handle (opaque wrapper around a Rapier joint index).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct JointHandle(pub u32);
