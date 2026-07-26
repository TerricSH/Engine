//! Serialize / deserialize hooks for physics components.
//!
//! These functions are registered as [`ComponentExtension`] hooks so that
//! physics components (RigidBody, Collider, PhysicsMaterial) can be saved
//! to and loaded from scene files through the `engine-scene` serialization
//! pipeline.

use std::collections::BTreeMap;

use engine_serialize::Value;

use crate::components::{BodyType, Collider, ColliderShape, PhysicsMaterial, RigidBody};
use crate::destruction::Destructible;
use crate::gravity::{GravityFalloff, GravityMode, GravitySource};
use crate::joints::{JointLimits, JointMotor, JointType, PhysicsJoint};

pub(super) fn serialize_destructible(component: &dyn std::any::Any) -> BTreeMap<String, Value> {
    let destructible = component
        .downcast_ref::<Destructible>()
        .expect("Destructible expected");
    let mut fields = BTreeMap::new();
    fields.insert("enabled".into(), Value::Bool(destructible.enabled));
    fields.insert("max_health".into(), Value::Float32(destructible.max_health));
    fields.insert("health".into(), Value::Float32(destructible.health));
    fields.insert(
        "minimum_damage".into(),
        Value::Float32(destructible.minimum_damage),
    );
    fields.insert(
        "damage_scale".into(),
        Value::Float32(destructible.damage_scale),
    );
    if let Some(prefab) = &destructible.replacement_prefab {
        fields.insert("replacement_prefab".into(), Value::Asset(prefab.clone()));
    }
    fields.insert(
        "destroy_on_break".into(),
        Value::Bool(destructible.destroy_on_break),
    );
    fields.insert(
        "inherit_velocity".into(),
        Value::Bool(destructible.inherit_velocity),
    );
    fields.insert(
        "fracture_impulse_scale".into(),
        Value::Float32(destructible.fracture_impulse_scale),
    );
    fields.insert("broken".into(), Value::Bool(destructible.broken));
    fields
}

pub(super) fn deserialize_destructible(fields: &BTreeMap<String, Value>) -> Box<dyn std::any::Any> {
    let defaults = Destructible::default();
    let max_health = finite_non_negative_field(fields, "max_health")
        .filter(|value| *value > 0.0)
        .unwrap_or(defaults.max_health);
    let broken = bool_field(fields, "broken").unwrap_or(false);
    let health = finite_non_negative_field(fields, "health")
        .unwrap_or(if broken { 0.0 } else { max_health })
        .min(max_health);
    Box::new(Destructible {
        enabled: bool_field(fields, "enabled").unwrap_or(true),
        max_health,
        health: if broken { 0.0 } else { health },
        minimum_damage: finite_non_negative_field(fields, "minimum_damage")
            .unwrap_or(defaults.minimum_damage),
        damage_scale: finite_non_negative_field(fields, "damage_scale")
            .unwrap_or(defaults.damage_scale),
        replacement_prefab: match fields.get("replacement_prefab") {
            Some(Value::Asset(asset)) => Some(asset.clone()),
            _ => None,
        },
        destroy_on_break: bool_field(fields, "destroy_on_break")
            .or_else(|| bool_field(fields, "destroy_source"))
            .unwrap_or(true),
        inherit_velocity: bool_field(fields, "inherit_velocity").unwrap_or(true),
        fracture_impulse_scale: finite_non_negative_field(fields, "fracture_impulse_scale")
            .unwrap_or(defaults.fracture_impulse_scale),
        broken,
    })
}

// ══════════════════════════════════════════════════════════════════════════════
// RigidBody
// ══════════════════════════════════════════════════════════════════════════════

pub(super) fn serialize_rigid_body(component: &dyn std::any::Any) -> BTreeMap<String, Value> {
    let rb = component
        .downcast_ref::<RigidBody>()
        .expect("RigidBody expected");
    let mut fields = BTreeMap::new();
    fields.insert(
        "body_type".into(),
        Value::Enum(match rb.body_type {
            BodyType::Static => "Static".into(),
            BodyType::Dynamic => "Dynamic".into(),
            BodyType::Kinematic => "Kinematic".into(),
        }),
    );
    fields.insert("mass".into(), Value::Float32(rb.mass));
    fields.insert("linear_damping".into(), Value::Float32(rb.linear_damping));
    fields.insert("angular_damping".into(), Value::Float32(rb.angular_damping));
    fields.insert("enabled".into(), Value::Bool(rb.enabled));
    fields.insert("gravity_scale".into(), Value::Float32(rb.gravity_scale));
    fields.insert("can_sleep".into(), Value::Bool(rb.can_sleep));
    fields.insert("ccd_enabled".into(), Value::Bool(rb.ccd_enabled));
    fields
}

