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
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;
use std::sync::{LazyLock, RwLock};

use crate::registry;
use crate::types::{FfiComponentTypeId, FfiEntityId};

// ---------------------------------------------------------------------------
// Component type registry
// ---------------------------------------------------------------------------

static COMPONENT_REGISTRY: LazyLock<RwLock<ComponentRegistryInner>> =
    LazyLock::new(|| RwLock::new(ComponentRegistryInner::new()));

#[cfg(test)]
static COMPONENT_REGISTRY_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
pub(crate) fn lock_component_registry_for_test() -> std::sync::MutexGuard<'static, ()> {
    COMPONENT_REGISTRY_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

struct ComponentRegistryInner {
    name_to_id: HashMap<String, FfiComponentTypeId>,
    id_to_name: HashMap<FfiComponentTypeId, String>,
    /// Maps FFI type ID → engine Component::TYPE_ID (e.g. "engine.physics.rigid_body").
    id_to_engine_type_id: HashMap<FfiComponentTypeId, &'static str>,
    stable_name_to_id: HashMap<String, FfiComponentTypeId>,
    stable_engine_type_to_id: HashMap<&'static str, FfiComponentTypeId>,
    next_id: u32,
}

impl ComponentRegistryInner {
    fn new() -> Self {
        Self {
            name_to_id: HashMap::new(),
            id_to_name: HashMap::new(),
            id_to_engine_type_id: HashMap::new(),
            stable_name_to_id: HashMap::new(),
            stable_engine_type_to_id: HashMap::new(),
            next_id: 1, // 0 = INVALID
        }
    }

    fn clear_active(&mut self) {
        self.name_to_id.clear();
        self.id_to_name.clear();
        self.id_to_engine_type_id.clear();
    }

    fn allocate_for_name(&mut self, name: &str) -> FfiComponentTypeId {
        if let Some(&id) = self.stable_name_to_id.get(name) {
            return id;
        }
        let id = self.allocate_id();
        self.stable_name_to_id.insert(name.to_string(), id);
        id
    }

    fn allocate_id(&mut self) -> FfiComponentTypeId {
        let id = FfiComponentTypeId(self.next_id);
        self.next_id += 1;
        id
    }

    fn allocate_for_engine_type(
        &mut self,
        name: &str,
        engine_type_id: &'static str,
    ) -> FfiComponentTypeId {
        if let Some(&id) = self.stable_engine_type_to_id.get(engine_type_id) {
            self.stable_name_to_id.entry(name.to_string()).or_insert(id);
            return id;
        }

        // Engine TYPE_ID is the canonical identity. A display name may be
        // shared by unrelated component types, so it must never cause two
        // different engine types to reuse one numeric ID.
        let id = self.allocate_id();
        self.stable_engine_type_to_id.insert(engine_type_id, id);
        // Preserve the first display-name mapping as a compatibility alias;
        // later collisions remain reachable through their canonical TYPE_ID.
        self.stable_name_to_id.entry(name.to_string()).or_insert(id);
        id
    }

    fn activate(&mut self, name: &str, engine_type_id: Option<&'static str>) -> FfiComponentTypeId {
        let id = match engine_type_id {
            Some(engine_type_id) => self.allocate_for_engine_type(name, engine_type_id),
            None => self.allocate_for_name(name),
        };
        if let Some(engine_type_id) = engine_type_id {
            // Canonical engine aliases always win over colliding display names.
            self.name_to_id.insert(engine_type_id.to_string(), id);
            self.name_to_id.entry(name.to_string()).or_insert(id);
            self.id_to_name.insert(id, engine_type_id.to_string());
            self.id_to_engine_type_id.insert(id, engine_type_id);
        } else {
            self.name_to_id.entry(name.to_string()).or_insert(id);
            self.id_to_name.insert(id, name.to_string());
        }
        id
    }
}

/// Register a component type so C# can look it up by name.
/// Returns the assigned type ID.
///
/// Called automatically by the engine at startup for each known component.
pub fn register_component_type(name: &str) -> FfiComponentTypeId {
    let mut reg = COMPONENT_REGISTRY
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let id = reg.activate(name, None);
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
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .name_to_id
        .get(name)
        .copied()
}

/// Look up a component type name by ID (for debug / diagnostics).
pub fn lookup_component_name(type_id: FfiComponentTypeId) -> Option<String> {
    COMPONENT_REGISTRY
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .id_to_name
        .get(&type_id)
        .cloned()
}

