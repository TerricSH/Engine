#![forbid(unsafe_code)]

//! Kinematic character controller for the game engine.
//!
//! Provides a simple capsule-based character controller that moves through
//! the world using ray-cast collision detection rather than a full physics
//! rigid body. Performs ground detection, jumping, falling, and horizontal
//! movement with separate acceleration/deceleration profiles for ground
//! and air.
//!
//! # Design
//!
//! - **Kinematic only** — no physics rigid body is created or managed.
//! - **Ray-based collision** — uses the engine-physics world for ray-cast
//!   queries only.
//! - **State machine** — [`CharacterState`] tracks vertical state (grounded,
//!   jumping, falling, free) with the **State** design pattern.
//! - **Configurable parameters** — speed, acceleration, jump velocity,
//!   gravity scale, slope limit, etc. are exposed as public fields.
//!
//! Per FD-031 the coordinate system is right-handed, +Y up, −Z forward,
//! with metres as the unit of distance and gravity = (0, −9.81, 0).
//!
//! # Module layout
//!
//! | Module        | Contents                                           |
//! |---------------|----------------------------------------------------|
//! | `controller`  | [`CharacterController`], [`CharacterState`], [`CharacterError`] |
//! | `movement`    | [`process_movement`], [`CharacterMovement`], [`CharacterOutput`] |
//! | `collision`   | [`ground_check`], [`resolve_collision`]             |

mod collision;
mod controller;
mod movement;

pub use collision::{ground_check, resolve_collision};
pub use controller::{CharacterController, CharacterError, CharacterState};
pub use movement::{process_movement, CharacterMovement, CharacterOutput};

#[cfg(test)]
mod tests {
    use super::*;

    // ── CharacterState tests ─────────────────────────────────────────────

    #[test]
    fn character_state_can_transition_grounded_to_jumping() {
        assert!(CharacterState::Grounded.can_transition_to(CharacterState::Jumping));
    }

    #[test]
    fn character_state_can_transition_grounded_to_falling() {
        assert!(CharacterState::Grounded.can_transition_to(CharacterState::Falling));
    }

    #[test]
    fn character_state_can_transition_jumping_to_falling() {
        assert!(CharacterState::Jumping.can_transition_to(CharacterState::Falling));
    }

    #[test]
    fn character_state_cannot_transition_jumping_directly_to_grounded() {
        assert!(!CharacterState::Jumping.can_transition_to(CharacterState::Grounded));
    }

    #[test]
    fn character_state_falling_to_grounded() {
        assert!(CharacterState::Falling.can_transition_to(CharacterState::Grounded));
    }

    #[test]
    fn character_state_grounded_to_free() {
        assert!(CharacterState::Grounded.can_transition_to(CharacterState::Free));
    }

    #[test]
    fn character_state_free_to_any() {
        assert!(CharacterState::Free.can_transition_to(CharacterState::Grounded));
        assert!(CharacterState::Free.can_transition_to(CharacterState::Jumping));
        assert!(CharacterState::Free.can_transition_to(CharacterState::Falling));
    }

    #[test]
    fn character_state_identity_transition() {
        assert!(CharacterState::Grounded.can_transition_to(CharacterState::Grounded));
        assert!(CharacterState::Jumping.can_transition_to(CharacterState::Jumping));
        assert!(CharacterState::Falling.can_transition_to(CharacterState::Falling));
        assert!(CharacterState::Free.can_transition_to(CharacterState::Free));
    }

    #[test]
    fn character_state_illegal_transitions() {
        assert!(!CharacterState::Jumping.can_transition_to(CharacterState::Grounded));
        assert!(!CharacterState::Falling.can_transition_to(CharacterState::Jumping));
    }

    #[test]
    fn character_state_debug() {
        assert_eq!(format!("{:?}", CharacterState::Grounded), "Grounded");
        assert_eq!(format!("{:?}", CharacterState::Jumping), "Jumping");
        assert_eq!(format!("{:?}", CharacterState::Falling), "Falling");
        assert_eq!(format!("{:?}", CharacterState::Free), "Free");
    }

    // ── CharacterController tests ────────────────────────────────────────

