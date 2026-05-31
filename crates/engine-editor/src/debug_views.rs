//! Debug draw views for the editor.
//!
//! Provides standalone functions that render editor-specific debug
//! visualisations of physics colliders, navigation meshes, and skeleton
//! bones directly into a [`DebugDrawBuffer`] by querying engine ECS
//! components.
//!
//! These functions are intended to be called from the editor's scene
//! view each frame when the corresponding debug overlay is enabled.

use glam::Vec3;

use engine_animation::components::AnimationPlayer;
use engine_nav::NavMesh;
use engine_physics::Collider;
use engine_renderer::DebugDrawBuffer;
use engine_scene::components::Transform;
use engine_scene::{Entity, World};

// ---------------------------------------------------------------------------
// Colour constants
// ---------------------------------------------------------------------------

/// Green wireframe for physics colliders.
const COLOR_PHYSICS: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
/// Green wireframe for navmesh polygon edges.
const COLOR_NAVMESH: [f32; 4] = [0.0, 0.8, 0.2, 1.0];
/// Cyan for skeleton joint spheres.
const COLOR_SKELETON_JOINT: [f32; 4] = [0.3, 0.8, 1.0, 1.0];
/// Light-blue for skeleton bone arrows.
const COLOR_SKELETON_BONE: [f32; 4] = [0.6, 0.9, 1.0, 0.6];

// ---------------------------------------------------------------------------
// draw_physics_debug
// ---------------------------------------------------------------------------

