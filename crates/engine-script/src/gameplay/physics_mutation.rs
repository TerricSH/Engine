use serde::{Deserialize, Serialize};

use super::validation::validate_entity_id;

/// Upper bound applied to script physics query distances and radii.
///
/// Script-provided distances are clamped to this value before touching the
/// physics backend so a misbehaving script cannot force unbounded native
/// work through the gameplay bridge.
pub const MAX_PHYSICS_QUERY_DISTANCE: f32 = 10_000.0;

/// Maximum persistent entity ids returned by a single overlap query.
pub const MAX_PHYSICS_OVERLAP_RESULTS: usize = 64;

/// Maximum script physics queries the runtime buffers from one command
/// drain. Queries beyond the cap are rejected with a script diagnostic.
pub const MAX_PENDING_PHYSICS_QUERIES: usize = 256;

/// Maximum script physics mutations the runtime buffers from one command
/// drain. Mutations beyond the cap are rejected with a script diagnostic.
pub const MAX_PENDING_PHYSICS_MUTATIONS: usize = 256;

/// Per-axis force/impulse bound accepted from untrusted script hosts.
pub const MAX_PHYSICS_MUTATION_COMPONENT: f32 = 1_000_000.0;

/// Joint type accepted by the script gameplay bridge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GameplayJointType {
    Fixed,
    Revolute,
    Prismatic,
    Spherical,
}

/// Optional authored limits for a script-created joint.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GameplayJointLimits {
    pub min: f32,
    pub max: f32,
    pub stiffness: f32,
    pub damping: f32,
}

/// Optional motor configuration for a script-created joint.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GameplayJointMotor {
    pub target_vel: f32,
    pub target_pos: f32,
    pub stiffness: f32,
    pub damping: f32,
}

/// A bounded physics mutation requested by gameplay code.
///
/// The runtime resolves persistent IDs at the frame boundary and never
/// exposes raw ECS or physics-backend handles to managed code.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GameplayPhysicsMutation {
    ApplyForce {
        entity_id: String,
        force: [f32; 3],
    },
    ApplyImpulse {
        entity_id: String,
        impulse: [f32; 3],
    },
    ApplyTorque {
        entity_id: String,
        torque: [f32; 3],
    },
    ApplyTorqueImpulse {
        entity_id: String,
        torque_impulse: [f32; 3],
    },
    /// Create or replace a persistent joint. Reusing `joint_id` is the
    /// supported way to update limits, motor targets, anchors, or break
    /// thresholds without leaking backend handles.
    CreateJoint {
        joint_id: String,
        body_a: String,
        body_b: String,
        joint_type: GameplayJointType,
        anchor_a: [f32; 3],
        anchor_b: [f32; 3],
        axis: [f32; 3],
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limits: Option<GameplayJointLimits>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        motor: Option<GameplayJointMotor>,
        break_force: f32,
        break_torque: f32,
    },
    RemoveJoint {
        joint_id: String,
    },
}

impl GameplayPhysicsMutation {
    pub fn entity_id(&self) -> &str {
        match self {
            Self::ApplyForce { entity_id, .. }
            | Self::ApplyImpulse { entity_id, .. }
            | Self::ApplyTorque { entity_id, .. }
            | Self::ApplyTorqueImpulse { entity_id, .. } => entity_id,
            Self::CreateJoint { body_a, .. } => body_a,
            Self::RemoveJoint { joint_id } => joint_id,
        }
    }

