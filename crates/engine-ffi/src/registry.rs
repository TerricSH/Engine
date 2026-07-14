//! Runtime callback registry for the FFI bridge.
//!
//! The engine runtime registers function pointers at startup so that
//! `extern "C"` FFI entry points can dispatch to the real systems
//! (entity manager, coroutine system, etc.) without `engine-ffi`
//! depending on `engine-core` or `engine-scene`.
//!
//! # Usage
//!
//! On startup, the engine calls [`register`] once with a fully populated
//! [`FfiRegistry`].  After that, any FFI function can call [`get`] to
//! obtain the registry and invoke the appropriate callback.
//!
//! # Safety
//!
//! Every function pointer in [`FfiRegistry`] MUST be valid for the
//! entire lifetime of the process (or until shutdown).  The registry is
//! meant to be populated once during engine initialisation and never
//! changed afterwards.

use std::sync::OnceLock;

use crate::types::{
    FfiAsyncCallback, FfiAsyncHandle, FfiComponentTypeId, FfiCoroutineHandle, FfiEntityId,
    FfiManagedCoroutineDescriptor,
};

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

/// ABI version of [`FfiRegistry`] passed from the Rust host into the cdylib.
/// Increment this whenever the callback table layout or signatures change.
pub const FFI_REGISTRY_ABI_VERSION: u32 = 3;

pub const FFI_INSTALL_OK: u32 = 0;
pub const FFI_INSTALL_NULL_REGISTRY: u32 = 1;
pub const FFI_INSTALL_VERSION_MISMATCH: u32 = 2;
pub const FFI_INSTALL_SIZE_MISMATCH: u32 = 3;
pub const FFI_INSTALL_ALREADY_INITIALIZED: u32 = 4;
pub const FFI_INSTALL_PANICKED: u32 = 5;

/// Function-pointer table routed through by the FFI entry points.
///
/// All fields use `extern "C"` ABI so the registry is safe to populate
/// from any Rust crate without layout concerns.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FfiRegistry {
    // ── Entity lifecycle ────────────────────────────────────────────────
    /// Spawn a new empty entity.  Returns [`FfiEntityId::INVALID`] on failure.
    pub entity_spawn: extern "C" fn() -> FfiEntityId,
    /// Destroy an entity.  Returns `true` on success.
    pub entity_destroy: extern "C" fn(entity: FfiEntityId) -> bool,
    /// Check whether an entity handle is still valid.
    pub entity_is_alive: extern "C" fn(entity: FfiEntityId) -> bool,

    /// Resolve a UTF-8 component name through the host's active type table.
    pub component_type_id: extern "C" fn(name: *const std::ffi::c_char) -> FfiComponentTypeId,
    /// Count entries in the host's active component type table.
    pub component_type_count: extern "C" fn() -> u32,

    // ── Component access ────────────────────────────────────────────────
    /// Read a component's raw data as a byte slice.
    pub component_get_ptr: extern "C" fn(
        entity: FfiEntityId,
        type_id: FfiComponentTypeId,
        out_len: &mut u32,
    ) -> *mut u8,
    /// Write component data from a byte slice.
    pub component_set_ptr: extern "C" fn(
        entity: FfiEntityId,
        type_id: FfiComponentTypeId,
        data: *const u8,
        len: u32,
    ) -> bool,

    // ── Coroutines ──────────────────────────────────────────────────────
    /// Start a managed coroutine and take ownership of its context on success.
    pub coroutine_start: unsafe extern "C" fn(
        descriptor: *const FfiManagedCoroutineDescriptor,
    ) -> FfiCoroutineHandle,
    /// Cancel a running coroutine.
    pub coroutine_cancel: extern "C" fn(handle: FfiCoroutineHandle),

    // ── Async I/O ───────────────────────────────────────────────────────
    /// Check whether an async operation has completed.
    pub async_is_complete: extern "C" fn(handle: FfiAsyncHandle) -> bool,
    /// Begin an async image load.
    pub async_load_image: extern "C" fn(
        url: *const std::ffi::c_char,
        callback: FfiAsyncCallback,
        user_data: u64,
    ) -> FfiAsyncHandle,
    /// Begin an async HTTP GET.
    pub async_http_get: extern "C" fn(
        url: *const std::ffi::c_char,
        callback: FfiAsyncCallback,
        user_data: u64,
    ) -> FfiAsyncHandle,

    // ── Condition evaluation ────────────────────────────────────────────
    // ── Lifecycle ───────────────────────────────────────────────────────
    /// Called once per frame to dispatch pending main-thread callbacks.
    pub dispatch_main_thread_callbacks: extern "C" fn(),
}

// ---------------------------------------------------------------------------
// Global storage
// ---------------------------------------------------------------------------

static REGISTRY: OnceLock<FfiRegistry> = OnceLock::new();

/// Register the FFI callback table.
///
/// Must be called **exactly once** during engine startup, before any FFI
/// entry point is invoked from C#.  Returns `Ok(())` on success or
/// `Err(registry)` if already initialised.
pub fn register(registry: FfiRegistry) -> Result<(), FfiRegistry> {
    REGISTRY.set(registry)
}

/// Returns `true` if the registry has been populated.
pub fn is_initialized() -> bool {
    REGISTRY.get().is_some()
}

/// Return the ABI version expected by this native library.
#[no_mangle]
pub extern "C" fn ffi_registry_abi_version() -> u32 {
    FFI_REGISTRY_ABI_VERSION
}

/// Return the byte size of the callback table expected by this native library.
#[no_mangle]
pub extern "C" fn ffi_registry_struct_size() -> u32 {
    std::mem::size_of::<FfiRegistry>()
        .try_into()
        .unwrap_or(u32::MAX)
}

/// Install the host callback table into the dynamically loaded cdylib copy.
///
/// The Rust host links an `rlib`, while C# loads the `cdylib`; Rust statics are
/// not shared between those modules. The host resolves this entry point from
/// the cdylib and copies a versioned table of `extern "C"` callbacks that route
/// back into the host-owned world.
///
/// # Safety
///
/// `host_registry` must point to a readable [`FfiRegistry`] whose layout is
/// described by `abi_version` and `struct_size`. The table is copied before
/// this function returns.
#[no_mangle]
pub unsafe extern "C" fn ffi_install_host_registry(
    host_registry: *const FfiRegistry,
    abi_version: u32,
    struct_size: u32,
) -> u32 {
    if host_registry.is_null() {
        return FFI_INSTALL_NULL_REGISTRY;
    }
    if abi_version != FFI_REGISTRY_ABI_VERSION {
        return FFI_INSTALL_VERSION_MISMATCH;
    }
    if struct_size != ffi_registry_struct_size() {
        return FFI_INSTALL_SIZE_MISMATCH;
    }

    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // SAFETY: The pointer and exact table layout were validated above;
        // the caller guarantees the memory is readable for this call.
        let registry = unsafe { *host_registry };
        match register(registry) {
            Ok(()) => FFI_INSTALL_OK,
            Err(_) => FFI_INSTALL_ALREADY_INITIALIZED,
        }
    }))
    .unwrap_or(FFI_INSTALL_PANICKED)
}

/// Obtain the global [`FfiRegistry`].
///
/// # Panics
///
/// Panics if [`register`] has not been called yet.  Callers that might
/// run before initialisation should check [`is_initialized`] first.
pub fn get() -> &'static FfiRegistry {
    REGISTRY
        .get()
        .expect("FfiRegistry not initialised — call engine_init_ffi() first")
}