/// Draw wireframe collider shapes for all entities that have a
/// [`Collider`] component.
///
/// Iterates the ECS world, queries for [`Collider`] components, and
/// draws the corresponding shape (box, sphere, capsule) at the entity's
/// [`Transform`] position.  Entities without a `Transform` are skipped.
pub fn draw_physics_debug(buffer: &mut DebugDrawBuffer, world: &World) {
    for (entity, collider) in world.query::<Collider>() {
        let transform = match world.get::<Transform>(entity) {
            Some(t) => t,
            None => continue,
        };
        let position = transform.translation;
        let rotation = transform.rotation;

        match &collider.shape {
            engine_physics::ColliderShape::Cuboid { hx, hy, hz } => {
                buffer.box_wireframe(position, Vec3::new(*hx, *hy, *hz), COLOR_PHYSICS);
            }
            engine_physics::ColliderShape::Ball { radius } => {
                buffer.sphere_wireframe(position, *radius, COLOR_PHYSICS);
            }
            engine_physics::ColliderShape::Capsule {
                half_height,
                radius,
            } => {
                let top = position + rotation * Vec3::new(0.0, *half_height, 0.0);
                let bottom = position + rotation * Vec3::new(0.0, -*half_height, 0.0);
                buffer.sphere_wireframe(top, *radius, COLOR_PHYSICS);
                buffer.sphere_wireframe(bottom, *radius, COLOR_PHYSICS);
                buffer.line(top, bottom, COLOR_PHYSICS);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// draw_navmesh_debug
// ---------------------------------------------------------------------------

/// Draw navmesh polygon edges as wireframe lines.
///
/// Walks every polygon in the [`NavMesh`] and draws each edge as a line
/// segment, reusing the same geometry logic as
/// [`NavMeshDebugDraw`](engine_nav::debug::NavMeshDebugDraw).
pub fn draw_navmesh_debug(buffer: &mut DebugDrawBuffer, navmesh: &NavMesh) {
    for i in 0..navmesh.polygon_count() {
        let poly_idx = engine_nav::PolygonIndex(i as u32);
        let Some(indices) = navmesh.polygon_vertex_indices(poly_idx) else {
            continue;
        };

        let n = indices.len();
        if n < 2 {
            continue;
        }

        for j in 0..n {
            let a = indices[j];
            let b = indices[(j + 1) % n];

            if let (Some(va), Some(vb)) = (navmesh.vertex(a), navmesh.vertex(b)) {
                buffer.line(*va, *vb, COLOR_NAVMESH);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// draw_skeleton_debug
// ---------------------------------------------------------------------------

/// Draw skeleton bones for an entity that has an [`AnimationPlayer`]
/// component.
///
/// Renders a small sphere at each cached bone (joint) position and
/// connects consecutive bones with arrows.  If the entity does not have
/// an `AnimationPlayer` or has no cached positions, this function is a
/// no-op.
pub fn draw_skeleton_debug(buffer: &mut DebugDrawBuffer, entity: Entity, world: &World) {
    let player = match world.get::<AnimationPlayer>(entity) {
        Some(p) => p,
        None => return,
    };

    let positions = &player.cached_bone_positions;
    if positions.is_empty() {
        return;
    }

    // Draw joints as small spheres.
    for pos_3 in positions {
        let center = Vec3::from(*pos_3);
        buffer.sphere_wireframe(center, 0.04, COLOR_SKELETON_JOINT);
    }

    // Draw bones as arrows from child to parent.  Since we do not have
    // explicit parent indices from the cached positions alone, we use
    // sequential indexing as a heuristic (the skeleton is stored in
    // parent-before-child order).
    for i in 1..positions.len() {
        let from = Vec3::from(positions[i - 1]);
        let to = Vec3::from(positions[i]);
        buffer.arrow(from, to, COLOR_SKELETON_BONE);
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ── draw_physics_debug ──────────────────────────────────────────

    #[test]
    fn draw_physics_debug_empty_world_no_crash() {
        let world = engine_scene::World::new();
        let mut buf = DebugDrawBuffer::new();
        draw_physics_debug(&mut buf, &world);
        assert!(buf.is_empty());
    }

    #[test]
    fn draw_physics_debug_with_collider_no_transform_no_crash() {
        let mut world = engine_scene::World::new();
        let entity = world.create_entity();
        // Add a collider but no transform – entity should be skipped
        // without panic.
        world.add_component(
            entity,
            engine_physics::Collider {
                shape: engine_physics::ColliderShape::Ball { radius: 1.0 },
                ..engine_physics::Collider::default()
            },
        );
        let mut buf = DebugDrawBuffer::new();
        draw_physics_debug(&mut buf, &world);
        // No transform → nothing drawn
        assert!(buf.is_empty());
    }

    #[test]
    fn draw_physics_debug_box_collider() {
        let mut world = engine_scene::World::new();
        let entity = world.create_entity();
        world.add_component(
            entity,
            engine_physics::Collider {
                shape: engine_physics::ColliderShape::Cuboid {
                    hx: 0.5,
                    hy: 0.5,
                    hz: 0.5,
                },
                ..engine_physics::Collider::default()
            },
        );
        world.add_component(entity, Transform::default());
        let mut buf = DebugDrawBuffer::new();
        draw_physics_debug(&mut buf, &world);
        // Should produce one box shape
        assert_eq!(buf.shapes.len(), 1);
    }

    #[test]
    fn draw_physics_debug_ball_collider() {
        let mut world = engine_scene::World::new();
        let entity = world.create_entity();
        world.add_component(
            entity,
            engine_physics::Collider {
                shape: engine_physics::ColliderShape::Ball { radius: 0.5 },
                ..engine_physics::Collider::default()
            },
        );
        world.add_component(entity, Transform::default());
        let mut buf = DebugDrawBuffer::new();
        draw_physics_debug(&mut buf, &world);
        assert_eq!(buf.shapes.len(), 1);
    }

    #[test]
    fn draw_physics_debug_capsule_collider() {
        let mut world = engine_scene::World::new();
        let entity = world.create_entity();
        world.add_component(
            entity,
            engine_physics::Collider {
                shape: engine_physics::ColliderShape::Capsule {
                    half_height: 1.0,
                    radius: 0.3,
                },
                ..engine_physics::Collider::default()
            },
        );
        world.add_component(entity, Transform::default());
        let mut buf = DebugDrawBuffer::new();
        draw_physics_debug(&mut buf, &world);
        // Capsule produces 2 spheres + 1 line
        assert_eq!(buf.shapes.len(), 2);
        assert_eq!(buf.lines.len(), 1);
    }

    // ── draw_navmesh_debug ──────────────────────────────────────────

    #[test]
    fn draw_navmesh_debug_empty_no_crash() {
        let navmesh = NavMesh::new();
        let mut buf = DebugDrawBuffer::new();
        draw_navmesh_debug(&mut buf, &navmesh);
        assert!(buf.is_empty());
    }

    #[test]
    fn draw_navmesh_debug_single_triangle() {
        let mut navmesh = NavMesh::new();
        let a = navmesh.add_vertex(Vec3::new(0.0, 0.0, 0.0));
        let b = navmesh.add_vertex(Vec3::new(1.0, 0.0, 0.0));
        let c = navmesh.add_vertex(Vec3::new(0.0, 0.0, 1.0));
        navmesh.add_polygon(&[a, b, c], 1.0);

        let mut buf = DebugDrawBuffer::new();
        draw_navmesh_debug(&mut buf, &navmesh);
        // Triangle has 3 edges → 3 lines
        assert_eq!(buf.lines.len(), 3);
    }

    #[test]
    fn draw_navmesh_debug_multiple_polygons() {
        let mut navmesh = NavMesh::new();
        let a = navmesh.add_vertex(Vec3::new(0.0, 0.0, 0.0));
        let b = navmesh.add_vertex(Vec3::new(1.0, 0.0, 0.0));
        let c = navmesh.add_vertex(Vec3::new(0.0, 0.0, 1.0));
        let d = navmesh.add_vertex(Vec3::new(1.0, 0.0, 1.0));
        navmesh.add_polygon(&[a, b, d, c], 1.0); // quad = 4 edges
        navmesh.add_polygon(&[a, b, c], 1.0); // tri = 3 edges

        let mut buf = DebugDrawBuffer::new();
        draw_navmesh_debug(&mut buf, &navmesh);
        assert_eq!(buf.lines.len(), 7); // 4 + 3
    }

    // ── draw_skeleton_debug ─────────────────────────────────────────

    #[test]
    fn draw_skeleton_debug_no_player_no_crash() {
        let mut world = engine_scene::World::new();
        let entity = world.create_entity();
        // Entity has no AnimationPlayer – no-op
        let mut buf = DebugDrawBuffer::new();
        draw_skeleton_debug(&mut buf, entity, &world);
        assert!(buf.is_empty());
    }

    #[test]
    fn draw_skeleton_debug_empty_cached_positions() {
        let mut world = engine_scene::World::new();
        let entity = world.create_entity();
        world.add_component(
            entity,
            AnimationPlayer {
                cached_bone_positions: vec![],
                ..AnimationPlayer::new()
            },
        );
        let mut buf = DebugDrawBuffer::new();
        draw_skeleton_debug(&mut buf, entity, &world);
        assert!(buf.is_empty());
    }

    #[test]
    fn draw_skeleton_debug_with_positions() {
        let mut world = engine_scene::World::new();
        let entity = world.create_entity();
        world.add_component(
            entity,
            AnimationPlayer {
                cached_bone_positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]],
                ..AnimationPlayer::new()
            },
        );
        let mut buf = DebugDrawBuffer::new();
        draw_skeleton_debug(&mut buf, entity, &world);
        // 3 joint spheres + 2 bone arrows = 5 shapes total
        assert_eq!(buf.shapes.len(), 5);
    }

    #[test]
    fn draw_skeleton_debug_stale_entity_no_crash() {
        let mut world = engine_scene::World::new();
        let entity = world.create_entity();
        world.destroy_entity(entity);
        let mut buf = DebugDrawBuffer::new();
        draw_skeleton_debug(&mut buf, entity, &world);
        assert!(buf.is_empty());
    }
}
