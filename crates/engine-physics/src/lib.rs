#![forbid(unsafe_code)]

mod backend;
pub mod components;
mod convert;
mod debug;
pub mod destruction;
pub mod events;
pub mod gravity;
pub mod joints;
pub mod queries;
mod serde;
mod world;

pub use backend::{RapierBackend, RaycastHit};
pub use components::{BodyType, Collider, ColliderShape, PhysicsMaterial, RigidBody};
pub use convert::{from_rapier_vec, to_rapier_vec};
pub use debug::{ColliderDebugInfo, PhysicsDebugDraw};
pub use destruction::{
    apply_damage, DamageError, DamageKind, DamageRequest, Destructible, DestructibleDamageEvent,
};
pub use events::{
    CollisionEvent, CollisionEventKind, JointBreakEvent, PhysicsEvents, TriggerEvent,
    TriggerEventKind,
};
pub use gravity::{
    resolve_effective_gravity, shift_gravity_source_centers, sum_source_gravity, GravityFalloff,
    GravityMode, GravitySource, GRAVITY_SOURCE_MIN_DISTANCE,
};
pub use joints::{JointDescriptor, JointHandle, JointLimits, JointMotor, JointType, PhysicsJoint};
pub use queries::{
    OverlapHitResult, OverlapQuery, PhysicsQueryFilter, QueryBatcher, QueryResults,
    RaycastHitResult, RaycastQuery, SweepHitResult, SweepQuery,
};
pub use world::{PhysicsCommand, PhysicsWorld, RigidBodyRuntimeState};

// Re-export key types from engine-scene for convenience
pub use engine_scene::components::Transform;
pub use engine_scene::{Component, ComponentStorageDyn, Entity, SparseSet, World};

use engine_renderer::debug_draw::DebugDrawRegistry;
use engine_scene::registry::ComponentRegistry;

/// Canonical scene field map for editor and tooling creation of a
/// [`Destructible`] component.
pub fn serialize_destructible_fields(
    destructible: &Destructible,
) -> std::collections::BTreeMap<String, engine_serialize::Value> {
    serde::serialize_destructible(destructible)
}

/// Register physics extensions with Gate 9 extension surfaces.
///
/// This function should be called once during engine initialisation to
/// register physics component types, debug draw providers, and any other
/// Gate 9 extensions.
pub fn register_physics_extensions(
    component_registry: &mut ComponentRegistry,
    debug_draw_registry: Option<&mut DebugDrawRegistry>,
) {
    use crate::{ComponentStorageDyn, SparseSet};
    use engine_scene::registry::{ComponentExtension, ComponentMeta};

    // ── RigidBody ──────────────────────────────────────────────────────
    component_registry
        .register(ComponentExtension {
            meta: ComponentMeta {
                type_id: RigidBody::TYPE_ID,
                display_name: "RigidBody",
                schema_version: (0, 1, 0),
                has_editor: true,
                script_access: engine_scene::registry::ScriptAccess::ReadWrite,
            },
            storage_factory: || -> Box<dyn ComponentStorageDyn> {
                Box::new(SparseSet::<RigidBody>::new())
            },
            serialize: Some(serde::serialize_rigid_body),
            deserialize: Some(serde::deserialize_rigid_body),
        })
        .ok();

    // ── Destructible prop ────────────────────────────────────────────────
    component_registry
        .register(ComponentExtension {
            meta: ComponentMeta {
                type_id: Destructible::TYPE_ID,
                display_name: "Destructible",
                schema_version: (0, 1, 0),
                has_editor: true,
                script_access: engine_scene::registry::ScriptAccess::DedicatedApi,
            },
            storage_factory: || -> Box<dyn ComponentStorageDyn> {
                Box::new(SparseSet::<Destructible>::new())
            },
            serialize: Some(serde::serialize_destructible),
            deserialize: Some(serde::deserialize_destructible),
        })
        .ok();

    component_registry
        .register(ComponentExtension {
            meta: ComponentMeta {
                type_id: PhysicsJoint::TYPE_ID,
                display_name: "Physics Joint",
                schema_version: (0, 1, 0),
                has_editor: true,
                script_access: engine_scene::registry::ScriptAccess::DedicatedApi,
            },
            storage_factory: || -> Box<dyn ComponentStorageDyn> {
                Box::new(SparseSet::<PhysicsJoint>::new())
            },
            serialize: Some(serde::serialize_physics_joint),
            deserialize: Some(serde::deserialize_physics_joint),
        })
        .ok();

    // ── Collider ───────────────────────────────────────────────────────
    component_registry
        .register(ComponentExtension {
            meta: ComponentMeta {
                type_id: Collider::TYPE_ID,
                display_name: "Collider",
                schema_version: (0, 1, 0),
                has_editor: true,
                script_access: engine_scene::registry::ScriptAccess::ReadWrite,
            },
            storage_factory: || -> Box<dyn ComponentStorageDyn> {
                Box::new(SparseSet::<Collider>::new())
            },
            serialize: Some(serde::serialize_collider),
            deserialize: Some(serde::deserialize_collider),
        })
        .ok();

    // ── PhysicsMaterial ────────────────────────────────────────────────
    component_registry
        .register(ComponentExtension {
            meta: ComponentMeta {
                type_id: PhysicsMaterial::TYPE_ID,
                display_name: "PhysicsMaterial",
                schema_version: (0, 1, 0),
                has_editor: true,
                script_access: engine_scene::registry::ScriptAccess::ReadWrite,
            },
            storage_factory: || -> Box<dyn ComponentStorageDyn> {
                Box::new(SparseSet::<PhysicsMaterial>::new())
            },
            serialize: Some(serde::serialize_physics_material),
            deserialize: Some(serde::deserialize_physics_material),
        })
        .ok();

    // ── GravitySource ──────────────────────────────────────────────────
    component_registry
        .register(ComponentExtension {
            meta: ComponentMeta {
                type_id: crate::GravitySource::TYPE_ID,
                display_name: "Gravity Source",
                schema_version: (0, 1, 0),
                has_editor: true,
                script_access: engine_scene::registry::ScriptAccess::ReadWrite,
            },
            storage_factory: || -> Box<dyn ComponentStorageDyn> {
                Box::new(SparseSet::<crate::GravitySource>::new())
            },
            serialize: Some(serde::serialize_gravity_source),
            deserialize: Some(serde::deserialize_gravity_source),
        })
        .ok();

    // ── Debug draw provider ────────────────────────────────────────────
    if let Some(ddr) = debug_draw_registry {
        ddr.register(Box::new(PhysicsDebugDraw::new()));
    }
}

#[cfg(test)]
mod tests;