/// Number of distinct component IDs in the current active runtime table.
/// Canonical TYPE_ID names and compatibility display aliases count once.
pub fn component_type_count() -> u32 {
    COMPONENT_REGISTRY
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .id_to_name
        .len()
        .try_into()
        .unwrap_or(u32::MAX)
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
pub fn register_component_type_with_id(
    name: &str,
    engine_type_id: &'static str,
) -> FfiComponentTypeId {
    let mut reg = COMPONENT_REGISTRY
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let id = reg.activate(name, Some(engine_type_id));
    tracing::info!(
        component = name,
        type_id = id.0,
        engine_type = engine_type_id,
        "Registered component type with engine TYPE_ID"
    );
    id
}

/// Replace the complete component type table used by the active runtime.
///
/// A process-lifetime allocator keeps each engine type's numeric ID stable,
/// while types belonging to a previously active runtime are removed from the
/// lookup surface instead of leaking into the new runtime.
pub(crate) fn replace_component_types(entries: &[(&'static str, &'static str)]) {
    let mut reg = COMPONENT_REGISTRY
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    reg.clear_active();

    for &(name, engine_type_id) in entries {
        reg.activate(name, Some(engine_type_id));
    }
}

/// Clear the component type table when no runtime world is active.
pub(crate) fn clear_component_types() {
    let mut reg = COMPONENT_REGISTRY
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    reg.clear_active();
}

/// Look up the engine `Component::TYPE_ID` for a given FFI component type ID.
///
/// Returns `None` if the type was only registered via [`register_component_type`]
/// (which doesn't store the engine TYPE_ID).
pub fn lookup_engine_type_id(type_id: FfiComponentTypeId) -> Option<&'static str> {
    COMPONENT_REGISTRY
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
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
    if registry::is_initialized() {
        return (registry::get().component_type_id)(name);
    }
    // SAFETY: this export has the same pointer contract as the helper.
    unsafe { lookup_component_type_ptr(name) }
}

/// Resolve a C string directly against this module's active table without
/// routing through the host callback registry.
///
/// # Safety
///
/// `name` must be null or point to a valid NUL-terminated C string.
pub(crate) unsafe fn lookup_component_type_ptr(
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
    if registry::is_initialized() {
        (registry::get().component_type_count)()
    } else {
        component_type_count()
    }
}

/// Copy one component's serialized UTF-8 JSON into a caller-owned buffer.
///
/// This is a two-call length/buffer protocol:
///
/// 1. Call with `buffer = null` and `buffer_capacity = 0`. The function
///    returns `false` and writes the required byte length to `out_len`.
/// 2. Allocate at least that many bytes and call again. The function returns
///    `true` after copying the complete JSON document and writes the actual
///    byte length to `out_len`.
///
/// A zero `out_len` means that the entity is stale, the component is absent,
/// the type is unknown/not serializable, or the FFI registry is unavailable.
/// No pointer into ECS storage or a temporary Rust buffer escapes this call.
///
/// # Safety
///
/// When non-null, `out_len` must be writable for one `u32`. When non-null,
/// `buffer` must be writable for at least `buffer_capacity` bytes.
#[no_mangle]
pub unsafe extern "C" fn ffi_component_get(
    entity: FfiEntityId,
    type_id: FfiComponentTypeId,
    buffer: *mut u8,
    buffer_capacity: u32,
    out_len: *mut u32,
) -> bool {
    if out_len.is_null() {
        return false;
    }

    // SAFETY: `out_len` was null-checked; the caller guarantees it is writable.
    unsafe { *out_len = 0 };

    let result = catch_unwind(AssertUnwindSafe(|| {
        if !registry::is_initialized() {
            return false;
        }

        let mut required_len = 0_u32;
        let source = (registry::get().component_get_ptr)(entity, type_id, &mut required_len);

        // Publish the required length even when this is only the sizing call
        // or when the supplied buffer is too small.
        // SAFETY: `out_len` was validated before entering the unwind guard.
        unsafe { *out_len = required_len };

        // SAFETY: A non-null source pointer and its length come from the
        // registered callback. The destination contract is documented above.
        unsafe { copy_component_bytes(source.cast_const(), required_len, buffer, buffer_capacity) }
    }));

    match result {
        Ok(copied) => copied,
        Err(_) => {
            // SAFETY: `out_len` was validated before entering the unwind guard.
            unsafe { *out_len = 0 };
            false
        }
    }
}

/// Deserialize one component from caller-owned UTF-8 JSON bytes.
///
/// Returns `false` for null/empty input, invalid UTF-8 or JSON, a stale
/// entity, an unknown/not-deserializable component type, an unavailable
/// registry, or a callback failure.
///
/// # Safety
///
/// `data` must point to at least `len` readable bytes for the duration of
/// this call. A null pointer is accepted and returns `false`.
#[no_mangle]
pub unsafe extern "C" fn ffi_component_set(
    entity: FfiEntityId,
    type_id: FfiComponentTypeId,
    data: *const u8,
    len: u32,
) -> bool {
    if data.is_null() || len == 0 {
        return false;
    }

    catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: The pointer was null-checked and the caller guarantees at
        // least `len` readable bytes for the duration of this call.
        let bytes = unsafe { std::slice::from_raw_parts(data, len as usize) };
        if std::str::from_utf8(bytes).is_err()
            || serde_json::from_slice::<serde_json::Value>(bytes).is_err()
        {
            return false;
        }
        if !registry::is_initialized() {
            return false;
        }

        (registry::get().component_set_ptr)(entity, type_id, data, len)
    }))
    .unwrap_or(false)
}

