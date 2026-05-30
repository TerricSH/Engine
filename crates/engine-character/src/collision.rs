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

/// Resolve collisions using capsule-shaped sweep queries.
///
/// Casts multiple rays at the capsule's surface levels — base, step‑height
/// and centre — along each movement axis to approximate a capsule sweep.
/// The minimum clear distance across all rays determines how far the
/// character can travel before hitting an obstacle.
///
/// When horizontal movement is blocked at the capsule base but clear at the
/// configured [`step_height`](CharacterController::step_height), the character
/// is lifted onto the obstacle (step‑up behaviour).
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
    // avoid self-intersection.  Full radius because we cast from the
    // capsule rim.
    let offset = controller.radius;
    let bottom_y = position.y - controller.height * 0.5;

    // Resolve horizontal first (X, Z) then vertical (Y).
    for &axis in &[0usize, 2, 1] {
        let displacement = final_vel[axis];
        if displacement.abs() < 0.0001 {
            continue;
        }

        let sign = displacement.signum();
        let dist = displacement.abs();

        if axis == 1 {
            // ── Vertical: single ray at centre ───────────────────────────
            let mut ray_dir = Vec3::ZERO;
            ray_dir[1] = sign;
            let ray_origin = final_pos + ray_dir * offset;
            if let Some(hit) = physics.cast_ray(ray_origin, ray_dir, dist + offset) {
                let available = (hit.distance - offset).max(0.0);
                final_pos.y += sign * available;
                if available < dist {
                    final_vel.y = 0.0;
                }
            } else {
                final_pos.y += displacement;
            }
            continue;
        }

        // ── Horizontal: multi‑ray capsule approximation ──────────────────
        // Cast rays at three vertical levels of the capsule volume in the
        // movement direction and take the minimum clear distance.
        let base_y = bottom_y + offset * 0.5 + EPSILON;
        let step_y = bottom_y + controller.step_height;
        let mid_y = final_pos.y;

        // Small helper: cast a ray in the horizontal axis direction.
        let cast_at_level = |pos: Vec3, y: f32| -> Option<f32> {
            let mut origin = pos;
            origin.y = y;
            origin[axis] += offset * sign;
            let mut dir = Vec3::ZERO;
            dir[axis] = sign;
            physics
                .cast_ray(origin, dir, dist + offset)
                .map(|hit| (hit.distance - offset).max(0.0))
        };

        let base_avail = cast_at_level(final_pos, base_y).unwrap_or(dist);
        let step_avail = cast_at_level(final_pos, step_y).unwrap_or(dist);
        let mid_avail = cast_at_level(final_pos, mid_y).unwrap_or(dist);

        let available = base_avail.min(step_avail).min(mid_avail);

        // ── Step‑up logic ────────────────────────────────────────────────
        // When the base is blocked but the path is clear when lifted to
        // step_height, perform a step‑up.
        let stepped_up = if available < dist
            && controller.step_height > 0.0
            && base_avail < dist * 0.5
        {
            // Re‑evaluate from the lifted position.
            let lifted_y = final_pos.y + controller.step_height;
            let lifted_base_avail =
                cast_at_level(final_pos, bottom_y + controller.step_height).unwrap_or(dist);
            let lifted_mid_avail = cast_at_level(final_pos, lifted_y).unwrap_or(dist);

            if lifted_base_avail >= dist && lifted_mid_avail >= dist {
                // Clear path when lifted — execute step‑up.
                final_pos.y += controller.step_height;
                final_pos[axis] += displacement;
                final_vel.y = 0.0;
                final_vel[axis] = 0.0;
                true
            } else {
                false
            }
        } else {
            false
        };

        // ── Apply movement ───────────────────────────────────────────────
        if !stepped_up {
            let travel = available.min(dist);
            final_pos[axis] += sign * travel;
            if travel < dist {
                final_vel[axis] = 0.0;
            }
        }
    }

    (final_pos, final_vel)
}