pub(super) fn deserialize_rigid_body(fields: &BTreeMap<String, Value>) -> Box<dyn std::any::Any> {
    let body_type = match fields.get("body_type") {
        Some(Value::Enum(s)) if s == "Static" => BodyType::Static,
        Some(Value::Enum(s)) if s == "Kinematic" => BodyType::Kinematic,
        _ => BodyType::Dynamic,
    };
    let mass = match fields.get("mass") {
        Some(Value::Float32(v)) => *v,
        Some(Value::Float64(v)) => *v as f32,
        _ => RigidBody::default().mass,
    };
    let linear_damping = match fields.get("linear_damping") {
        Some(Value::Float32(v)) => *v,
        _ => RigidBody::default().linear_damping,
    };
    let angular_damping = match fields.get("angular_damping") {
        Some(Value::Float32(v)) => *v,
        _ => RigidBody::default().angular_damping,
    };
    let enabled = match fields.get("enabled") {
        Some(Value::Bool(v)) => *v,
        _ => true,
    };
    let gravity_scale = match fields.get("gravity_scale") {
        Some(Value::Float32(v)) => *v,
        _ => RigidBody::default().gravity_scale,
    };
    let can_sleep = match fields.get("can_sleep") {
        Some(Value::Bool(v)) => *v,
        _ => true,
    };
    let ccd_enabled = match fields.get("ccd_enabled") {
        Some(Value::Bool(v)) => *v,
        _ => false,
    };
    Box::new(RigidBody {
        body_type,
        mass,
        linear_damping,
        angular_damping,
        enabled,
        gravity_scale,
        can_sleep,
        ccd_enabled,
    })
}

// ---------------------------------------------------------------------------
// Persistent physics joint
// ---------------------------------------------------------------------------

pub(super) fn serialize_physics_joint(component: &dyn std::any::Any) -> BTreeMap<String, Value> {
    let joint = component
        .downcast_ref::<PhysicsJoint>()
        .expect("PhysicsJoint expected");
    let mut fields = BTreeMap::new();
    fields.insert("enabled".into(), Value::Bool(joint.enabled));
    fields.insert("body_a".into(), Value::Str(joint.body_a.clone()));
    fields.insert("body_b".into(), Value::Str(joint.body_b.clone()));
    fields.insert(
        "joint_type".into(),
        Value::Enum(
            match joint.joint_type {
                JointType::Fixed => "Fixed",
                JointType::Revolute => "Revolute",
                JointType::Prismatic => "Prismatic",
                JointType::Spherical => "Spherical",
            }
            .into(),
        ),
    );
    fields.insert("anchor_a".into(), Value::Vec3(joint.anchor_a));
    fields.insert("anchor_b".into(), Value::Vec3(joint.anchor_b));
    fields.insert("axis".into(), Value::Vec3(joint.axis));
    if let Some(limits) = &joint.limits {
        fields.insert(
            "limits".into(),
            Value::Map(BTreeMap::from([
                ("min".into(), Value::Float32(limits.min)),
                ("max".into(), Value::Float32(limits.max)),
                ("stiffness".into(), Value::Float32(limits.stiffness)),
                ("damping".into(), Value::Float32(limits.damping)),
            ])),
        );
    }
    if let Some(motor) = &joint.motor {
        fields.insert(
            "motor".into(),
            Value::Map(BTreeMap::from([
                ("target_vel".into(), Value::Float32(motor.target_vel)),
                ("target_pos".into(), Value::Float32(motor.target_pos)),
                ("stiffness".into(), Value::Float32(motor.stiffness)),
                ("damping".into(), Value::Float32(motor.damping)),
            ])),
        );
    }
    fields.insert("break_force".into(), Value::Float32(joint.break_force));
    fields.insert("break_torque".into(), Value::Float32(joint.break_torque));
    fields
}

