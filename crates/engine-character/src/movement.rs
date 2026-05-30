//! Per-frame movement processing for the kinematic character controller.
//!
//! Applies gravity, horizontal acceleration/deceleration, ground detection,
//! jumping, and vertical state transitions. Uses the collision module for
//! ray-based queries.

use engine_physics::PhysicsWorld;
use glam::Vec3;
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::collision::{ground_check, resolve_collision};
use crate::controller::{CharacterController, CharacterState};

// ── Constants ────────────────────────────────────────────────────────────────

/// Gravity vector per FD-031: (0.0, −9.81, 0.0) in right-handed +Y up.
const GRAVITY: Vec3 = Vec3::new(0.0, -9.81, 0.0);

// ── Movement input / output ─────────────────────────────────────────────────

/// Per-frame input to drive the character controller.
///
/// Populated each frame by gameplay code (e.g., from player input or AI)
/// and passed to [`process_movement`].
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CharacterMovement {
    /// Horizontal movement direction.
    ///
    /// Should be normalised before submission. The Y component is ignored
    /// during horizontal acceleration — only (X, Z) contributes to movement.
    pub direction: Vec3,

    /// Whether the character requested a jump this frame.
    pub wish_jump: bool,

    /// Frame delta time in seconds.
    pub delta_time: f32,
}

/// Result of processing one frame of character movement.
///
/// Returned by [`process_movement`] and contains the updated position,
/// velocity, state, and grounded flag.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CharacterOutput {
    /// New world-space position (center of the capsule) after movement.
    pub new_position: Vec3,

    /// New velocity after movement and collision resolution.
    pub new_velocity: Vec3,

    /// Character state after movement.
    pub state: CharacterState,

    /// Whether the character is on the ground at the end of the frame.
    pub grounded: bool,

    /// Whether the character moved this frame (position changed).
    pub moved: bool,
}

// ── Core movement ────────────────────────────────────────────────────────────

