//! C# FFI bindings for the character controller.
//!
//! Exposes movement commands and state queries so C# gameplay scripts
//! can control characters without accessing engine internals.
//!
//! # Safety
//!
//! All FFI functions accept raw pointers and must be called with valid
//! pointers. Null pointers are checked and return safe defaults.
//!
//! This module is excepted from `#![deny(unsafe_code)]` because FFI
//! bridging inherently requires unsafe pointer dereferencing.
//! Every `unsafe` block has a `// SAFETY:` comment explaining its invariants.

#![allow(unsafe_code)]

use glam::Vec3;

use crate::controller::{CharacterController, CharacterState};

/// Grounded state value returned to C#.
const STATE_GROUNDED: i32 = 0;
/// Jumping state value returned to C#.
const STATE_JUMPING: i32 = 1;
/// Falling state value returned to C#.
const STATE_FALLING: i32 = 2;
/// Free state value returned to C#.
const STATE_FREE: i32 = 3;

/// Move the character in the given direction at the given speed.
///
/// Updates the controller's position, velocity, and state from the movement
/// computation. Returns `true` if the character moved this call.
///
/// # Safety
///
/// * `controller` must be a valid pointer to a `CharacterController`, or null.
/// * `physics`   must be a valid pointer to a `PhysicsWorld`, or null.
#[no_mangle]
pub extern "C" fn character_move(
    controller: *mut CharacterController,
    dir_x: f32,
    dir_z: f32,
    speed: f32,
    dt: f32,
    physics: *mut engine_physics::PhysicsWorld,
) -> bool {
    if controller.is_null() {
        return false;
    }

    // SAFETY: Null-checked above; caller guarantees a valid `CharacterController`.
    let ctrl = unsafe { &mut *controller };

    // SAFETY: Null-checked above; caller guarantees a valid `PhysicsWorld` or null.
    let pw = if physics.is_null() {
        None
    } else {
        Some(unsafe { &*physics })
    };

    let direction = Vec3::new(dir_x, 0.0, dir_z).normalize_or_zero();
    let input = crate::CharacterMovement {
        direction,
        wish_jump: false,
        delta_time: dt,
    };

    // Override speed if caller provided a non-zero value.
    if speed > 0.0 {
        ctrl.move_speed = speed;
    }

    let output = crate::process_movement(ctrl, &input, pw);

    // Apply the computed output back to the controller.
    ctrl.position = output.new_position;
    ctrl.velocity = output.new_velocity;
    ctrl.state = output.state;

    output.moved
}

/// Request a jump for the character.
///
/// Applies jump velocity immediately, bypassing the per-frame input system.
/// Returns `true` if the jump was initiated.
///
/// # Safety
///
/// `controller` must be a valid pointer to a `CharacterController`, or null.
#[no_mangle]
pub extern "C" fn character_jump(controller: *mut CharacterController) -> bool {
    if controller.is_null() {
        return false;
    }

    // SAFETY: Null-checked above; caller guarantees a valid `CharacterController`.
    let ctrl = unsafe { &mut *controller };

    if ctrl.is_grounded() {
        // Apply jump velocity directly (bypasses frame input system).
        ctrl.velocity.y = ctrl.jump_velocity;
        // Grounded → Jumping is a valid state transition per the state machine.
        let _ = ctrl.transition_state(CharacterState::Jumping);
        true
    } else {
        false
    }
}

/// Returns 1 if the character is on the ground, 0 otherwise.
///
/// # Safety
///
/// `controller` must be a valid pointer to a `CharacterController`, or null.
#[no_mangle]
pub extern "C" fn character_is_grounded(controller: *const CharacterController) -> i32 {
    if controller.is_null() {
        return 0;
    }

    // SAFETY: Null-checked above; caller guarantees a valid `CharacterController`.
    let ctrl = unsafe { &*controller };

    if ctrl.is_grounded() { 1 } else { 0 }
}

/// Returns the character's current movement state as an integer.
///
/// | Value | State     |
/// |-------|-----------|
/// | 0     | Grounded  |
/// | 1     | Jumping   |
/// | 2     | Falling   |
/// | 3     | Free      |
///
/// # Safety
///
/// `controller` must be a valid pointer to a `CharacterController`, or null.
#[no_mangle]
pub extern "C" fn character_get_move_state(controller: *const CharacterController) -> i32 {
    if controller.is_null() {
        return STATE_FALLING;
    }

    // SAFETY: Null-checked above; caller guarantees a valid `CharacterController`.
    let ctrl = unsafe { &*controller };

    match ctrl.state() {
        CharacterState::Grounded => STATE_GROUNDED,
        CharacterState::Jumping => STATE_JUMPING,
        CharacterState::Falling => STATE_FALLING,
        CharacterState::Free => STATE_FREE,
    }
}

/// Returns the character's current X (right) velocity component.
///
/// # Safety
///
/// `controller` must be a valid pointer to a `CharacterController`, or null.
#[no_mangle]
pub extern "C" fn character_get_velocity_x(controller: *const CharacterController) -> f32 {
    if controller.is_null() {
        return 0.0;
    }
    // SAFETY: Null-checked above.
    unsafe { (*controller).velocity.x }
}