pub(super) fn deserialize_physics_joint(
    fields: &BTreeMap<String, Value>,
) -> Box<dyn std::any::Any> {
    let defaults = PhysicsJoint::default();
    let body_a = string_field(fields, "body_a").unwrap_or_default();
    let body_b = string_field(fields, "body_b").unwrap_or_default();
    let joint_type = match fields.get("joint_type") {
        Some(Value::Enum(value)) if value == "Revolute" => JointType::Revolute,
        Some(Value::Enum(value)) if value == "Prismatic" => JointType::Prismatic,
        Some(Value::Enum(value)) if value == "Spherical" => JointType::Spherical,
        _ => JointType::Fixed,
    };
    let limits = map_field(fields, "limits").map(|values| JointLimits {
        min: finite_float_field(values, "min").unwrap_or(0.0),
        max: finite_float_field(values, "max").unwrap_or(0.0),
        stiffness: finite_non_negative_field(values, "stiffness").unwrap_or(0.0),
        damping: finite_non_negative_field(values, "damping").unwrap_or(0.0),
    });
    let motor = map_field(fields, "motor").map(|values| JointMotor {
        target_vel: finite_float_field(values, "target_vel").unwrap_or(0.0),
        target_pos: finite_float_field(values, "target_pos").unwrap_or(0.0),
        stiffness: finite_non_negative_field(values, "stiffness").unwrap_or(0.0),
        damping: finite_non_negative_field(values, "damping").unwrap_or(0.0),
    });
    let joint = PhysicsJoint {
        enabled: bool_field(fields, "enabled").unwrap_or(true),
        body_a,
        body_b,
        joint_type,
        anchor_a: finite_vec3_field(fields, "anchor_a").unwrap_or(defaults.anchor_a),
        anchor_b: finite_vec3_field(fields, "anchor_b").unwrap_or(defaults.anchor_b),
        axis: finite_vec3_field(fields, "axis").unwrap_or(defaults.axis),
        limits,
        motor,
        break_force: finite_non_negative_field(fields, "break_force").unwrap_or(0.0),
        break_torque: finite_non_negative_field(fields, "break_torque").unwrap_or(0.0),
    };
    Box::new(joint)
}

// ══════════════════════════════════════════════════════════════════════════════
// Collider
// ══════════════════════════════════════════════════════════════════════════════

pub(super) fn serialize_collider(component: &dyn std::any::Any) -> BTreeMap<String, Value> {
    let c = component
        .downcast_ref::<Collider>()
        .expect("Collider expected");
    let mut fields = BTreeMap::new();

    let (shape_kind, shape_fields) = match &c.shape {
        ColliderShape::Cuboid { hx, hy, hz } => (
            "Cuboid",
            vec![
                ("hx".into(), Value::Float32(*hx)),
                ("hy".into(), Value::Float32(*hy)),
                ("hz".into(), Value::Float32(*hz)),
            ]
            .into_iter()
            .collect::<BTreeMap<_, _>>(),
        ),
        ColliderShape::Ball { radius } => (
            "Ball",
            vec![("radius".into(), Value::Float32(*radius))]
                .into_iter()
                .collect(),
        ),
        ColliderShape::Capsule {
            half_height,
            radius,
        } => (
            "Capsule",
            vec![
                ("half_height".into(), Value::Float32(*half_height)),
                ("radius".into(), Value::Float32(*radius)),
            ]
            .into_iter()
            .collect(),
        ),
        ColliderShape::HeightField {
            rows,
            columns,
            heights,
            scale,
        } => (
            "HeightField",
            vec![
                ("rows".into(), Value::UInt(u64::from(*rows))),
                ("columns".into(), Value::UInt(u64::from(*columns))),
                (
                    "heights".into(),
                    Value::List(heights.iter().copied().map(Value::Float32).collect()),
                ),
                ("scale".into(), Value::Vec3(*scale)),
            ]
            .into_iter()
            .collect(),
        ),
    };

    // Serialize shape as a map with "kind" and "params".
    let mut shape_value = BTreeMap::new();
    shape_value.insert("kind".into(), Value::Enum(shape_kind.into()));
    shape_value.insert("params".into(), Value::Map(shape_fields));
    fields.insert("shape".into(), Value::Map(shape_value));

    fields.insert("density".into(), Value::Float32(c.density));
    fields.insert("friction".into(), Value::Float32(c.friction));
    fields.insert("restitution".into(), Value::Float32(c.restitution));
    fields.insert("is_trigger".into(), Value::Bool(c.is_trigger));
    fields.insert(
        "collision_group".into(),
        Value::UInt(c.collision_group as u64),
    );
    fields.insert(
        "collision_mask".into(),
        Value::UInt(c.collision_mask as u64),
    );
    fields
}

