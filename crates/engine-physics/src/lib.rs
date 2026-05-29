#![forbid(unsafe_code)]

mod backend;
pub mod components;
mod convert;
mod debug;
pub mod events;
pub mod queries;
mod world;

pub use backend::{RapierBackend, RaycastHit};
pub use components::{BodyType, Collider, ColliderShape, PhysicsMaterial, RigidBody};
pub use convert::{from_rapier_vec, to_rapier_vec};
pub use debug::{ColliderDebugInfo, PhysicsDebugDraw};
pub use events::{CollisionEvent, CollisionEventKind, PhysicsEvents};
pub use queries::{OverlapQuery, QueryResults, RaycastQuery, SweepQuery};
pub use world::{PhysicsCommand, PhysicsWorld};

// Re-export key types from engine-scene for convenience
pub use engine_scene::{Component, ComponentStorageDyn, Entity, SparseSet, World};
pub use engine_scene::components::Transform;

use engine_renderer::debug_draw::DebugDrawRegistry;
use engine_scene::registry::ComponentRegistry;

/// Register physics extensions with Gate 9 extension surfaces.
///
/// This function should be called once during engine initialisation to
/// register physics component types, debug draw providers, and any other
/// Gate 9 extensions.
pub fn register_physics_extensions(
    component_registry: &mut ComponentRegistry,
    debug_draw_registry: Option<&mut DebugDrawRegistry>,
    _editor_registry: Option<&mut ()>,
    _script_registry: Option<&mut ()>,
) {
    use crate::{SparseSet, ComponentStorageDyn};
    use engine_scene::registry::{ComponentExtension, ComponentMeta};

    // ── RigidBody ──────────────────────────────────────────────────────
    component_registry
        .register(ComponentExtension {
            meta: ComponentMeta {
                type_id: RigidBody::TYPE_ID,
                display_name: "RigidBody",
                schema_version: (0, 1, 0),
                has_editor: true,
                has_script_binding: false,
            },
            storage_factory: || -> Box<dyn ComponentStorageDyn> {
                Box::new(SparseSet::<RigidBody>::new())
            },
            serialize: None,
            deserialize: None,
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
                has_script_binding: false,
            },
            storage_factory: || -> Box<dyn ComponentStorageDyn> {
                Box::new(SparseSet::<Collider>::new())
            },
            serialize: None,
            deserialize: None,
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
                has_script_binding: false,
            },
            storage_factory: || -> Box<dyn ComponentStorageDyn> {
                Box::new(SparseSet::<PhysicsMaterial>::new())
            },
            serialize: None,
            deserialize: None,
        })
        .ok();

    // ── Debug draw provider ────────────────────────────────────────────
    if let Some(ddr) = debug_draw_registry {
        ddr.register(Box::new(PhysicsDebugDraw::new()));
    }
}

#[cfg(test)]
mod tests;
