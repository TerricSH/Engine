//! Serialize / deserialize hooks for physics components.
//!
//! These functions are registered as [`ComponentExtension`] hooks so that
//! physics components (RigidBody, Collider, PhysicsMaterial) can be saved
//! to and loaded from scene files through the `engine-scene` serialization
//! pipeline.

use std::collections::BTreeMap;

use engine_serialize::Value;

use crate::components::{BodyType, Collider, ColliderShape, PhysicsMaterial, RigidBody};

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