    #[test]
    fn character_controller_defaults() {
        let ctrl = CharacterController::new();
        assert_eq!(ctrl.height, 1.8);
        assert_eq!(ctrl.radius, 0.3);
        assert_eq!(ctrl.move_speed, 5.0);
        assert_eq!(ctrl.acceleration, 20.0);
        assert_eq!(ctrl.deceleration, 15.0);
        assert_eq!(ctrl.air_acceleration, 5.0);
        assert_eq!(ctrl.air_deceleration, 2.0);
        assert_eq!(ctrl.jump_velocity, 5.0);
        assert_eq!(ctrl.gravity_scale, 1.0);
        assert_eq!(ctrl.max_fall_speed, 20.0);
        assert_eq!(ctrl.step_height, 0.3);
        assert_eq!(ctrl.slope_limit, 45.0);
    }

    #[test]
    fn character_controller_default_state() {
        let ctrl = CharacterController::new();
        assert_eq!(ctrl.state(), CharacterState::Falling);
    }

    #[test]
    fn character_controller_default_position() {
        let ctrl = CharacterController::new();
        assert_eq!(ctrl.position(), glam::Vec3::ZERO);
    }

    #[test]
    fn character_controller_default_velocity() {
        let ctrl = CharacterController::new();
        assert_eq!(ctrl.velocity(), glam::Vec3::ZERO);
    }

    #[test]
    fn character_controller_is_grounded_initially_false() {
        let ctrl = CharacterController::new();
        assert!(!ctrl.is_grounded());
    }

    #[test]
    fn character_controller_set_position() {
        let mut ctrl = CharacterController::new();
        ctrl.set_position(glam::Vec3::new(10.0, 5.0, -3.0));
        assert_eq!(ctrl.position(), glam::Vec3::new(10.0, 5.0, -3.0));
    }

    #[test]
    fn character_controller_transition_state_valid() {
        let mut ctrl = CharacterController::new();
        // Starts at Falling
        assert!(ctrl.transition_state(CharacterState::Grounded).is_ok());
        assert_eq!(ctrl.state(), CharacterState::Grounded);
    }

    #[test]
    fn character_controller_transition_state_invalid() {
        let mut ctrl = CharacterController::new();
        // Starts at Falling — cannot go directly to Jumping
        assert!(ctrl.transition_state(CharacterState::Jumping).is_err());
    }

    #[test]
    fn character_controller_default_impl() {
        let ctrl = CharacterController::default();
        assert_eq!(ctrl.height, 1.8);
    }

    // ── CharacterMovement tests ──────────────────────────────────────────

    #[test]
    fn character_movement_creation() {
        let mov = CharacterMovement {
            direction: glam::Vec3::X,
            wish_jump: false,
            delta_time: 1.0 / 60.0,
        };
        assert_eq!(mov.direction, glam::Vec3::X);
        assert!(!mov.wish_jump);
        assert!((mov.delta_time - 1.0 / 60.0).abs() < 1e-6);
    }

    #[test]
    fn character_movement_debug() {
        let mov = CharacterMovement {
            direction: glam::Vec3::Z,
            wish_jump: true,
            delta_time: 0.016,
        };
        let debug = format!("{:?}", mov);
        assert!(debug.contains("CharacterMovement"));
    }

    // ── CharacterOutput tests ────────────────────────────────────────────

    #[test]
    fn character_output_construction() {
        let output = CharacterOutput {
            new_position: glam::Vec3::new(1.0, 2.0, 3.0),
            new_velocity: glam::Vec3::new(0.0, -5.0, 0.0),
            state: CharacterState::Falling,
            grounded: false,
            moved: true,
        };
        assert_eq!(output.new_position, glam::Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(output.new_velocity, glam::Vec3::new(0.0, -5.0, 0.0));
        assert_eq!(output.state, CharacterState::Falling);
        assert!(!output.grounded);
        assert!(output.moved);
    }

    #[test]
    fn character_output_debug() {
        let output = CharacterOutput {
            new_position: glam::Vec3::ZERO,
            new_velocity: glam::Vec3::ZERO,
            state: CharacterState::Grounded,
            grounded: true,
            moved: false,
        };
        let debug = format!("{:?}", output);
        assert!(debug.contains("CharacterOutput"));
    }

    // ── CharacterError tests ─────────────────────────────────────────────

    #[test]
    fn character_error_invalid_input_display() {
        let err = CharacterError::InvalidInput("speed is negative".to_string());
        assert_eq!(err.to_string(), "invalid input: speed is negative");
    }

    #[test]
    fn character_error_physics_world_missing_display() {
        let err = CharacterError::PhysicsWorldMissing;
        assert_eq!(
            err.to_string(),
            "physics world is not available for collision queries"
        );
    }

    #[test]
    fn character_error_debug() {
        let err = CharacterError::InvalidInput("bad".to_string());
        let debug = format!("{:?}", err);
        assert!(debug.contains("InvalidInput"));
    }
}
