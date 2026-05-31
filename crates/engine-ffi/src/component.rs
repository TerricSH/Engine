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
    /// Maps FFI type ID → engine Component::TYPE_ID (e.g. "engine.physics.rigid_body").
    id_to_engine_type_id: HashMap<FfiComponentTypeId, &'static str>,
    next_id: u32,
}

impl ComponentRegistryInner {
    fn new() -> Self {
        Self {
            name_to_id: HashMap::new(),
            id_to_name: HashMap::new(),
            id_to_engine_type_id: HashMap::new(),
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

/// Register a component type with its engine TYPE_ID for FFI read/write.
///
/// `name` is the display name (e.g. `"RigidBody"`).
/// `engine_type_id` is the value of `Component::TYPE_ID` (e.g. `"engine.physics.rigid_body"`).
/// Returns the assigned FFI type ID.
///
/// This variant should be preferred over [`register_component_type`] because
/// it enables C# to read and write component data through
/// `component_get_ptr` / `component_set_ptr`.
pub fn register_component_type_with_id(name: &str, engine_type_id: &'static str) -> FfiComponentTypeId {
    let mut reg = COMPONENT_REGISTRY.write().unwrap();
    if let Some(&id) = reg.name_to_id.get(name) {
        reg.id_to_engine_type_id.insert(id, engine_type_id);
        return id;
    }
    let id = FfiComponentTypeId(reg.next_id);
    reg.next_id += 1;
    reg.name_to_id.insert(name.to_string(), id);
    reg.id_to_name.insert(id, name.to_string());
    reg.id_to_engine_type_id.insert(id, engine_type_id);
    tracing::info!(
        component = name,
        type_id = id.0,
        engine_type = engine_type_id,
        "Registered component type with engine TYPE_ID"
    );
    id
}

/// Look up the engine `Component::TYPE_ID` for a given FFI component type ID.
///
/// Returns `None` if the type was only registered via [`register_component_type`]
/// (which doesn't store the engine TYPE_ID).
pub fn lookup_engine_type_id(type_id: FfiComponentTypeId) -> Option<&'static str> {
    COMPONENT_REGISTRY
        .read()
        .unwrap()
        .id_to_engine_type_id
        .get(&type_id)
        .copied()
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
    // SAFETY: `name` was null-checked above; the caller guarantees a valid
    // NUL-terminated C string that lives for the duration of this FFI call.
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
