//! FFI functions for entity lifecycle operations.
//!
//! These functions are exported with `#[no_mangle] extern "C"` and called
//! from C# through P/Invoke.

use crate::types::FfiEntityId;

// ---------------------------------------------------------------------------
// Entity lifecycle (extern "C")
// ---------------------------------------------------------------------------

/// Spawn a new empty entity and return its ID.
/// The entity starts with no components — C# should add them via
/// `ffi_component_set` or the C# helper APIs.
#[no_mangle]
pub extern "C" fn ffi_entity_spawn(world: *mut std::ffi::c_void) -> FfiEntityId {
    let _ = world;
    // TODO: implement when world pointer is plumbed through
    tracing::warn!("ffi_entity_spawn: not yet implemented");
    FfiEntityId::INVALID
}

/// Destroy an entity and all its components.
#[no_mangle]
pub extern "C" fn ffi_entity_destroy(world: *mut std::ffi::c_void, entity: FfiEntityId) -> bool {
    let _ = (world, entity);
    // TODO: implement when world pointer is plumbed through
    tracing::warn!("ffi_entity_destroy: not yet implemented");
    false
}

/// Check whether an entity handle is still valid (alive).
#[no_mangle]
pub extern "C" fn ffi_entity_is_alive(world: *mut std::ffi::c_void, entity: FfiEntityId) -> bool {
    let _ = (world, entity);
    // TODO: implement when world pointer is plumbed through
    entity.is_valid()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_id_invalid() {
        assert!(!FfiEntityId::INVALID.is_valid());
    }

    #[test]
    fn entity_id_valid() {
        let id = FfiEntityId {
            index: 1,
            generation: 0,
        };
        assert!(id.is_valid());
    }
}