/// Returns the character's current Y (up) velocity component.
///
/// # Safety
///
/// `controller` must be a valid pointer to a `CharacterController`, or null.
#[no_mangle]
pub extern "C" fn character_get_velocity_y(controller: *const CharacterController) -> f32 {
    if controller.is_null() {
        return 0.0;
    }
    // SAFETY: Null-checked above.
    unsafe { (*controller).velocity.y }
}

/// Returns the character's current Z (forward/backward) velocity component.
///
/// # Safety
///
/// `controller` must be a valid pointer to a `CharacterController`, or null.
#[no_mangle]
pub extern "C" fn character_get_velocity_z(controller: *const CharacterController) -> f32 {
    if controller.is_null() {
        return 0.0;
    }
    // SAFETY: Null-checked above.
    unsafe { (*controller).velocity.z }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CharacterController;

    // ── Smoke tests for null-safety ──────────────────────────────────────

    #[test]
    fn move_null_controller() {
        assert!(!character_move(std::ptr::null_mut(), 1.0, 0.0, 5.0, 1.0 / 60.0, std::ptr::null_mut()));
    }

    #[test]
    fn jump_null_controller() {
        assert!(!character_jump(std::ptr::null_mut()));
    }

    #[test]
    fn is_grounded_null_controller() {
        assert_eq!(character_is_grounded(std::ptr::null()), 0);
    }

    #[test]
    fn get_move_state_null_controller() {
        assert_eq!(character_get_move_state(std::ptr::null()), STATE_FALLING);
    }

    #[test]
    fn velocity_getters_null_controller() {
        assert_eq!(character_get_velocity_x(std::ptr::null()), 0.0);
        assert_eq!(character_get_velocity_y(std::ptr::null()), 0.0);
        assert_eq!(character_get_velocity_z(std::ptr::null()), 0.0);
    }

    // ── Functional smoke tests ──────────────────────────────────────────

    #[test]
    fn character_move_no_physics_updates_velocity() {
        let mut ctrl = CharacterController::new();
        ctrl.state = CharacterState::Grounded;
        ctrl.position = Vec3::new(0.0, 10.0, 0.0);
        let result = character_move(
            &mut ctrl as *mut CharacterController,
            1.0, 0.0,   // dir_x, dir_z
            5.0,         // speed
            1.0 / 60.0,  // dt
            std::ptr::null_mut(), // no physics
        );
        assert!(result, "should have moved");
        // Position should have changed (moved in +X)
        assert!(ctrl.position().x > 0.0);
        // Velocity should have been updated
        assert!((ctrl.velocity().x).abs() > 0.0);
    }

    #[test]
    fn character_jump_grounded_success() {
        let mut ctrl = CharacterController::new();
        ctrl.state = CharacterState::Grounded;
        ctrl.velocity = Vec3::ZERO;

        let jumped = character_jump(&mut ctrl as *mut CharacterController);
        assert!(jumped, "grounded character should be able to jump");
        assert_eq!(ctrl.state(), CharacterState::Jumping);
        assert!((ctrl.velocity().y - ctrl.jump_velocity).abs() < 1e-6, "jump velocity should be applied");
    }

    #[test]
    fn character_jump_airborne_fails() {
        let mut ctrl = CharacterController::new();
        // Default state is Falling (not grounded)
        let jumped = character_jump(&mut ctrl as *mut CharacterController);
        assert!(!jumped, "airborne character should not be able to jump");
    }

    #[test]
    fn character_is_grounded_true() {
        let mut ctrl = CharacterController::new();
        ctrl.state = CharacterState::Grounded;
        assert_eq!(character_is_grounded(&ctrl as *const CharacterController), 1);
    }

    #[test]
    fn character_is_grounded_false() {
        let ctrl = CharacterController::new(); // starts as Falling
        assert_eq!(character_is_grounded(&ctrl as *const CharacterController), 0);
    }

    #[test]
    fn character_get_move_state_values() {
        let mut ctrl = CharacterController::new();

        ctrl.state = CharacterState::Grounded;
        assert_eq!(character_get_move_state(&ctrl as *const CharacterController), STATE_GROUNDED);

        ctrl.state = CharacterState::Jumping;
        assert_eq!(character_get_move_state(&ctrl as *const CharacterController), STATE_JUMPING);

        ctrl.state = CharacterState::Falling;
        assert_eq!(character_get_move_state(&ctrl as *const CharacterController), STATE_FALLING);

        ctrl.state = CharacterState::Free;
        assert_eq!(character_get_move_state(&ctrl as *const CharacterController), STATE_FREE);
    }

    #[test]
    fn character_velocity_getters() {
        let mut ctrl = CharacterController::new();
        ctrl.velocity = Vec3::new(1.0, 2.0, 3.0);

        assert!((character_get_velocity_x(&ctrl as *const CharacterController) - 1.0).abs() < 1e-6);
        assert!((character_get_velocity_y(&ctrl as *const CharacterController) - 2.0).abs() < 1e-6);
        assert!((character_get_velocity_z(&ctrl as *const CharacterController) - 3.0).abs() < 1e-6);
    }
}
