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

/// Initialise the FFI callback registry and set the engine world pointer.
///
/// Called exactly once during `EngineRuntime::new()`. If `world` is
/// null, entity/component callbacks will return sentinel values until
/// the world is set.
pub fn initialise(world: *mut std::ffi::c_void) {
    engine_ffi::world_bridge::populate_registry(world);
}
