//! Character controller FFI — forwarding layer.
//!
//! These `#[no_mangle] extern "C"` functions are the actual C# entry points.
//! They delegate to `engine-character`'s implementation functions.

use engine_character::{
    character_get_move_state_ffi, character_get_velocity_x_ffi,
    character_get_velocity_y_ffi, character_get_velocity_z_ffi,
    character_is_grounded_ffi, character_jump_ffi, character_move_ffi,
};

/// Move the character in the given direction at the given speed.
/// # Safety
/// `controller` must be a valid pointer to a CharacterController, or null.
#[no_mangle]
pub unsafe extern "C" fn character_move(
    controller: *mut std::ffi::c_void,
    dir_x: f32,
    dir_z: f32,
    speed: f32,
    dt: f32,
    physics: *mut std::ffi::c_void,
) -> bool {
    character_move_ffi(
        controller as *mut engine_character::CharacterController,
        dir_x,
        dir_z,
        speed,
        dt,
        physics as *mut engine_physics::PhysicsWorld,
    )
}

/// Request a jump for the character.
/// # Safety
/// `controller` must be a valid pointer to a CharacterController, or null.
#[no_mangle]
pub unsafe extern "C" fn character_jump(
    controller: *mut std::ffi::c_void,
) -> bool {
    character_jump_ffi(controller as *mut engine_character::CharacterController)
}

/// Returns 1 if the character is on the ground, 0 otherwise.
/// # Safety
/// `controller` must be a valid pointer to a CharacterController, or null.
#[no_mangle]
pub unsafe extern "C" fn character_is_grounded(
    controller: *const std::ffi::c_void,
) -> i32 {
    character_is_grounded_ffi(controller as *const engine_character::CharacterController)
}

/// Returns the character's current movement state as an integer.
/// # Safety
/// `controller` must be a valid pointer to a CharacterController, or null.
#[no_mangle]
pub unsafe extern "C" fn character_get_move_state(
    controller: *const std::ffi::c_void,
) -> i32 {
    character_get_move_state_ffi(controller as *const engine_character::CharacterController)
}

/// Returns the character's current X (right) velocity component.
/// # Safety
/// `controller` must be a valid pointer to a CharacterController, or null.
#[no_mangle]
pub unsafe extern "C" fn character_get_velocity_x(
    controller: *const std::ffi::c_void,
) -> f32 {
    character_get_velocity_x_ffi(controller as *const engine_character::CharacterController)
}

/// Returns the character's current Y (up) velocity component.
/// # Safety
/// `controller` must be a valid pointer to a CharacterController, or null.
#[no_mangle]
pub unsafe extern "C" fn character_get_velocity_y(
    controller: *const std::ffi::c_void,
) -> f32 {
    character_get_velocity_y_ffi(controller as *const engine_character::CharacterController)
}

/// Returns the character's current Z (forward/backward) velocity component.
/// # Safety
/// `controller` must be a valid pointer to a CharacterController, or null.
#[no_mangle]
pub unsafe extern "C" fn character_get_velocity_z(
    controller: *const std::ffi::c_void,
) -> f32 {
    character_get_velocity_z_ffi(controller as *const engine_character::CharacterController)
}

/// Enable or disable foot IK for the character.
/// # Safety
/// `controller` must be a valid pointer to a CharacterController, or null.
#[no_mangle]
pub unsafe extern "C" fn character_set_foot_ik_enabled(
    controller: *mut std::ffi::c_void,
    enabled: bool,
) {
    if controller.is_null() {
        return;
    }
    let ctrl = &mut *(controller as *mut engine_character::CharacterController);
    ctrl.set_foot_ik_enabled(enabled);
}

/// Returns whether foot IK is enabled for the character.
/// # Safety
/// `controller` must be a valid pointer to a CharacterController, or null.
#[no_mangle]
pub unsafe extern "C" fn character_get_foot_ik_enabled(
    controller: *const std::ffi::c_void,
) -> bool {
    if controller.is_null() {
        return false;
    }
    let ctrl = &*(controller as *const engine_character::CharacterController);
    ctrl.is_foot_ik_enabled()
}
