//! FFI functions for entity lifecycle operations.
//!
//! These functions are exported with `#[no_mangle] extern "C"` and called
//! from C# through P/Invoke.  They dispatch through the runtime callback
//! registry ([`crate::registry`]) which is populated by the engine during
//! startup.

use crate::registry;
use crate::types::FfiEntityId;

// ---------------------------------------------------------------------------
// Entity lifecycle (extern "C")
// ---------------------------------------------------------------------------

/// Spawn a new empty entity and return its ID.
///
/// The entity starts with no components — C# should add them via
/// `ffi_component_set` or the C# helper APIs.
#[no_mangle]
pub extern "C" fn ffi_entity_spawn() -> FfiEntityId {
    if !registry::is_initialized() {
        tracing::warn!("ffi_entity_spawn: engine not initialised");
        return FfiEntityId::INVALID;
    }
    (registry::get().entity_spawn)()
}

/// Destroy an entity and all its components.
///
/// Returns `true` if the entity existed and was destroyed.
#[no_mangle]
pub extern "C" fn ffi_entity_destroy(entity: FfiEntityId) -> bool {
    if !registry::is_initialized() {
        tracing::warn!("ffi_entity_destroy: engine not initialised");
        return false;
    }
    (registry::get().entity_destroy)(entity)
}

/// Check whether an entity handle is still valid (alive).
#[no_mangle]
pub extern "C" fn ffi_entity_is_alive(entity: FfiEntityId) -> bool {
    if !registry::is_initialized() {
        return entity.is_valid();
    }
    (registry::get().entity_is_alive)(entity)
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

    #[test]
    fn spawn_before_init_returns_invalid() {
        // Before registry is initialised, spawn should return INVALID.
        let id = ffi_entity_spawn();
        assert_eq!(id, FfiEntityId::INVALID);
    }

    #[test]
    fn destroy_before_init_returns_false() {
        assert!(!ffi_entity_destroy(FfiEntityId { index: 1, generation: 0 }));
    }

    #[test]
    fn alive_before_init_uses_fallback() {
        let valid = FfiEntityId { index: 1, generation: 0 };
        assert!(ffi_entity_is_alive(valid));
        assert!(!ffi_entity_is_alive(FfiEntityId::INVALID));
    }
}