pub(super) fn deserialize_collider(fields: &BTreeMap<String, Value>) -> Box<dyn std::any::Any> {
    // Deserialize shape.
    let shape = match fields.get("shape") {
        Some(Value::Map(shape_map)) => {
            let kind = match shape_map.get("kind") {
                Some(Value::Enum(k)) => k.as_str(),
                _ => "Cuboid",
            };
            let params = match shape_map.get("params") {
                Some(Value::Map(m)) => m,
                _ => &BTreeMap::new(),
            };
            match kind {
                "Ball" => ColliderShape::Ball {
                    radius: float_field(params, "radius").unwrap_or(0.5),
                },
                "Capsule" => ColliderShape::Capsule {
                    half_height: float_field(params, "half_height").unwrap_or(0.5),
                    radius: float_field(params, "radius").unwrap_or(0.25),
                },
                "HeightField" => {
                    let rows = uint_field(params, "rows")
                        .and_then(|value| u32::try_from(value).ok())
                        .unwrap_or(0);
                    let columns = uint_field(params, "columns")
                        .and_then(|value| u32::try_from(value).ok())
                        .unwrap_or(0);
                    let heights = match params.get("heights") {
                        Some(Value::List(values)) => values
                            .iter()
                            .filter_map(|value| match value {
                                Value::Float32(value) => Some(*value),
                                Value::Float64(value) => Some(*value as f32),
                                _ => None,
                            })
                            .collect(),
                        _ => Vec::new(),
                    };
                    let scale = match params.get("scale") {
                        Some(Value::Vec3(value)) => *value,
                        _ => [1.0; 3],
                    };
                    ColliderShape::HeightField {
                        rows,
                        columns,
                        heights,
                        scale,
                    }
                }
                _ => ColliderShape::Cuboid {
                    hx: float_field(params, "hx").unwrap_or(0.5),
                    hy: float_field(params, "hy").unwrap_or(0.5),
                    hz: float_field(params, "hz").unwrap_or(0.5),
                },
            }
        }
        _ => ColliderShape::default(),
    };

    let density = float_field(fields, "density").unwrap_or(Collider::default().density);
    let friction = float_field(fields, "friction").unwrap_or(Collider::default().friction);
    let restitution = float_field(fields, "restitution").unwrap_or(Collider::default().restitution);
    let is_trigger = bool_field(fields, "is_trigger").unwrap_or(false);
    let collision_group = uint_field(fields, "collision_group").unwrap_or(0xFFFF_FFFF) as u32;
    let collision_mask = uint_field(fields, "collision_mask").unwrap_or(0xFFFF_FFFF) as u32;

    Box::new(Collider {
        shape,
        density,
        friction,
        restitution,
        is_trigger,
        collision_group,
        collision_mask,
    })
}

// ══════════════════════════════════════════════════════════════════════════════
// PhysicsMaterial
// ══════════════════════════════════════════════════════════════════════════════

pub(super) fn serialize_physics_material(component: &dyn std::any::Any) -> BTreeMap<String, Value> {
    let m = component
        .downcast_ref::<PhysicsMaterial>()
        .expect("PhysicsMaterial expected");
    let mut fields = BTreeMap::new();
    fields.insert("friction".into(), Value::Float32(m.friction));
    fields.insert("restitution".into(), Value::Float32(m.restitution));
    fields.insert("density".into(), Value::Float32(m.density));
    fields
}

pub(super) fn deserialize_physics_material(
    fields: &BTreeMap<String, Value>,
) -> Box<dyn std::any::Any> {
    let friction = float_field(fields, "friction").unwrap_or(PhysicsMaterial::default().friction);
    let restitution =
        float_field(fields, "restitution").unwrap_or(PhysicsMaterial::default().restitution);
    let density = float_field(fields, "density").unwrap_or(PhysicsMaterial::default().density);
    Box::new(PhysicsMaterial {
        friction,
        restitution,
        density,
    })
}

// ══════════════════════════════════════════════════════════════════════════════
// GravitySource
// ══════════════════════════════════════════════════════════════════════════════

pub(super) fn serialize_gravity_source(component: &dyn std::any::Any) -> BTreeMap<String, Value> {
    let source = component
        .downcast_ref::<GravitySource>()
        .expect("GravitySource expected");
    let mut fields = BTreeMap::new();
    fields.insert(
        "mode".into(),
        Value::Enum(match source.mode {
            GravityMode::Directional => "Directional".into(),
            GravityMode::Point => "Point".into(),
        }),
    );
    fields.insert("enabled".into(), Value::Bool(source.enabled));
    fields.insert("strength".into(), Value::Float32(source.strength));
    fields.insert("direction".into(), Value::Vec3(source.direction.to_array()));
    fields.insert("center".into(), Value::Vec3(source.center.to_array()));
    fields.insert(
        "falloff".into(),
        Value::Enum(match source.falloff {
            GravityFalloff::None => "None".into(),
            GravityFalloff::Linear => "Linear".into(),
            GravityFalloff::InverseSquare => "InverseSquare".into(),
        }),
    );
    if let Some(radius) = source.max_radius {
        fields.insert("max_radius".into(), Value::Float32(radius));
    }
    fields
}

