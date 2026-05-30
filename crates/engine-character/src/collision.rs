//! Collision detection and resolution for the kinematic character controller.
//!
//! Provides shape‑based ground detection and per‑axis collision resolution
//! using the engine‑physics world's shape‑proximity and ray‑cast APIs.

use engine_physics::{ColliderShape, PhysicsWorld};
use glam::Vec3;

use crate::controller::CharacterController;

// ── Constants ────────────────────────────────────────────────────────────────

/// Small epsilon for ground-check ray offsets and tolerance comparisons.
const EPSILON: f32 = 0.01;

/// Additional tolerance added to `step_height` when casting the ground ray.
const GROUND_RAY_EXTRA: f32 = 0.05;

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Build a capsule [`ColliderShape`] from the controller parameters.
fn capsule_shape(controller: &CharacterController) -> ColliderShape {
    ColliderShape::Capsule {
        half_height: (controller.height * 0.5 - controller.radius).max(0.01),
        radius: controller.radius,
    }
}

/// Returns `true` if the capsule at `position` overlaps any physics colliders.
fn capsule_overlaps(
    position: Vec3,
    controller: &CharacterController,
    physics: &PhysicsWorld,
) -> bool {
    let shape = capsule_shape(controller);
    !physics.query_proximity(&shape, position).is_empty()
}

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

/// Resolve collisions using capsule‑volume validation and per‑axis ray casts.
///
/// 1.  Try moving the full displacement and check the capsule for overlap.
/// 2.  If the capsule is clear at the target position, accept the full move.
/// 3.  If blocked, fall back to per‑axis ray casting (three heights per axis)
///     with step‑up handling.
///
/// Horizontal axes (X, Z) are resolved first so the character slides along
/// walls. The vertical axis (Y) is resolved with a simple ceiling/floor ray.
pub fn resolve_collision(
    position: Vec3,
    velocity: Vec3,
    controller: &CharacterController,
    physics: &PhysicsWorld,
    dt: f32,
) -> (Vec3, Vec3) {
    let displacement = velocity * dt;
    let target = position + displacement;

    // ── 1. Try full movement with capsule validation ──────────────────────
    if !capsule_overlaps(target, controller, physics) {
        return (target, velocity);
    }

    // ── 2. Blocked — per‑axis ray-cast resolution ────────────────────────
    let mut final_pos = position;
    let mut final_vel = velocity;
    let offset = controller.radius;
    let bottom_y = position.y - controller.height * 0.5;

    for &axis in &[0usize, 2, 1] {
        let disp = final_vel[axis];
        if disp.abs() < 0.0001 {
            continue;
        }
        let sign = disp.signum();
        let dist = disp.abs();

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
                final_pos.y += disp;
            }
            continue;
        }

        // ── Horizontal: multi-ray capsule approximation ──────────────────
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

        let base_y = bottom_y + offset * 0.5 + EPSILON;
        let step_y = bottom_y + controller.step_height;
        let mid_y = final_pos.y;

        let base_avail = cast_at_level(final_pos, base_y).unwrap_or(dist);
        let step_avail = cast_at_level(final_pos, step_y).unwrap_or(dist);
        let mid_avail = cast_at_level(final_pos, mid_y).unwrap_or(dist);

        let available = base_avail.min(step_avail).min(mid_avail);

        // ── Step-up ──────────────────────────────────────────────────────
        if available < dist
            && controller.step_height > 0.0
            && base_avail < dist * 0.5
        {
            let lifted_y = final_pos.y + controller.step_height;
            let lifted_base = cast_at_level(final_pos, bottom_y + controller.step_height)
                .unwrap_or(dist);
            let lifted_mid = cast_at_level(final_pos, lifted_y).unwrap_or(dist);

            if lifted_base >= dist && lifted_mid >= dist {
                final_pos.y += controller.step_height;
                final_pos[axis] += disp;
                final_vel.y = 0.0;
                final_vel[axis] = 0.0;
                continue; // step-up succeeded, skip normal movement
            }
        }

        // ── Apply movement ───────────────────────────────────────────────
        let travel = available.min(dist);
        final_pos[axis] += sign * travel;
        if travel < dist {
            final_vel[axis] = 0.0;
        }
    }

    (final_pos, final_vel)
}
