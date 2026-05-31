//! Animation player FFI — forwarding layer.
//!
//! These `#[no_mangle] extern "C"` functions are the C# entry points for
//! animation control: state machine parameters, forced state transitions,
//! direct clip playback, and bone position queries.

use std::ffi::CStr;

use engine_animation::AnimParamValue;

/// Set a float parameter on the animation state machine.
///
/// # Safety
///
/// `player` must be a valid pointer to `AnimationPlayer`, or null.
/// `name` must be a valid C string (null-terminated UTF-8).
#[no_mangle]
pub unsafe extern "C" fn animation_set_param_float(
    player: *mut std::ffi::c_void,
    name: *const std::ffi::c_char,
    value: f32,
) {
    if player.is_null() || name.is_null() {
        return;
    }
    // SAFETY: Both pointers null-checked above; caller guarantees valid objects or null.
    let player = &mut *(player as *mut engine_animation::AnimationPlayer);
    // If the C# string is not valid UTF-8, return early instead of silently
    // using an empty string (which would silently no-op).
    let name = match CStr::from_ptr(name).to_str() {
        Ok(s) => s,
        Err(_) => return,
    };
    if let Some(ref mut sm) = player.state_machine {
        sm.set_param(name, AnimParamValue::Float(value));
    }
}

/// Set a bool parameter on the animation state machine.
///
/// # Safety
///
/// `player` must be a valid pointer to `AnimationPlayer`, or null.
/// `name` must be a valid C string (null-terminated UTF-8).
#[no_mangle]
pub unsafe extern "C" fn animation_set_param_bool(
    player: *mut std::ffi::c_void,
    name: *const std::ffi::c_char,
    value: bool,
) {
    if player.is_null() || name.is_null() {
        return;
    }
    // SAFETY: Both pointers null-checked above; caller guarantees valid objects or null.
    let player = &mut *(player as *mut engine_animation::AnimationPlayer);
    let name = match CStr::from_ptr(name).to_str() {
        Ok(s) => s,
        Err(_) => return,
    };
    if let Some(ref mut sm) = player.state_machine {
        sm.set_param(name, AnimParamValue::Bool(value));
    }
}

/// Force the state machine to transition to a named state immediately.
///
/// Returns `true` if the state was found and the transition was performed.
///
/// # Safety
///
/// `player` must be a valid pointer to `AnimationPlayer`, or null.
/// `state_name` must be a valid C string (null-terminated UTF-8).
#[no_mangle]
pub unsafe extern "C" fn animation_force_state(
    player: *mut std::ffi::c_void,
    state_name: *const std::ffi::c_char,
) -> bool {
    if player.is_null() || state_name.is_null() {
        return false;
    }
    // SAFETY: Both pointers null-checked above; caller guarantees valid objects or null.
    let player = &mut *(player as *mut engine_animation::AnimationPlayer);
    let name = match CStr::from_ptr(state_name).to_str() {
        Ok(s) => s,
        Err(_) => return false,
    };
    if let Some(ref mut sm) = player.state_machine {
        sm.force_transition_to(name)
    } else {
        false
    }
}

/// Play a specific animation clip, bypassing the state machine.
///
/// # Safety
///
/// `player` must be a valid pointer to `AnimationPlayer`, or null.
/// `clip_asset` must be a valid C string (null-terminated UTF-8).
#[no_mangle]
pub unsafe extern "C" fn animation_play_clip(
    player: *mut std::ffi::c_void,
    clip_asset: *const std::ffi::c_char,
) {
    if player.is_null() || clip_asset.is_null() {
        return;
    }
    // SAFETY: Both pointers null-checked above; caller guarantees valid objects or null.
    let player = &mut *(player as *mut engine_animation::AnimationPlayer);
    let name = match CStr::from_ptr(clip_asset).to_str() {
        Ok(s) => s,
        Err(_) => return,
    };
    player.play_clip(name);
}

/// Get the number of bones in the cached positions array.
///
/// # Safety
///
/// `player` must be a valid pointer to `AnimationPlayer`, or null.
#[no_mangle]
pub unsafe extern "C" fn animation_bone_count(
    player: *const std::ffi::c_void,
) -> u32 {
    if player.is_null() {
        return 0;
    }
    // SAFETY: Null-checked above; caller guarantees valid `AnimationPlayer` or null.
    let player = &*(player as *const engine_animation::AnimationPlayer);
    player.cached_bone_positions.len() as u32
}

/// Get cached bone world positions.
///
/// Fills `output` with `x, y, z` triples up to `max_count` bones.
/// Returns the number of bones written.
///
/// # Safety
///
/// `player` must be a valid pointer to `AnimationPlayer`, or null.
/// `output` must be a valid writable buffer of at least `max_count * 3` floats.
/// The caller is responsible for ensuring the output buffer is large enough.
#[no_mangle]
pub unsafe extern "C" fn animation_get_bone_positions(
    player: *const std::ffi::c_void,
    output: *mut f32,
    max_count: u32,
) -> u32 {
    if player.is_null() || output.is_null() {
        return 0;
    }
    // SAFETY: Both pointers null-checked above. Output buffer size is the
    // caller's responsibility per the doc contract. We bound writes to
    // min(cached_bones, max_count) so we never write beyond what the caller
    // declared as available.
    let player = &*(player as *const engine_animation::AnimationPlayer);
    let count = (player.cached_bone_positions.len() as u32).min(max_count);
    for i in 0..count as usize {
        let pos = &player.cached_bone_positions[i];
        *output.add(i * 3) = pos[0];
        *output.add(i * 3 + 1) = pos[1];
        *output.add(i * 3 + 2) = pos[2];
    }
    count
}
