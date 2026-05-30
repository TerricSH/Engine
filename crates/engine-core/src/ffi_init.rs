//! FFI callback registration + component type registration — bridges the
//! engine runtime to the `engine-ffi` extern "C" entry points.
//!
//! Called once during [`EngineRuntime::new`] to populate the
//! [`engine_ffi::registry::FfiRegistry`] with real implementations that
//! operate on the engine's [`engine_scene::World`].
//!
//! The actual callback implementations and world pointer management live
//! in [`engine_ffi::world_bridge`] (which is not subject to
//! `forbid(unsafe_code)`).

use engine_ffi::component::register_component_type;

/// Initialise the FFI callback registry and set the engine world pointer.
///
/// Called exactly once during `EngineRuntime::new()`. If `world` is
/// null, entity/component callbacks will return sentinel values until
/// the world is set.
///
/// Also registers all known component type names with the FFI system so
/// that C# scripts can look up component type IDs by name.
pub fn initialise(world: *mut std::ffi::c_void) {
    engine_ffi::world_bridge::populate_registry(world);

    // ── Register component type names with the FFI system ────────────
    // so that C# scripts can look up component type IDs by name.
    //
    // Core engine components (defined in engine-scene):
    register_component_type("Name");
    register_component_type("Transform");
    register_component_type("Renderable");
    register_component_type("Camera");
    register_component_type("Light");
    register_component_type("Bounds");

    // Physics components (defined in engine-physics):
    register_component_type("RigidBody");
    register_component_type("Collider");
    register_component_type("PhysicsMaterial");

    // Character controller (defined in engine-character):
    register_component_type("Character Controller");

    // Animation components (defined in engine-animation):
    register_component_type("AnimStateMachineInstance");
    register_component_type("BoneAttachment");
    register_component_type("RootMotion");

    tracing::info!(count = 14, "Registered component types for C# scripting");
}