pub(super) fn deserialize_gravity_source(
    fields: &BTreeMap<String, Value>,
) -> Box<dyn std::any::Any> {
    let defaults = GravitySource::default();
    let mode = match fields.get("mode") {
        Some(Value::Enum(mode)) if mode == "Point" => GravityMode::Point,
        _ => GravityMode::Directional,
    };
    let enabled = bool_field(fields, "enabled").unwrap_or(true);
    // Every stored value must stay finite: non-finite scene data falls back
    // to the component defaults so resolution never sees NaN/inf.
    let strength = float_field(fields, "strength")
        .filter(|value| value.is_finite())
        .unwrap_or(defaults.strength);
    let direction = vec3_field(fields, "direction")
        .filter(|value| value.is_finite())
        .unwrap_or(defaults.direction);
    let center = vec3_field(fields, "center")
        .filter(|value| value.is_finite())
        .unwrap_or(defaults.center);
    let falloff = match fields.get("falloff") {
        Some(Value::Enum(falloff)) if falloff == "Linear" => GravityFalloff::Linear,
        Some(Value::Enum(falloff)) if falloff == "InverseSquare" => GravityFalloff::InverseSquare,
        _ => GravityFalloff::None,
    };
    let max_radius =
        float_field(fields, "max_radius").filter(|value| value.is_finite() && *value > 0.0);
    Box::new(GravitySource {
        mode,
        enabled,
        strength,
        direction,
        center,
        falloff,
        max_radius,
    })
}

// ══════════════════════════════════════════════════════════════════════════════
// ColliderShape default (for deserialization fallback)
// ══════════════════════════════════════════════════════════════════════════════

impl Default for ColliderShape {
    fn default() -> Self {
        ColliderShape::Cuboid {
            hx: 0.5,
            hy: 0.5,
            hz: 0.5,
        }
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// Field extraction helpers
// ══════════════════════════════════════════════════════════════════════════════

fn float_field(fields: &BTreeMap<String, Value>, key: &str) -> Option<f32> {
    match fields.get(key)? {
        Value::Float32(v) => Some(*v),
        Value::Float64(v) => Some(*v as f32),
        _ => None,
    }
}

fn vec3_field(fields: &BTreeMap<String, Value>, key: &str) -> Option<glam::Vec3> {
    match fields.get(key)? {
        Value::Vec3(values) => Some(glam::Vec3::from_array(*values)),
        _ => None,
    }
}

fn bool_field(fields: &BTreeMap<String, Value>, key: &str) -> Option<bool> {
    match fields.get(key)? {
        Value::Bool(v) => Some(*v),
        _ => None,
    }
}

fn uint_field(fields: &BTreeMap<String, Value>, key: &str) -> Option<u64> {
    match fields.get(key)? {
        Value::UInt(v) => Some(*v),
        Value::Int(v) if *v >= 0 => Some(*v as u64),
        _ => None,
    }
}

fn string_field(fields: &BTreeMap<String, Value>, key: &str) -> Option<String> {
    match fields.get(key)? {
        Value::Str(value) => Some(value.clone()),
        _ => None,
    }
}

fn map_field<'a>(
    fields: &'a BTreeMap<String, Value>,
    key: &str,
) -> Option<&'a BTreeMap<String, Value>> {
    match fields.get(key)? {
        Value::Map(value) => Some(value),
        _ => None,
    }
}

fn finite_float_field(fields: &BTreeMap<String, Value>, key: &str) -> Option<f32> {
    float_field(fields, key).filter(|value| value.is_finite())
}

fn finite_non_negative_field(fields: &BTreeMap<String, Value>, key: &str) -> Option<f32> {
    finite_float_field(fields, key).filter(|value| *value >= 0.0)
}

fn finite_vec3_field(fields: &BTreeMap<String, Value>, key: &str) -> Option<[f32; 3]> {
    match fields.get(key)? {
        Value::Vec3(value) if value.iter().all(|component| component.is_finite()) => Some(*value),
        _ => None,
    }
}
