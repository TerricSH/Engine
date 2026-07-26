use serde::{Deserialize, Serialize};

use crate::{Component, Entity};

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

/// Persistent, scene-serializable joint component.
///
/// Attach this to a dedicated persistent "constraint entity". `body_a` and
/// `body_b` name the rigid-body entities by persistent ID, so the joint can be
/// rebuilt after scene reload, checkpoint restore, streaming, or ECS handle
/// recycling without serializing backend handles.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PhysicsJoint {
    pub enabled: bool,
    pub body_a: String,
    pub body_b: String,
    pub joint_type: JointType,
    pub anchor_a: [f32; 3],
    pub anchor_b: [f32; 3],
    pub axis: [f32; 3],
    pub limits: Option<JointLimits>,
    pub motor: Option<JointMotor>,
    /// Break force in newtons. `0` means unbreakable.
    pub break_force: f32,
    /// Break torque in newton-metres. `0` means unbreakable.
    pub break_torque: f32,
}

impl Default for PhysicsJoint {
    fn default() -> Self {
        Self {
            enabled: true,
            body_a: String::new(),
            body_b: String::new(),
            joint_type: JointType::Fixed,
            anchor_a: [0.0; 3],
            anchor_b: [0.0; 3],
            axis: [1.0, 0.0, 0.0],
            limits: None,
            motor: None,
            break_force: 0.0,
            break_torque: 0.0,
        }
    }
}

impl Component for PhysicsJoint {
    const TYPE_ID: &'static str = "engine.physics.joint";
}

impl PhysicsJoint {
    /// Validate authored or script-provided joint data before it reaches
    /// Rapier. Entity existence is checked separately by the ECS synchronizer.
    pub fn validate(&self) -> Result<(), String> {
        if self.body_a.is_empty() || self.body_b.is_empty() {
            return Err("joint body_a and body_b must be non-empty persistent IDs".into());
        }
        if self.body_a == self.body_b {
            return Err("joint body_a and body_b must name different entities".into());
        }
        if !self
            .anchor_a
            .iter()
            .chain(self.anchor_b.iter())
            .chain(self.axis.iter())
            .all(|value| value.is_finite())
        {
            return Err("joint anchors and axis must contain only finite values".into());
        }
        if matches!(self.joint_type, JointType::Revolute | JointType::Prismatic)
            && self.axis.iter().map(|value| value * value).sum::<f32>() <= f32::EPSILON
        {
            return Err("revolute/prismatic joint axis must be non-zero".into());
        }
        if let Some(limits) = &self.limits {
            if ![limits.min, limits.max, limits.stiffness, limits.damping]
                .iter()
                .all(|value| value.is_finite())
                || limits.min > limits.max
                || limits.stiffness < 0.0
                || limits.damping < 0.0
            {
                return Err(
                    "joint limits must be finite, ordered, and have non-negative tuning".into(),
                );
            }
        }
        if let Some(motor) = &self.motor {
            if ![
                motor.target_vel,
                motor.target_pos,
                motor.stiffness,
                motor.damping,
            ]
            .iter()
            .all(|value| value.is_finite())
                || motor.stiffness < 0.0
                || motor.damping < 0.0
            {
                return Err("joint motor values must be finite with non-negative tuning".into());
            }
        }
        if !self.break_force.is_finite()
            || !self.break_torque.is_finite()
            || self.break_force < 0.0
            || self.break_torque < 0.0
        {
            return Err("joint break thresholds must be finite and non-negative".into());
        }
        Ok(())
    }

    pub(crate) fn descriptor(&self, entity_a: Entity, entity_b: Entity) -> JointDescriptor {
        JointDescriptor {
            entity_a,
            entity_b,
            joint_type: self.joint_type.clone(),
            anchor_a: self.anchor_a,
            anchor_b: self.anchor_b,
            axis: self.axis,
            limits: self.limits.clone(),
            motor: self.motor.clone(),
            break_force: self.break_force,
            break_torque: self.break_torque,
        }
    }
}

/// Engine-level joint descriptor (no Rapier handles exposed).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JointDescriptor {
    /// Complete handle of the first entity.
    pub entity_a: Entity,
    /// Complete handle of the second entity (or same as `entity_a` for world-attached).
    pub entity_b: Entity,
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

impl JointDescriptor {
    pub fn validate(&self) -> Result<(), String> {
        let component = PhysicsJoint {
            enabled: true,
            body_a: "body-a".into(),
            body_b: "body-b".into(),
            joint_type: self.joint_type.clone(),
            anchor_a: self.anchor_a,
            anchor_b: self.anchor_b,
            axis: self.axis,
            limits: self.limits.clone(),
            motor: self.motor.clone(),
            break_force: self.break_force,
            break_torque: self.break_torque,
        };
        if self.entity_a == self.entity_b {
            return Err("joint entities must be different".into());
        }
        component.validate()
    }
}

/// User-facing joint handle (opaque wrapper around a Rapier joint index).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct JointHandle(pub u32);
