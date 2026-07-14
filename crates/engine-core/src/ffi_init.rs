//! FFI callback registration — bridges the engine runtime to the
//! `engine-ffi` extern "C" entry points.
//!
//! Called once during [`EngineRuntime::new`] to populate the
//! [`engine_ffi::registry::FfiRegistry`] with real implementations that
//! operate on the engine's [`engine_scene::World`].
//!
//! The actual callback implementations and world pointer management live
//! in [`engine_ffi::world_bridge`] (which is not subject to
//! `forbid(unsafe_code)`).

/// Initialise the process-wide FFI callback and component registries.
///
/// This function is idempotent. World lifecycle is managed separately by
/// `engine_ffi::world_bridge::{activate_world, deactivate_world}`.
pub fn initialise() {
    engine_ffi::world_bridge::populate_registry();
}

/// Install the host callback registry into the native `engine_ffi` library
/// loaded by in-process C# through P/Invoke.
///
/// The Rust `rlib` and native `cdylib` have separate global state. This
/// explicit bridge is therefore required before managed code may access the
/// active [`engine_scene::World`]. Process-based script hosts must not call
/// this function because they do not share the engine process or World slot.
pub fn install_cdylib_bridge() -> Result<(), engine_ffi::host_bridge::HostBridgeError> {
    initialise();
    engine_ffi::host_bridge::install_cdylib_registry(engine_ffi::registry::get())
}