    /// Persistent entities that must already exist when this command enters
    /// the native runtime. A create-joint command deliberately omits its
    /// `joint_id` because that dedicated constraint entity is created by the
    /// engine.
    pub fn required_existing_entity_ids(&self) -> Vec<&str> {
        match self {
            Self::ApplyForce { entity_id, .. }
            | Self::ApplyImpulse { entity_id, .. }
            | Self::ApplyTorque { entity_id, .. }
            | Self::ApplyTorqueImpulse { entity_id, .. } => vec![entity_id],
            Self::CreateJoint { body_a, body_b, .. } => vec![body_a, body_b],
            Self::RemoveJoint { joint_id } => vec![joint_id],
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::ApplyForce { entity_id, force } => {
                validate_entity_id(entity_id)?;
                validate_physics_vector("force", force, false)
            }
            Self::ApplyImpulse { entity_id, impulse } => {
                validate_entity_id(entity_id)?;
                validate_physics_vector("impulse", impulse, false)
            }
            Self::ApplyTorque { entity_id, torque } => {
                validate_entity_id(entity_id)?;
                validate_physics_vector("torque", torque, false)
            }
            Self::ApplyTorqueImpulse {
                entity_id,
                torque_impulse,
            } => {
                validate_entity_id(entity_id)?;
                validate_physics_vector("torque impulse", torque_impulse, false)
            }
            Self::CreateJoint {
                joint_id,
                body_a,
                body_b,
                joint_type,
                anchor_a,
                anchor_b,
                axis,
                limits,
                motor,
                break_force,
                break_torque,
            } => {
                validate_entity_id(joint_id)?;
                validate_entity_id(body_a)?;
                validate_entity_id(body_b)?;
                if body_a == body_b {
                    return Err("joint body_a and body_b must name different entities".into());
                }
                validate_physics_vector("joint anchor_a", anchor_a, false)?;
                validate_physics_vector("joint anchor_b", anchor_b, false)?;
                validate_physics_vector("joint axis", axis, false)?;
                if matches!(
                    joint_type,
                    GameplayJointType::Revolute | GameplayJointType::Prismatic
                ) && axis.iter().map(|value| value * value).sum::<f32>() <= f32::EPSILON
                {
                    return Err("revolute/prismatic joint axis must be non-zero".into());
                }
                if let Some(limits) = limits {
                    let values = [limits.min, limits.max, limits.stiffness, limits.damping];
                    if !values.iter().all(|value| value.is_finite())
                        || values
                            .iter()
                            .any(|value| value.abs() > MAX_PHYSICS_MUTATION_COMPONENT)
                        || limits.min > limits.max
                        || limits.stiffness < 0.0
                        || limits.damping < 0.0
                    {
                        return Err("joint limits are invalid or out of range".into());
                    }
                }
                if let Some(motor) = motor {
                    let values = [
                        motor.target_vel,
                        motor.target_pos,
                        motor.stiffness,
                        motor.damping,
                    ];
                    if !values.iter().all(|value| value.is_finite())
                        || values
                            .iter()
                            .any(|value| value.abs() > MAX_PHYSICS_MUTATION_COMPONENT)
                        || motor.stiffness < 0.0
                        || motor.damping < 0.0
                    {
                        return Err("joint motor is invalid or out of range".into());
                    }
                }
                for (name, value) in [
                    ("break_force", *break_force),
                    ("break_torque", *break_torque),
                ] {
                    if !value.is_finite()
                        || !(0.0..=MAX_PHYSICS_MUTATION_COMPONENT).contains(&value)
                    {
                        return Err(format!("{name} is invalid or out of range"));
                    }
                }
                Ok(())
            }
            Self::RemoveJoint { joint_id } => validate_entity_id(joint_id),
        }
    }
}

fn validate_physics_vector(kind: &str, vector: &[f32; 3], reject_zero: bool) -> Result<(), String> {
    if !vector.iter().all(|value| value.is_finite()) {
        return Err(format!("{kind} must contain only finite values"));
    }
    if vector
        .iter()
        .any(|value| value.abs() > MAX_PHYSICS_MUTATION_COMPONENT)
    {
        return Err(format!(
            "{kind} components must not exceed {MAX_PHYSICS_MUTATION_COMPONENT}"
        ));
    }
    if reject_zero && vector.iter().map(|value| value * value).sum::<f32>() <= f32::EPSILON {
        return Err(format!("{kind} must be non-zero"));
    }
    Ok(())
}
