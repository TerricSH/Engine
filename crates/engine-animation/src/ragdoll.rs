use std::collections::{BTreeMap, BTreeSet};

use engine_scene::Component;
use engine_serialize::Value;
use serde::{Deserialize, Serialize};

use crate::{BoneTransform, Skeleton};

pub const MAX_RAGDOLL_BODIES: usize = 128;
pub const MAX_RAGDOLL_CONSTRAINTS: usize = 256;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum RagdollMode {
    #[default]
    Animated,
    Simulated,
    Recovering,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum RagdollShape {
    Ball { radius: f32 },
    Capsule { half_height: f32, radius: f32 },
    Box { half_extents: [f32; 3] },
}

impl Default for RagdollShape {
    fn default() -> Self {
        Self::Capsule {
            half_height: 0.2,
            radius: 0.1,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RagdollBody {
    pub bone: String,
    pub shape: RagdollShape,
    pub local_translation: [f32; 3],
    pub local_rotation: [f32; 4],
    pub mass: f32,
    pub linear_damping: f32,
    pub angular_damping: f32,
}

impl Default for RagdollBody {
    fn default() -> Self {
        Self {
            bone: String::new(),
            shape: RagdollShape::default(),
            local_translation: [0.0; 3],
            local_rotation: [0.0, 0.0, 0.0, 1.0],
            mass: 1.0,
            linear_damping: 0.05,
            angular_damping: 0.1,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum RagdollJointType {
    Fixed,
    Revolute,
    #[default]
    Spherical,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RagdollConstraint {
    pub parent_bone: String,
    pub child_bone: String,
    pub joint_type: RagdollJointType,
    pub anchor_parent: [f32; 3],
    pub anchor_child: [f32; 3],
    pub axis: [f32; 3],
    pub limits: Option<[f32; 2]>,
    pub break_force: f32,
    pub break_torque: f32,
}

impl Default for RagdollConstraint {
    fn default() -> Self {
        Self {
            parent_bone: String::new(),
            child_bone: String::new(),
            joint_type: RagdollJointType::Spherical,
            anchor_parent: [0.0; 3],
            anchor_child: [0.0; 3],
            axis: [1.0, 0.0, 0.0],
            limits: None,
            break_force: 0.0,
            break_torque: 0.0,
        }
    }
}

/// Serializable ragdoll authoring and runtime ownership state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RagdollComponent {
    pub enabled: bool,
    pub mode: RagdollMode,
    pub recovery_duration: f32,
    pub recovery_elapsed: f32,
    pub bodies: Vec<RagdollBody>,
    pub constraints: Vec<RagdollConstraint>,
    /// Deterministic generated body IDs, persisted so checkpoints can rebuild
    /// the same physics ownership graph without serializing backend handles.
    pub generated_body_ids: BTreeMap<String, String>,
    pub generated_joint_ids: Vec<String>,
    pub pending_impulse: [f32; 3],
    pub impulse_pending: bool,
}

impl Default for RagdollComponent {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: RagdollMode::Animated,
            recovery_duration: 0.35,
            recovery_elapsed: 0.0,
            bodies: Vec::new(),
            constraints: Vec::new(),
            generated_body_ids: BTreeMap::new(),
            generated_joint_ids: Vec::new(),
            pending_impulse: [0.0; 3],
            impulse_pending: false,
        }
    }
}

impl Component for RagdollComponent {
    const TYPE_ID: &'static str = "engine.ragdoll";
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum RagdollPartRole {
    Body,
    Joint,
}

/// Internal ownership marker for generated ragdoll entities.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RagdollPart {
    pub owner_id: String,
    pub role: RagdollPartRole,
    pub key: String,
}

impl Component for RagdollPart {
    const TYPE_ID: &'static str = "engine.ragdoll_part";
}

impl RagdollComponent {
    pub fn validate(&self) -> Result<(), String> {
        if !self.recovery_duration.is_finite()
            || self.recovery_duration < 0.0
            || !self.recovery_elapsed.is_finite()
            || self.recovery_elapsed < 0.0
            || !self.pending_impulse.iter().all(|value| value.is_finite())
        {
            return Err("ragdoll recovery timing must be finite and non-negative".into());
        }
        if self.bodies.is_empty() {
            return Err("ragdoll must define at least one body".into());
        }
        if self.bodies.len() > MAX_RAGDOLL_BODIES {
            return Err(format!(
                "ragdoll has {} bodies; maximum is {MAX_RAGDOLL_BODIES}",
                self.bodies.len()
            ));
        }
        if self.constraints.len() > MAX_RAGDOLL_CONSTRAINTS {
            return Err(format!(
                "ragdoll has {} constraints; maximum is {MAX_RAGDOLL_CONSTRAINTS}",
                self.constraints.len()
            ));
        }

        let mut body_bones = BTreeSet::new();
        for body in &self.bodies {
            if body.bone.is_empty() || !body_bones.insert(body.bone.as_str()) {
                return Err(format!(
                    "ragdoll body bone names must be non-empty and unique: '{}'",
                    body.bone
                ));
            }
            if !body
                .local_translation
                .iter()
                .chain(body.local_rotation.iter())
                .chain([body.mass, body.linear_damping, body.angular_damping].iter())
                .all(|value| value.is_finite())
            {
                return Err(format!(
                    "ragdoll body '{}' contains non-finite data",
                    body.bone
                ));
            }
            let rotation_length_squared = body
                .local_rotation
                .iter()
                .map(|value| value * value)
                .sum::<f32>();
            if rotation_length_squared <= f32::EPSILON {
                return Err(format!(
                    "ragdoll body '{}' has a zero-length local rotation",
                    body.bone
                ));
            }
            if body.mass <= 0.0 || body.linear_damping < 0.0 || body.angular_damping < 0.0 {
                return Err(format!(
                    "ragdoll body '{}' mass must be positive and damping non-negative",
                    body.bone
                ));
            }
            match body.shape {
                RagdollShape::Ball { radius } => {
                    if !radius.is_finite() || radius <= 0.0 {
                        return Err(format!(
                            "ragdoll body '{}' ball radius must be positive",
                            body.bone
                        ));
                    }
                }
                RagdollShape::Capsule {
                    half_height,
                    radius,
                } => {
                    if !half_height.is_finite()
                        || !radius.is_finite()
                        || half_height < 0.0
                        || radius <= 0.0
                    {
                        return Err(format!(
                            "ragdoll body '{}' capsule dimensions are invalid",
                            body.bone
                        ));
                    }
                }
                RagdollShape::Box { half_extents } => {
                    if !half_extents
                        .iter()
                        .all(|value| value.is_finite() && *value > 0.0)
                    {
                        return Err(format!(
                            "ragdoll body '{}' box half-extents must be positive",
                            body.bone
                        ));
                    }
                }
            }
        }

        let mut constraint_children = BTreeSet::new();
        for constraint in &self.constraints {
            if constraint.parent_bone == constraint.child_bone
                || !body_bones.contains(constraint.parent_bone.as_str())
                || !body_bones.contains(constraint.child_bone.as_str())
            {
                return Err(format!(
                    "ragdoll constraint '{} -> {}' must reference two different bodies",
                    constraint.parent_bone, constraint.child_bone
                ));
            }
            if !constraint_children.insert(constraint.child_bone.as_str()) {
                return Err(format!(
                    "ragdoll child body '{}' has more than one constraint",
                    constraint.child_bone
                ));
            }
            if !constraint
                .anchor_parent
                .iter()
                .chain(constraint.anchor_child.iter())
                .chain(constraint.axis.iter())
                .chain([constraint.break_force, constraint.break_torque].iter())
                .all(|value| value.is_finite())
                || constraint.break_force < 0.0
                || constraint.break_torque < 0.0
            {
                return Err(format!(
                    "ragdoll constraint '{} -> {}' contains invalid values",
                    constraint.parent_bone, constraint.child_bone
                ));
            }
            if matches!(constraint.joint_type, RagdollJointType::Revolute)
                && constraint
                    .axis
                    .iter()
                    .map(|value| value * value)
                    .sum::<f32>()
                    <= f32::EPSILON
            {
                return Err(format!(
                    "ragdoll revolute constraint '{}' needs a non-zero axis",
                    constraint.child_bone
                ));
            }
            if let Some([min, max]) = constraint.limits {
                if !min.is_finite() || !max.is_finite() || min > max {
                    return Err(format!(
                        "ragdoll constraint '{}' limits must be finite and ordered",
                        constraint.child_bone
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn validate_for_skeleton(&self, skeleton: &Skeleton) -> Result<(), String> {
        self.validate()?;
        let skeleton_bones = skeleton
            .joints()
            .iter()
            .map(|joint| joint.name.as_str())
            .collect::<BTreeSet<_>>();
        for body in &self.bodies {
            if !skeleton_bones.contains(body.bone.as_str()) {
                return Err(format!(
                    "ragdoll body '{}' does not exist in the skeleton",
                    body.bone
                ));
            }
        }
        Ok(())
    }
}

/// Frame-local pose supplied by an external owner such as ragdoll physics.
#[derive(Clone, Debug)]
pub struct ExternalPoseOverride {
    pub local_transforms: Vec<BoneTransform>,
    pub weight: f32,
}

pub(crate) fn serialize_ragdoll(component: &dyn std::any::Any) -> BTreeMap<String, Value> {
    let Some(ragdoll) = component.downcast_ref::<RagdollComponent>() else {
        return BTreeMap::new();
    };
    let mut fields = BTreeMap::new();
    fields.insert("enabled".into(), Value::Bool(ragdoll.enabled));
    fields.insert(
        "mode".into(),
        Value::Enum(
            match ragdoll.mode {
                RagdollMode::Animated => "Animated",
                RagdollMode::Simulated => "Simulated",
                RagdollMode::Recovering => "Recovering",
            }
            .into(),
        ),
    );
    fields.insert(
        "recovery_duration".into(),
        Value::Float32(ragdoll.recovery_duration),
    );
    fields.insert(
        "recovery_elapsed".into(),
        Value::Float32(ragdoll.recovery_elapsed),
    );
    fields.insert(
        "bodies".into(),
        Value::List(
            ragdoll
                .bodies
                .iter()
                .map(|body| {
                    let mut values = BTreeMap::from([
                        ("bone".into(), Value::Str(body.bone.clone())),
                        (
                            "local_translation".into(),
                            Value::Vec3(body.local_translation),
                        ),
                        ("local_rotation".into(), Value::Quat(body.local_rotation)),
                        ("mass".into(), Value::Float32(body.mass)),
                        ("linear_damping".into(), Value::Float32(body.linear_damping)),
                        (
                            "angular_damping".into(),
                            Value::Float32(body.angular_damping),
                        ),
                    ]);
                    match body.shape {
                        RagdollShape::Ball { radius } => {
                            values.insert("shape".into(), Value::Enum("Ball".into()));
                            values.insert("radius".into(), Value::Float32(radius));
                        }
                        RagdollShape::Capsule {
                            half_height,
                            radius,
                        } => {
                            values.insert("shape".into(), Value::Enum("Capsule".into()));
                            values.insert("half_height".into(), Value::Float32(half_height));
                            values.insert("radius".into(), Value::Float32(radius));
                        }
                        RagdollShape::Box { half_extents } => {
                            values.insert("shape".into(), Value::Enum("Box".into()));
                            values.insert("half_extents".into(), Value::Vec3(half_extents));
                        }
                    }
                    Value::Map(values)
                })
                .collect(),
        ),
    );
    fields.insert(
        "constraints".into(),
        Value::List(
            ragdoll
                .constraints
                .iter()
                .map(|constraint| {
                    let mut values = BTreeMap::from([
                        (
                            "parent_bone".into(),
                            Value::Str(constraint.parent_bone.clone()),
                        ),
                        (
                            "child_bone".into(),
                            Value::Str(constraint.child_bone.clone()),
                        ),
                        (
                            "joint_type".into(),
                            Value::Enum(
                                match constraint.joint_type {
                                    RagdollJointType::Fixed => "Fixed",
                                    RagdollJointType::Revolute => "Revolute",
                                    RagdollJointType::Spherical => "Spherical",
                                }
                                .into(),
                            ),
                        ),
                        (
                            "anchor_parent".into(),
                            Value::Vec3(constraint.anchor_parent),
                        ),
                        ("anchor_child".into(), Value::Vec3(constraint.anchor_child)),
                        ("axis".into(), Value::Vec3(constraint.axis)),
                        ("break_force".into(), Value::Float32(constraint.break_force)),
                        (
                            "break_torque".into(),
                            Value::Float32(constraint.break_torque),
                        ),
                    ]);
                    if let Some([min, max]) = constraint.limits {
                        values.insert(
                            "limits".into(),
                            Value::List(vec![Value::Float32(min), Value::Float32(max)]),
                        );
                    }
                    Value::Map(values)
                })
                .collect(),
        ),
    );
    fields.insert(
        "generated_body_ids".into(),
        Value::Map(
            ragdoll
                .generated_body_ids
                .iter()
                .map(|(bone, id)| (bone.clone(), Value::Str(id.clone())))
                .collect(),
        ),
    );
    fields.insert(
        "generated_joint_ids".into(),
        Value::List(
            ragdoll
                .generated_joint_ids
                .iter()
                .cloned()
                .map(Value::Str)
                .collect(),
        ),
    );
    fields.insert(
        "pending_impulse".into(),
        Value::Vec3(ragdoll.pending_impulse),
    );
    fields.insert(
        "impulse_pending".into(),
        Value::Bool(ragdoll.impulse_pending),
    );
    fields
}

pub(crate) fn deserialize_ragdoll(fields: &BTreeMap<String, Value>) -> Box<dyn std::any::Any> {
    let defaults = RagdollComponent::default();
    let bodies = list_maps(fields.get("bodies"))
        .map(|values| values.map(decode_body).collect())
        .unwrap_or_default();
    let constraints = list_maps(fields.get("constraints"))
        .map(|values| values.map(decode_constraint).collect())
        .unwrap_or_default();
    let generated_body_ids = match fields.get("generated_body_ids") {
        Some(Value::Map(values)) => values
            .iter()
            .filter_map(|(bone, value)| match value {
                Value::Str(id) if !bone.is_empty() && !id.is_empty() => {
                    Some((bone.clone(), id.clone()))
                }
                _ => None,
            })
            .collect(),
        _ => BTreeMap::new(),
    };
    let generated_joint_ids = match fields.get("generated_joint_ids") {
        Some(Value::List(values)) => values
            .iter()
            .filter_map(|value| match value {
                Value::Str(id) if !id.is_empty() => Some(id.clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    Box::new(RagdollComponent {
        enabled: bool_value(fields.get("enabled")).unwrap_or(true),
        mode: match enum_value(fields.get("mode")) {
            Some("Simulated") => RagdollMode::Simulated,
            Some("Recovering") => RagdollMode::Recovering,
            _ => RagdollMode::Animated,
        },
        recovery_duration: float_value(fields.get("recovery_duration"))
            .filter(|value| value.is_finite() && *value >= 0.0)
            .unwrap_or(defaults.recovery_duration),
        recovery_elapsed: float_value(fields.get("recovery_elapsed"))
            .filter(|value| value.is_finite() && *value >= 0.0)
            .unwrap_or(0.0),
        bodies,
        constraints,
        generated_body_ids,
        generated_joint_ids,
        pending_impulse: vec3_value(fields.get("pending_impulse")).unwrap_or([0.0; 3]),
        impulse_pending: bool_value(fields.get("impulse_pending")).unwrap_or(false),
    })
}

pub(crate) fn serialize_ragdoll_part(component: &dyn std::any::Any) -> BTreeMap<String, Value> {
    let Some(part) = component.downcast_ref::<RagdollPart>() else {
        return BTreeMap::new();
    };
    BTreeMap::from([
        ("owner_id".into(), Value::Entity(part.owner_id.clone())),
        (
            "role".into(),
            Value::Enum(
                match part.role {
                    RagdollPartRole::Body => "Body",
                    RagdollPartRole::Joint => "Joint",
                }
                .into(),
            ),
        ),
        ("key".into(), Value::Str(part.key.clone())),
    ])
}

pub(crate) fn deserialize_ragdoll_part(fields: &BTreeMap<String, Value>) -> Box<dyn std::any::Any> {
    let owner_id = match fields.get("owner_id") {
        Some(Value::Entity(value)) | Some(Value::Str(value)) => value.clone(),
        _ => String::new(),
    };
    Box::new(RagdollPart {
        owner_id,
        role: match enum_value(fields.get("role")) {
            Some("Joint") => RagdollPartRole::Joint,
            _ => RagdollPartRole::Body,
        },
        key: string_value(fields.get("key")).unwrap_or_default(),
    })
}

fn decode_body(values: &BTreeMap<String, Value>) -> RagdollBody {
    let defaults = RagdollBody::default();
    let radius = float_value(values.get("radius")).unwrap_or(0.1);
    let shape = match enum_value(values.get("shape")) {
        Some("Ball") => RagdollShape::Ball { radius },
        Some("Box") => RagdollShape::Box {
            half_extents: vec3_value(values.get("half_extents")).unwrap_or([0.1; 3]),
        },
        _ => RagdollShape::Capsule {
            half_height: float_value(values.get("half_height")).unwrap_or(0.2),
            radius,
        },
    };
    RagdollBody {
        bone: string_value(values.get("bone")).unwrap_or_default(),
        shape,
        local_translation: vec3_value(values.get("local_translation")).unwrap_or([0.0; 3]),
        local_rotation: quat_value(values.get("local_rotation")).unwrap_or([0.0, 0.0, 0.0, 1.0]),
        mass: float_value(values.get("mass")).unwrap_or(defaults.mass),
        linear_damping: float_value(values.get("linear_damping"))
            .unwrap_or(defaults.linear_damping),
        angular_damping: float_value(values.get("angular_damping"))
            .unwrap_or(defaults.angular_damping),
    }
}

fn decode_constraint(values: &BTreeMap<String, Value>) -> RagdollConstraint {
    let limits = match values.get("limits") {
        Some(Value::List(values)) if values.len() == 2 => Some([
            float_value(values.first()).unwrap_or(0.0),
            float_value(values.get(1)).unwrap_or(0.0),
        ]),
        _ => None,
    };
    RagdollConstraint {
        parent_bone: string_value(values.get("parent_bone")).unwrap_or_default(),
        child_bone: string_value(values.get("child_bone")).unwrap_or_default(),
        joint_type: match enum_value(values.get("joint_type")) {
            Some("Fixed") => RagdollJointType::Fixed,
            Some("Revolute") => RagdollJointType::Revolute,
            _ => RagdollJointType::Spherical,
        },
        anchor_parent: vec3_value(values.get("anchor_parent")).unwrap_or([0.0; 3]),
        anchor_child: vec3_value(values.get("anchor_child")).unwrap_or([0.0; 3]),
        axis: vec3_value(values.get("axis")).unwrap_or([1.0, 0.0, 0.0]),
        limits,
        break_force: float_value(values.get("break_force")).unwrap_or(0.0),
        break_torque: float_value(values.get("break_torque")).unwrap_or(0.0),
    }
}

fn list_maps(value: Option<&Value>) -> Option<impl Iterator<Item = &BTreeMap<String, Value>>> {
    match value {
        Some(Value::List(values)) => Some(values.iter().filter_map(|value| match value {
            Value::Map(values) => Some(values),
            _ => None,
        })),
        _ => None,
    }
}

fn bool_value(value: Option<&Value>) -> Option<bool> {
    match value {
        Some(Value::Bool(value)) => Some(*value),
        _ => None,
    }
}

fn float_value(value: Option<&Value>) -> Option<f32> {
    match value {
        Some(Value::Float32(value)) => Some(*value),
        Some(Value::Float64(value)) => Some(*value as f32),
        _ => None,
    }
}

fn string_value(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::Str(value)) => Some(value.clone()),
        _ => None,
    }
}

fn enum_value(value: Option<&Value>) -> Option<&str> {
    match value {
        Some(Value::Enum(value)) => Some(value),
        _ => None,
    }
}

fn vec3_value(value: Option<&Value>) -> Option<[f32; 3]> {
    match value {
        Some(Value::Vec3(value)) => Some(*value),
        _ => None,
    }
}

fn quat_value(value: Option<&Value>) -> Option<[f32; 4]> {
    match value {
        Some(Value::Quat(value)) => Some(*value),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Joint, JointTransform};

    fn test_ragdoll() -> RagdollComponent {
        RagdollComponent {
            bodies: vec![
                RagdollBody {
                    bone: "hips".into(),
                    ..Default::default()
                },
                RagdollBody {
                    bone: "chest".into(),
                    shape: RagdollShape::Box {
                        half_extents: [0.2, 0.3, 0.1],
                    },
                    mass: 3.0,
                    ..Default::default()
                },
            ],
            constraints: vec![RagdollConstraint {
                parent_bone: "hips".into(),
                child_bone: "chest".into(),
                limits: Some([-0.5, 0.5]),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn ragdoll_validates_against_skeleton_and_roundtrips_scene_fields() {
        let skeleton = Skeleton {
            joints: vec![
                Joint {
                    name: "hips".into(),
                    parent_index: None,
                    local_transform: JointTransform::IDENTITY,
                },
                Joint {
                    name: "chest".into(),
                    parent_index: Some(0),
                    local_transform: JointTransform::IDENTITY,
                },
            ],
            inverse_bind_matrices: vec![
                glam::Mat4::IDENTITY.to_cols_array_2d(),
                glam::Mat4::IDENTITY.to_cols_array_2d(),
            ],
        };
        let mut expected = test_ragdoll();
        expected.mode = RagdollMode::Simulated;
        expected
            .generated_body_ids
            .insert("hips".into(), "npc.__ragdoll.hips".into());
        expected
            .generated_joint_ids
            .push("npc.__ragdoll_joint.chest".into());
        expected.validate_for_skeleton(&skeleton).unwrap();

        let fields = serialize_ragdoll(&expected);
        let decoded = deserialize_ragdoll(&fields)
            .downcast::<RagdollComponent>()
            .unwrap();
        assert_eq!(*decoded, expected);
    }

    #[test]
    fn ragdoll_rejects_duplicate_bodies_and_unknown_skeleton_bones() {
        let mut ragdoll = test_ragdoll();
        ragdoll.bodies[1].bone = "hips".into();
        assert!(ragdoll.validate().unwrap_err().contains("unique"));

        let skeleton = Skeleton {
            joints: vec![Joint {
                name: "root".into(),
                parent_index: None,
                local_transform: JointTransform::IDENTITY,
            }],
            inverse_bind_matrices: vec![glam::Mat4::IDENTITY.to_cols_array_2d()],
        };
        assert!(test_ragdoll()
            .validate_for_skeleton(&skeleton)
            .unwrap_err()
            .contains("does not exist"));
    }
}
