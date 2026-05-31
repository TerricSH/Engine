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

use engine_ffi::component::register_component_type_with_id;

/// All (display_name, engine_TYPE_ID) pairs for registered component types.
///
/// The engine TYPE_ID string is the value of `Component::TYPE_ID` for that
/// type.  These are stable contract strings that MUST match the constants
/// defined in each component's `impl Component` block.
const COMPONENT_TYPE_ENTRIES: &[(&str, &str)] = &[
    // engine-scene core components
    ("Name", "engine.name"),
    ("Transform", "engine.transform"),
    ("Renderable", "engine.renderable"),
    ("Camera", "engine.camera"),
    ("Light", "engine.light"),
    ("Bounds", "engine.bounds"),
    // engine-physics components
    ("RigidBody", "engine.physics.rigid_body"),
    ("Collider", "engine.physics.collider"),
    ("PhysicsMaterial", "engine.physics.physics_material"),
    // engine-character components
    ("Character Controller", "engine.character_controller"),
    // engine-animation components
    ("AnimStateMachineInstance", "engine.animation.anim_state_machine_instance"),
    ("BoneAttachment", "engine.animation.bone_attachment"),
    ("RootMotion", "engine.animation.root_motion"),
];

/// Initialise the FFI callback registry and set the engine world pointer.
///
/// Called exactly once during `EngineRuntime::new()`. If `world` is
/// null, entity/component callbacks will return sentinel values until
/// the world is set.
pub fn initialise(world: *mut std::ffi::c_void) {
    engine_ffi::world_bridge::populate_registry(world);

    // Register all component types with their engine TYPE_IDs
    // so C# can read/write component data through component_get_ptr/set_ptr.
    for &(name, type_id) in COMPONENT_TYPE_ENTRIES {
        register_component_type_with_id(name, type_id);
    }
    tracing::info!(
        count = COMPONENT_TYPE_ENTRIES.len(),
        "Registered component types for C# scripting"
    );
}