unsafe fn copy_component_bytes(
    source: *const u8,
    required_len: u32,
    buffer: *mut u8,
    buffer_capacity: u32,
) -> bool {
    if source.is_null() || required_len == 0 || buffer.is_null() || buffer_capacity < required_len {
        return false;
    }

    // SAFETY: The source callback guarantees `required_len` readable bytes.
    // The caller guarantees `buffer_capacity` writable bytes and the check
    // above establishes that the destination is large enough. The registry's
    // thread-local source buffer cannot alias caller-owned managed memory.
    unsafe { ptr::copy_nonoverlapping(source, buffer, required_len as usize) };
    true
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
        let _guard = lock_component_registry_for_test();
        let id = register_component_type("TestComponent");
        assert_ne!(id, FfiComponentTypeId::INVALID);
        assert_eq!(lookup_component_type("TestComponent"), Some(id));
    }

    #[test]
    fn register_dedup() {
        let _guard = lock_component_registry_for_test();
        let a = register_component_type("DedupTest");
        let b = register_component_type("DedupTest");
        assert_eq!(a, b);
    }

    #[test]
    fn lookup_missing() {
        let _guard = lock_component_registry_for_test();
        assert_eq!(lookup_component_type("NonexistentComponent"), None);
    }

    #[test]
    fn lookup_name_roundtrip() {
        let _guard = lock_component_registry_for_test();
        let id = register_component_type("RoundtripComponent");
        assert_eq!(
            lookup_component_name(id),
            Some("RoundtripComponent".to_string())
        );
    }

    #[test]
    fn ffi_lookup_null_safe() {
        let _guard = lock_component_registry_for_test();
        let id = unsafe { ffi_component_type_id(std::ptr::null()) };
        assert_eq!(id, FfiComponentTypeId::INVALID);
    }

    #[test]
    fn ffi_lookup_by_name() {
        let _guard = lock_component_registry_for_test();
        register_component_type("FFIComponent");
        let c_name = CString::new("FFIComponent").unwrap();
        let id = unsafe { ffi_component_type_id(c_name.as_ptr()) };
        assert_ne!(id, FfiComponentTypeId::INVALID);
    }

    #[test]
    fn component_get_rejects_null_out_length() {
        let _guard = lock_component_registry_for_test();
        let copied = unsafe {
            ffi_component_get(
                FfiEntityId::INVALID,
                FfiComponentTypeId::INVALID,
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
            )
        };
        assert!(!copied);
    }

    #[test]
    fn component_set_rejects_null_and_invalid_json() {
        let _guard = lock_component_registry_for_test();
        assert!(!unsafe {
            ffi_component_set(
                FfiEntityId::INVALID,
                FfiComponentTypeId::INVALID,
                std::ptr::null(),
                0,
            )
        });

        let invalid_json = b"not-json";
        assert!(!unsafe {
            ffi_component_set(
                FfiEntityId::INVALID,
                FfiComponentTypeId::INVALID,
                invalid_json.as_ptr(),
                invalid_json.len() as u32,
            )
        });
    }

    #[test]
    fn component_copy_uses_length_then_caller_buffer() {
        let _guard = lock_component_registry_for_test();
        let source = br#"{"value":42}"#;
        let mut destination = vec![0_u8; source.len()];

        assert!(!unsafe {
            copy_component_bytes(
                source.as_ptr(),
                source.len() as u32,
                std::ptr::null_mut(),
                0,
            )
        });
        assert!(!unsafe {
            copy_component_bytes(
                source.as_ptr(),
                source.len() as u32,
                destination.as_mut_ptr(),
                (source.len() - 1) as u32,
            )
        });
        assert!(unsafe {
            copy_component_bytes(
                source.as_ptr(),
                source.len() as u32,
                destination.as_mut_ptr(),
                destination.len() as u32,
            )
        });
        assert_eq!(destination, source);
    }

    #[test]
    fn canonical_engine_type_ids_survive_display_name_collisions() {
        let _guard = lock_component_registry_for_test();
        replace_component_types(&[
            ("Shared Display Name", "test.ffi.collision.first"),
            ("Shared Display Name", "test.ffi.collision.second"),
        ]);

        let first = lookup_component_type("test.ffi.collision.first").unwrap();
        let second = lookup_component_type("test.ffi.collision.second").unwrap();
        assert_ne!(first, second);
        assert_eq!(
            lookup_component_type("Shared Display Name"),
            Some(first),
            "the first display alias remains stable instead of being overwritten"
        );
        assert_eq!(component_type_count(), 2, "aliases must not inflate count");
        assert_eq!(
            lookup_engine_type_id(first),
            Some("test.ffi.collision.first")
        );
        assert_eq!(
            lookup_engine_type_id(second),
            Some("test.ffi.collision.second")
        );

        clear_component_types();
    }
}
