//! Collision detection and resolution for the kinematic character controller.
//!
//! Provides ray-based ground detection and per-axis collision resolution
//! using the engine-physics world.

use engine_physics::PhysicsWorld;
use glam::Vec3;

use crate::controller::CharacterController;

// ── Constants ────────────────────────────────────────────────────────────────

/// Small epsilon for ground-check ray offsets and tolerance comparisons.
const EPSILON: f32 = 0.01;

/// Additional tolerance added to `step_height` when casting the ground ray.
const GROUND_RAY_EXTRA: f32 = 0.05;

// ── Ground detection ─────────────────────────────────────────────────────────

/// Cast a ray downward from the character's capsule base to find the ground.
///
/// Returns `Some(distance)` when ground is within `step_height + ε` and the
/// surface slope is within [`CharacterController::slope_limit`]. Returns
/// `None` when no suitable ground is found.
///
/// The ray origin is placed just below the capsule's bottom to avoid
/// self-intersection with the character's own collider.
pub fn ground_check(
    position: Vec3,
    controller: &CharacterController,
    physics: &PhysicsWorld,
) -> Option<f32> {
    // Bottom of the capsule in world-space.
    let bottom_y = position.y - controller.height * 0.5;
    let ray_origin = Vec3::new(position.x, bottom_y + EPSILON, position.z);
    let max_distance = controller.step_height + GROUND_RAY_EXTRA;

    let hit = physics.cast_ray(ray_origin, Vec3::NEG_Y, max_distance)?;

    // Reject surfaces steeper than the slope limit.
    let slope_deg = hit.normal.angle_between(Vec3::Y).to_degrees();
    if slope_deg > controller.slope_limit {
        return None;
    }

    Some(hit.distance)
}

// ── Collision resolution ─────────────────────────────────────────────────────

/// Resolve collisions using per-axis ray-casts.
///
/// Moves the character along each axis independently, casting a ray in the
/// movement direction. If a collision is detected before the full distance,
/// the character stops at the collision point and the velocity component
/// for that axis is zeroed.
///
/// Horizontal axes (X, Z) are resolved first so the character slides along
/// walls. The vertical axis (Y) is resolved last so ceiling/floor hits
/// override the horizontal slide if needed.
pub fn resolve_collision(
    position: Vec3,
    velocity: Vec3,
    controller: &CharacterController,
    physics: &PhysicsWorld,
) -> (Vec3, Vec3) {
    let mut final_pos = position;
    let mut final_vel = velocity;

    // Offset used to push the ray origin ahead of the capsule surface to
    // avoid self-intersection.
    let offset = controller.radius * 0.5;

    // Resolve horizontal first (X, Z) then vertical (Y).
    for &axis in &[0usize, 2, 1] {
        let displacement = final_vel[axis];
        if displacement.abs() < 0.0001 {
            continue;
        }

        let sign = displacement.signum();
        let dist = displacement.abs();

        let mut ray_dir = Vec3::ZERO;
        ray_dir[axis] = sign;

        let ray_origin = final_pos + ray_dir * offset;

        if let Some(hit) = physics.cast_ray(ray_origin, ray_dir, dist + offset) {
            let available = (hit.distance - offset).max(0.0);
            final_pos[axis] += sign * available;

            // Zero velocity in this axis when fully blocked.
            if available < dist {
                final_vel[axis] = 0.0;
            }
        } else {
            final_pos[axis] += displacement;
        }
    }

    (final_pos, final_vel)
}
