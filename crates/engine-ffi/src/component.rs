//! FFI functions for reading and writing ECS components.
//!
//! These functions are exported with `#[no_mangle] extern "C"` and called
//! from C# through P/Invoke (or CLR bindings in ILRuntime).
//!
//! # Component Type Registry
//!
//! Every component type must be registered at startup so that C# can look
//! up `FfiComponentTypeId` by name. The registry maps `"Gold" → type_id(1)`,
//! `"Position" → type_id(2)`, etc.

use std::collections::HashMap;
use std::ffi::CStr;
use std::sync::{LazyLock, RwLock};

use crate::types::FfiComponentTypeId;

// ---------------------------------------------------------------------------
// Component type registry
// ---------------------------------------------------------------------------

static COMPONENT_REGISTRY: LazyLock<RwLock<ComponentRegistryInner>> =
    LazyLock::new(|| RwLock::new(ComponentRegistryInner::new()));

struct ComponentRegistryInner {
    name_to_id: HashMap<String, FfiComponentTypeId>,
    id_to_name: HashMap<FfiComponentTypeId, String>,
    next_id: u32,
}

impl ComponentRegistryInner {
    fn new() -> Self {
        Self {
            name_to_id: HashMap::new(),
            id_to_name: HashMap::new(),
            next_id: 1, // 0 = INVALID
        }
    }
}

/// Register a component type so C# can look it up by name.
/// Returns the assigned type ID.
///
/// Called automatically by the engine at startup for each known component.
pub fn register_component_type(name: &str) -> FfiComponentTypeId {
    let mut reg = COMPONENT_REGISTRY.write().unwrap();
    if let Some(&id) = reg.name_to_id.get(name) {
        return id;
    }
    let id = FfiComponentTypeId(reg.next_id);
    reg.next_id += 1;
    reg.name_to_id.insert(name.to_string(), id);
    reg.id_to_name.insert(id, name.to_string());
    tracing::info!(
        component = name,
        type_id = id.0,
        "Registered component type"
    );
    id
}

/// Look up a component type ID by name.
pub fn lookup_component_type(name: &str) -> Option<FfiComponentTypeId> {
    COMPONENT_REGISTRY
        .read()
        .unwrap()
        .name_to_id
        .get(name)
        .copied()
}

/// Look up a component type name by ID (for debug / diagnostics).
pub fn lookup_component_name(type_id: FfiComponentTypeId) -> Option<String> {
    COMPONENT_REGISTRY
        .read()
        .unwrap()
        .id_to_name
        .get(&type_id)
        .cloned()
}

// ---------------------------------------------------------------------------
// Extern "C" exports
// ---------------------------------------------------------------------------

/// Look up a component type ID by name from C#.
///
/// Returns 0 (INVALID) if the component type is not registered.
///
/// # Safety
///
/// `name` must be a valid, null-terminated C string pointer or null.
#[no_mangle]
pub unsafe extern "C" fn ffi_component_type_id(
    name: *const std::ffi::c_char,
) -> FfiComponentTypeId {
    if name.is_null() {
        return FfiComponentTypeId::INVALID;
    }
    let c_str = unsafe { CStr::from_ptr(name) };
    match c_str.to_str() {
        Ok(s) => lookup_component_type(s).unwrap_or(FfiComponentTypeId::INVALID),
        Err(_) => FfiComponentTypeId::INVALID,
    }
}

/// Return the number of registered component types (for C# validation).
#[no_mangle]
pub extern "C" fn ffi_component_type_count() -> u32 {
    COMPONENT_REGISTRY.read().unwrap().next_id
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[test]
    fn register_and_lookup() {
        let id = register_component_type("TestComponent");
        assert_ne!(id, FfiComponentTypeId::INVALID);
        assert_eq!(lookup_component_type("TestComponent"), Some(id));
    }

    #[test]
    fn register_dedup() {
        let a = register_component_type("DedupTest");
        let b = register_component_type("DedupTest");
        assert_eq!(a, b);
    }

    #[test]
    fn lookup_missing() {
        assert_eq!(lookup_component_type("NonexistentComponent"), None);
    }

    #[test]
    fn lookup_name_roundtrip() {
        let id = register_component_type("RoundtripComponent");
        assert_eq!(
            lookup_component_name(id),
            Some("RoundtripComponent".to_string())
        );
    }

    #[test]
    fn ffi_lookup_null_safe() {
        let id = unsafe { ffi_component_type_id(std::ptr::null()) };
        assert_eq!(id, FfiComponentTypeId::INVALID);
    }

    #[test]
    fn ffi_lookup_by_name() {
        register_component_type("FFIComponent");
        let c_name = CString::new("FFIComponent").unwrap();
        let id = unsafe { ffi_component_type_id(c_name.as_ptr()) };
        assert_ne!(id, FfiComponentTypeId::INVALID);
    }
}