/// Process one frame of character movement.
///
/// Applies gravity, horizontal acceleration/deceleration (with different
/// rates for ground vs. air), ground detection, jumping, and vertical state
/// transitions. Returns the updated position, velocity, state, and grounded
/// flag packed into a [`CharacterOutput`].
///
/// # State machine transitions
///
/// The function implements the following state transitions each frame:
///
/// ```text
///                          ┌──────────┐
///                      ┌──│ Grounded │<──────┐
///                      │   └────┬─────┘      │
///                      │        │            │
///                 jump │   walk off     land │
///                      │   edge             │
///                      │   ┌────▼─────┐     │
///                      │   │ Jumping  │     │
///                      │   └────┬─────┘     │
///                      │        │           │
///                      │   apex│            │
///                      │   ┌────▼─────┐     │
///                      └───│ Falling  │─────┘
///                          └──────────┘
/// ```
///
/// | Step | Condition                          | Transition               |
/// |------|------------------------------------|--------------------------|
/// | 6    | Ground detected                   | any → `Grounded`         |
/// | 7    | `wish_jump` && grounded           | `Grounded` → `Jumping`   |
/// | 8    | Not grounded, was `Grounded`      | `Grounded` → `Falling`   |
/// | 8    | Not grounded, was `Jumping`, v≤0  | `Jumping` → `Falling`    |
/// | —    | All other cases                   | State unchanged          |
///
/// # Physics
///
/// When `physics` is `Some(...)`, collision resolution and ground detection
/// are performed via the physics world's ray-cast API. When `None`, a
/// simplified kinematic pass runs without any collision queries.
pub fn process_movement(
    controller: &CharacterController,
    input: &CharacterMovement,
    physics: Option<&PhysicsWorld>,
) -> CharacterOutput {
    let mut velocity = controller.velocity();
    let mut position = controller.position();
    let mut state = controller.state();

    // ── 1. Apply gravity ────────────────────────────────────────────────
    // Per FD-031 gravity is (0, −9.81, 0).
    //
    // When the character was grounded last frame we first reset any
    // accumulated downward velocity so the character does not sink into
    // the ground between frames. Gravity is then reapplied so that the
    // character immediately starts falling when it walks off an edge.
    if state == CharacterState::Grounded && velocity.y <= 0.0 {
        velocity.y = 0.0;
    }
    velocity.y += GRAVITY.y * controller.gravity_scale * input.delta_time;

    // ── 2. Horizontal acceleration / deceleration ───────────────────────
    let wish_dir = Vec3::new(input.direction.x, 0.0, input.direction.z);
    let on_ground = state == CharacterState::Grounded;

    if wish_dir.length_squared() > 0.01_f32 {
        // Accelerate towards the input direction.
        let accel = if on_ground {
            controller.acceleration
        } else {
            controller.air_acceleration
        };

        let wish_dir_n = wish_dir.normalize();
        let current_h = Vec3::new(velocity.x, 0.0, velocity.z);
        let proj_speed = current_h.dot(wish_dir_n);

        let add_speed = (controller.move_speed - proj_speed).clamp(0.0, accel * input.delta_time);

        velocity += wish_dir_n * add_speed;
    } else {
        // Decelerate to rest.
        let decel = if on_ground {
            controller.deceleration
        } else {
            controller.air_deceleration
        };

        let current_h = Vec3::new(velocity.x, 0.0, velocity.z);
        let speed = current_h.length();
        if speed > 0.0 {
            let reduction = (decel * input.delta_time).min(speed);
            let factor = 1.0 - reduction / speed;
            velocity.x *= factor;
            velocity.z *= factor;
        }
    }

    // ── 3. Clamp horizontal speed ───────────────────────────────────────
    let h_vel = Vec3::new(velocity.x, 0.0, velocity.z);
    let h_speed = h_vel.length();
    if h_speed > controller.move_speed {
        let scale = controller.move_speed / h_speed;
        velocity.x *= scale;
        velocity.z *= scale;
    }

    // ── 4. Clamp fall speed ─────────────────────────────────────────────
    if velocity.y < -controller.max_fall_speed {
        velocity.y = -controller.max_fall_speed;
    }

    // ── 5. Move & resolve collisions ────────────────────────────────────
    if let Some(pw) = physics {
        let (resolved_pos, resolved_vel) = resolve_collision(position, velocity, controller, pw);
        position = resolved_pos;
        velocity = resolved_vel;
    } else {
        position += velocity * input.delta_time;
    }

    // ── 6. Ground detection ─────────────────────────────────────────────
    // Transition: any state → Grounded (when ground is found)
    let mut grounded = false;
    if let Some(pw) = physics {
        if let Some(ground_dist) = ground_check(position, controller, pw) {
            grounded = true;
            // Snap the character to the ground surface.
            position.y -= ground_dist;

            // Prevent downward velocity accumulation while on the ground.
            if velocity.y < 0.0 {
                velocity.y = 0.0;
            }

            if state != CharacterState::Grounded {
                debug!(
                    old_state = ?state,
                    new_state = ?CharacterState::Grounded,
                    "character landed"
                );
            }
            state = CharacterState::Grounded;
        }
    }

    // ── 7. Jump ─────────────────────────────────────────────────────────
    // Transition: Grounded → Jumping (when wish_jump && grounded)
    if input.wish_jump && grounded {
        velocity.y = controller.jump_velocity;
        state = CharacterState::Jumping;
        grounded = false;
        debug!(
            old_state = ?CharacterState::Grounded,
            new_state = ?CharacterState::Jumping,
            "character jumped"
        );
    }

    // ── 8. Air state transitions ────────────────────────────────────────
    // Transition: Grounded → Falling (walked off edge)
    // Transition: Jumping  → Falling (reached apex, v.y ≤ 0)
    if !grounded && state != CharacterState::Free {
        match state {
            CharacterState::Grounded => {
                // Left the ground without jumping (walked off an edge).
                state = CharacterState::Falling;
                debug!(
                    old_state = ?CharacterState::Grounded,
                    new_state = ?CharacterState::Falling,
                    "character walked off edge"
                );
            }
            CharacterState::Jumping if velocity.y <= 0.0 => {
                state = CharacterState::Falling;
                debug!(
                    old_state = ?CharacterState::Jumping,
                    new_state = ?CharacterState::Falling,
                    "character reached jump apex"
                );
            }
            _ => {}
        }
    }

    let moved = (position - controller.position()).length_squared() > 0.0;

    CharacterOutput {
        new_position: position,
        new_velocity: velocity,
        state,
        grounded,
        moved,
    }
}
