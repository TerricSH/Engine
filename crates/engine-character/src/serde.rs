//! Serialize / deserialize hooks for the character controller component.
//!
//! These functions are registered as [`ComponentExtension`] hooks so that
//! `CharacterController` can be saved to and loaded from scene files through
//! the `engine-scene` serialization pipeline.

use std::collections::BTreeMap;

use engine_serialize::Value;

use crate::controller::CharacterController;

/// Serialize a `CharacterController` component into a field map.
pub fn serialize_character_controller(
    component: &dyn std::any::Any,
) -> BTreeMap<String, Value> {
    let ctrl = component
        .downcast_ref::<CharacterController>()
        .expect("CharacterController expected");
    let mut fields = BTreeMap::new();

    // ── Capsule shape ────────────────────────────────────────────────────
    fields.insert("height".into(), Value::Float32(ctrl.height));
    fields.insert("radius".into(), Value::Float32(ctrl.radius));

    // ── Movement parameters ──────────────────────────────────────────────
    fields.insert("move_speed".into(), Value::Float32(ctrl.move_speed));
    fields.insert("acceleration".into(), Value::Float32(ctrl.acceleration));
    fields.insert("deceleration".into(), Value::Float32(ctrl.deceleration));
    fields.insert(
        "air_acceleration".into(),
        Value::Float32(ctrl.air_acceleration),
    );
    fields.insert(
        "air_deceleration".into(),
        Value::Float32(ctrl.air_deceleration),
    );

    // ── Jump & gravity ───────────────────────────────────────────────────
    fields.insert(
        "jump_velocity".into(),
        Value::Float32(ctrl.jump_velocity),
    );
    fields.insert("gravity_scale".into(), Value::Float32(ctrl.gravity_scale));
    fields.insert(
        "max_fall_speed".into(),
        Value::Float32(ctrl.max_fall_speed),
    );

    // ── Collision ────────────────────────────────────────────────────────
    fields.insert("step_height".into(), Value::Float32(ctrl.step_height));
    fields.insert("slope_limit".into(), Value::Float32(ctrl.slope_limit));
    fields.insert("skin_offset".into(), Value::Float32(ctrl.skin_offset));

    // ── State ────────────────────────────────────────────────────────────
    fields.insert(
        "state".into(),
        Value::Enum(match ctrl.state {
            crate::CharacterState::Grounded => "Grounded".into(),
            crate::CharacterState::Jumping => "Jumping".into(),
            crate::CharacterState::Falling => "Falling".into(),
            crate::CharacterState::Landing => "Landing".into(),
            crate::CharacterState::Free => "Free".into(),
        }),
    );
    fields.insert("position".into(), Value::Vec3(ctrl.position.into()));
    fields.insert("velocity".into(), Value::Vec3(ctrl.velocity.into()));

    // ── Misc ─────────────────────────────────────────────────────────────
    fields.insert(
        "foot_ik_enabled".into(),
        Value::Bool(ctrl.foot_ik_enabled),
    );

    fields
}

/// Helper: extract a f32 from a Value field, handling both Float32 and Float64.
fn f32_field(fields: &BTreeMap<String, Value>, key: &str) -> Option<f32> {
    match fields.get(key) {
        Some(Value::Float32(v)) => Some(*v),
        Some(Value::Float64(v)) => Some(*v as f32),
        _ => None,
    }
}

/// Deserialize a `CharacterController` component from a field map.
pub fn deserialize_character_controller(
    fields: &BTreeMap<String, Value>,
) -> Box<dyn std::any::Any> {
    let mut ctrl = CharacterController::new();

    if let Some(v) = f32_field(fields, "height") {
        ctrl.height = v;
    }
    if let Some(v) = f32_field(fields, "radius") {
        ctrl.radius = v;
    }
    if let Some(v) = f32_field(fields, "move_speed") {
        ctrl.move_speed = v;
    }
    if let Some(v) = f32_field(fields, "acceleration") {
        ctrl.acceleration = v;
    }
    if let Some(v) = f32_field(fields, "deceleration") {
        ctrl.deceleration = v;
    }
    if let Some(v) = f32_field(fields, "air_acceleration") {
        ctrl.air_acceleration = v;
    }
    if let Some(v) = f32_field(fields, "air_deceleration") {
        ctrl.air_deceleration = v;
    }
    if let Some(v) = f32_field(fields, "jump_velocity") {
        ctrl.jump_velocity = v;
    }
    if let Some(v) = f32_field(fields, "gravity_scale") {
        ctrl.gravity_scale = v;
    }
    if let Some(v) = f32_field(fields, "max_fall_speed") {
        ctrl.max_fall_speed = v;
    }
    if let Some(v) = f32_field(fields, "step_height") {
        ctrl.step_height = v;
    }
    if let Some(v) = f32_field(fields, "slope_limit") {
        ctrl.slope_limit = v;
    }
    if let Some(v) = f32_field(fields, "skin_offset") {
        ctrl.skin_offset = v;
    }
    if let Some(Value::Enum(s)) = fields.get("state") {
        ctrl.state = match s.as_str() {
            "Grounded" => crate::CharacterState::Grounded,
            "Jumping" => crate::CharacterState::Jumping,
            "Falling" => crate::CharacterState::Falling,
            "Landing" => crate::CharacterState::Landing,
            "Free" => crate::CharacterState::Free,
            _ => crate::CharacterState::Falling,
        };
    }
    if let Some(Value::Vec3(v)) = fields.get("position") {
        ctrl.position = (*v).into();
    }
    if let Some(Value::Vec3(v)) = fields.get("velocity") {
        ctrl.velocity = (*v).into();
    }
    if let Some(Value::Bool(v)) = fields.get("foot_ik_enabled") {
        ctrl.foot_ik_enabled = *v;
    }

    Box::new(ctrl)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn character_controller_serde_roundtrip() {
        let mut ctrl = CharacterController::new();
        ctrl.height = 2.0;
        ctrl.radius = 0.4;
        ctrl.move_speed = 6.0;
        ctrl.jump_velocity = 6.0;
        ctrl.slope_limit = 50.0;
        ctrl.skin_offset = 0.02;
        ctrl.foot_ik_enabled = false;

        let serialized = serialize_character_controller(&ctrl);
        let deserialized = deserialize_character_controller(&serialized);
        let restored: &CharacterController = deserialized.downcast_ref().unwrap();

        assert!((restored.height - 2.0).abs() < 1e-6);
        assert!((restored.radius - 0.4).abs() < 1e-6);
        assert!((restored.move_speed - 6.0).abs() < 1e-6);
        assert!((restored.jump_velocity - 6.0).abs() < 1e-6);
        assert!((restored.slope_limit - 50.0).abs() < 1e-6);
        assert!((restored.skin_offset - 0.02).abs() < 1e-6);
        assert!(!restored.foot_ik_enabled);
    }

    #[test]
    fn character_controller_serde_defaults_on_empty() {
        let fields = BTreeMap::new();
        let deserialized = deserialize_character_controller(&fields);
        let restored: &CharacterController = deserialized.downcast_ref().unwrap();

        assert!((restored.height - 1.8).abs() < 1e-6);
        assert!((restored.move_speed - 5.0).abs() < 1e-6);
        assert!((restored.skin_offset - 0.01).abs() < 1e-6);
        assert!(restored.foot_ik_enabled);
    }
}
