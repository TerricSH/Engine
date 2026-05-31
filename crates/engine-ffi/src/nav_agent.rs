//! NavAgent FFI — forwarding layer.
//!
//! These `#[no_mangle] extern "C"` functions are the C# entry points
//! for AI agent control.  They delegate to `engine-nav`'s [`NavAgent`].
//!
//! # Safety
//!
//! All functions accept a raw pointer to a `NavAgent`.  Null pointers
//! are checked and return safe defaults.  Non-null pointers must
//! originate from a live `NavAgent` allocation.

use glam::Vec3;

/// Set the agent's movement destination.
///
/// Creates a straight‑line path from the agent's current position
/// to the target coordinates.  For navmesh‑aware pathfinding the
/// path should be computed separately and set via [`set_path`].
///
/// # Safety
///
/// `agent` must be a valid pointer to a `NavAgent`, or null.
#[no_mangle]
pub unsafe extern "C" fn nav_agent_set_target(
    agent: *mut std::ffi::c_void,
    x: f32,
    y: f32,
    z: f32,
) {
    if agent.is_null() {
        return;
    }
    // SAFETY: Null-checked above; caller guarantees a valid `NavAgent`.
    let agent = &mut *(agent as *mut engine_nav::NavAgent);
    agent.set_target(Vec3::new(x, y, z));
}

/// Get the agent's current world position.
///
/// Returns `true` on success, `false` if the agent pointer is null.
///
/// # Safety
///
/// * `agent` must be a valid pointer to a `NavAgent`, or null.
/// * `out_x`, `out_y`, `out_z` must be valid, non-null writeable pointers.
#[no_mangle]
pub unsafe extern "C" fn nav_agent_get_position(
    agent: *const std::ffi::c_void,
    out_x: *mut f32,
    out_y: *mut f32,
    out_z: *mut f32,
) -> bool {
    if agent.is_null() || out_x.is_null() || out_y.is_null() || out_z.is_null() {
        return false;
    }
    // SAFETY: Null-checked above; caller guarantees valid pointers.
    let agent = &*(agent as *const engine_nav::NavAgent);
    let pos = agent.position();
    *out_x = pos.x;
    *out_y = pos.y;
    *out_z = pos.z;
    true
}

/// Returns `true` when the agent has reached the end of its path
/// (or has no path assigned).
///
/// # Safety
///
/// `agent` must be a valid pointer to a `NavAgent`, or null.
#[no_mangle]
pub unsafe extern "C" fn nav_agent_is_path_finished(
    agent: *const std::ffi::c_void,
) -> bool {
    if agent.is_null() {
        return true;
    }
    // SAFETY: Null-checked above; caller guarantees a valid `NavAgent`.
    let agent = &*(agent as *const engine_nav::NavAgent);
    agent.is_path_finished()
}

/// Returns the remaining distance along the agent's path.
///
/// Returns `0.0` for null agents or when no path is set.
///
/// # Safety
///
/// `agent` must be a valid pointer to a `NavAgent`, or null.
#[no_mangle]
pub unsafe extern "C" fn nav_agent_get_remaining_distance(
    agent: *const std::ffi::c_void,
) -> f32 {
    if agent.is_null() {
        return 0.0;
    }
    // SAFETY: Null-checked above; caller guarantees a valid `NavAgent`.
    let agent = &*(agent as *const engine_nav::NavAgent);
    agent.remaining_distance()
}
