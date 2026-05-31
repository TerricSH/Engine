//! Audio system FFI — forwarding layer for C# scripting.
//!
//! These `#[no_mangle] extern "C"` functions are the C# entry points for
//! playing sounds, adjusting volume, and configuring the audio listener.
//! All functions operate on an `AudioEngine` pointer passed from the engine
//! service layer.

use std::ffi::{c_void, CStr};
use std::os::raw::c_char;
use std::path::Path;
use std::sync::Arc;

use engine_audio::{AudioClip, AudioEngine};

// ---------------------------------------------------------------------------
// audio_play_sound
// ---------------------------------------------------------------------------

/// Play a sound clip by asset path.
///
/// Returns a `u64` handle ID that can be passed to [`audio_stop_sound`] or
/// [`audio_set_volume`]. Returns `0` if the clip could not be loaded or
/// played (the error is logged).
///
/// # Safety
///
/// * `engine` must be a valid pointer to an `AudioEngine`, or null.
/// * `clip_asset` must be a valid, null-terminated C string pointer, or null.
#[no_mangle]
pub unsafe extern "C" fn audio_play_sound(
    engine: *mut c_void,
    clip_asset: *const c_char,
    volume: f32,
    looping: bool,
) -> u64 {
    if engine.is_null() || clip_asset.is_null() {
        return 0;
    }

    // SAFETY: Null-checked above; caller guarantees a valid AudioEngine pointer.
    let eng = unsafe { &mut *(engine as *mut AudioEngine) };

    // SAFETY: Null-checked above; caller guarantees a valid NUL-terminated C string.
    let asset_path = match unsafe { CStr::from_ptr(clip_asset) }.to_str() {
        Ok(s) => s,
        Err(_) => {
            tracing::error!("audio_play_sound: clip_asset is not valid UTF-8");
            return 0;
        }
    };

    let clip = match AudioClip::decode(Path::new(asset_path)) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("audio_play_sound: failed to decode '{}': {}", asset_path, e);
            return 0;
        }
    };

    let mut handle = match eng.play(Arc::new(clip)) {
        Ok(h) => h,
        Err(e) => {
            tracing::error!("audio_play_sound: engine.play failed: {}", e);
            return 0;
        }
    };

    // Apply requested volume and looping through the handle.
    let vol = volume.clamp(0.0, 1.0);
    let _ = handle.set_volume(vol);
    let _ = handle.set_loop(looping);

    handle.id()
}

// ---------------------------------------------------------------------------
// audio_stop_sound
// ---------------------------------------------------------------------------

/// Stop a currently playing sound by handle ID.
///
/// # Safety
///
/// `engine` must be a valid pointer to an `AudioEngine`, or null.
#[no_mangle]
pub unsafe extern "C" fn audio_stop_sound(engine: *mut c_void, handle_id: u64) {
    if engine.is_null() || handle_id == 0 {
        return;
    }

    // SAFETY: Null-checked above; caller guarantees a valid AudioEngine pointer.
    let eng = unsafe { &mut *(engine as *mut AudioEngine) };

    if eng.stop(handle_id) {
        tracing::debug!("audio_stop_sound({}): stopped", handle_id);
    } else {
        tracing::warn!("audio_stop_sound({}): handle not found", handle_id);
    }
}

// ---------------------------------------------------------------------------
// audio_set_volume
// ---------------------------------------------------------------------------

/// Set the volume of a playing sound by handle ID.
///
/// `volume` is clamped to `[0, 1]`.
///
/// # Safety
///
/// `engine` must be a valid pointer to an `AudioEngine`, or null.
#[no_mangle]
pub unsafe extern "C" fn audio_set_volume(engine: *mut c_void, handle_id: u64, volume: f32) {
    if engine.is_null() || handle_id == 0 {
        return;
    }

    // SAFETY: Null-checked above; caller guarantees a valid AudioEngine pointer.
    let eng = unsafe { &mut *(engine as *mut AudioEngine) };

    if eng.set_volume(handle_id, volume) {
        tracing::debug!("audio_set_volume({}, {}): set", handle_id, volume);
    } else {
        tracing::warn!(
            "audio_set_volume({}, {}): handle not found",
            handle_id,
            volume
        );
    }
}

// ---------------------------------------------------------------------------
// audio_set_listener
// ---------------------------------------------------------------------------

/// Set the audio listener position and orientation.
///
/// Parameters:
/// * `x, y, z`     — listener world-space position.
/// * `fx, fy, fz`  — listener forward vector (should be normalised).
/// * `ux, uy, uz`  — listener up vector (should be normalised).
///
/// # Safety
///
/// `engine` must be a valid pointer to an `AudioEngine`, or null.
#[no_mangle]
pub unsafe extern "C" fn audio_set_listener(
    engine: *mut c_void,
    x: f32,
    y: f32,
    z: f32,
    fx: f32,
    fy: f32,
    fz: f32,
    ux: f32,
    uy: f32,
    uz: f32,
) {
    if engine.is_null() {
        return;
    }

    // SAFETY: Null-checked above; caller guarantees a valid AudioEngine pointer.
    let eng = unsafe { &mut *(engine as *mut AudioEngine) };

    use engine_audio::AudioListener;

    let mut listener = AudioListener::new();
    listener.set_position(glam::Vec3::new(x, y, z));
    listener.set_orientation(glam::Vec3::new(fx, fy, fz), glam::Vec3::new(ux, uy, uz));
    eng.set_listener(listener);
}

// ---------------------------------------------------------------------------
// audio_set_master_volume
// ---------------------------------------------------------------------------

/// Set the global master volume (`0.0`–`1.0`).
///
/// # Safety
///
/// `engine` must be a valid pointer to an `AudioEngine`, or null.
#[no_mangle]
pub unsafe extern "C" fn audio_set_master_volume(engine: *mut c_void, volume: f32) {
    if engine.is_null() {
        return;
    }

    // SAFETY: Null-checked above; caller guarantees a valid AudioEngine pointer.
    let eng = unsafe { &mut *(engine as *mut AudioEngine) };

    eng.set_master_volume(volume);
}
