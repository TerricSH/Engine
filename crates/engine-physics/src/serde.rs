//! Serialize / deserialize hooks for physics components.
//!
//! These functions are registered as [`ComponentExtension`] hooks so that
//! physics components (RigidBody, Collider, PhysicsMaterial) can be saved
//! to and loaded from scene files through the `engine-scene` serialization
//! pipeline.

use std::collections::BTreeMap;

use engine_serialize::Value;

use crate::components::{BodyType, Collider, ColliderShape, PhysicsMaterial, RigidBody};
use crate::gravity::{GravityFalloff, GravityMode, GravitySource};

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
