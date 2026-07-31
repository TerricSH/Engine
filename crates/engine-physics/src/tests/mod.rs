use crate::{
    apply_damage, BodyType, Collider, ColliderDebugInfo, ColliderShape, CollisionEvent,
    CollisionEventKind, Component, DamageKind, DamageRequest, Destructible, Entity,
    JointDescriptor, JointLimits, JointMotor, JointType, PhysicsCommand, PhysicsEvents,
    PhysicsJoint, PhysicsMaterial, PhysicsWorld, RapierBackend, RigidBody, Transform, TriggerEvent,
    TriggerEventKind,
};
use engine_renderer::DebugDrawProvider;
use engine_scene::World;
use glam::Vec3;

#[test]
fn legacy_damage_kind_is_the_destruction_damage_kind() {
    assert_eq!(
        std::any::TypeId::of::<DamageKind>(),
        std::any::TypeId::of::<crate::DestructionDamageKind>()
    );
    assert_eq!(
        serde_json::to_string(&crate::DestructionDamageKind::Blast).unwrap(),
        "\"Blast\""
    );
}

fn entity(index: u32) -> Entity {
    Entity::new(index, 0)
}

fn fixed_joint(entity_a: Entity, entity_b: Entity) -> JointDescriptor {
    JointDescriptor {
        entity_a,
        entity_b,
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

// ══════════════════════════════════════════════════════════════════════════════
include!("components.rs");
include!("queries.rs");
include!("shape_filters.rs");
include!("events.rs");
include!("commands.rs");
include!("ecs_world.rs");
include!("backend.rs");
include!("joints_runtime.rs");
include!("registration_queries.rs");
include!("gravity_components.rs");
include!("gravity_resolution.rs");
include!("gravity_scene.rs");
include!("gravity_step.rs");
