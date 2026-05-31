//! IK target component FFI — forwarding layer.
//!
//! These `#[no_mangle] extern "C"` functions are the C# entry points for
//! setting and querying IK effector target positions.

use std::ffi::CStr;

/// Set an IK effector's target position.
///
/// Returns `true` if the effector was found and the target was updated.
///
/// # Safety
///
/// `ik` must be a valid pointer to `IkTargetComponent`, or null.
/// `name` must be a valid C string.
#[no_mangle]
pub unsafe extern "C" fn ik_set_effector_target(
    ik: *mut std::ffi::c_void,
    name: *const std::ffi::c_char,
    x: f32,
    y: f32,
    z: f32,
) -> bool {
    if ik.is_null() || name.is_null() {
        return false;
    }
    let ik = &mut *(ik as *mut engine_animation::IkTargetComponent);
    let name = match CStr::from_ptr(name).to_str() {
        Ok(s) => s,
        Err(_) => return false,
    };
    ik.set_target(name, glam::Vec3::new(x, y, z));
    true
}

/// Get an IK effector's current target position.
///
/// Returns `true` if the effector was found; `out_x`, `out_y`, `out_z` are
/// filled with the target position.
///
/// # Safety
///
/// `ik` must be a valid pointer to `IkTargetComponent`, or null.
/// `name` must be a valid C string.
/// `out_x`, `out_y`, `out_z` must be valid writable f32 pointers.
#[no_mangle]
pub unsafe extern "C" fn ik_get_effector_target(
    ik: *const std::ffi::c_void,
    name: *const std::ffi::c_char,
    out_x: *mut f32,
    out_y: *mut f32,
    out_z: *mut f32,
) -> bool {
    if ik.is_null() || name.is_null() || out_x.is_null() || out_y.is_null() || out_z.is_null() {
        return false;
    }
    let ik = &*(ik as *const engine_animation::IkTargetComponent);
    let name = match CStr::from_ptr(name).to_str() {
        Ok(s) => s,
        Err(_) => return false,
    };
    if let Some(effector) = ik.effector(name) {
        *out_x = effector.position.x;
        *out_y = effector.position.y;
        *out_z = effector.position.z;
        true
    } else {
        false
    }
}
