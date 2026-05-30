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
    FfiYieldInstruction,
};

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

/// Function-pointer table routed through by the FFI entry points.
///
/// All fields use `extern "C"` ABI so the registry is safe to populate
/// from any Rust crate without layout concerns.
#[repr(C)]
pub struct FfiRegistry {
    // ── Entity lifecycle ────────────────────────────────────────────────
    /// Spawn a new empty entity.  Returns [`FfiEntityId::INVALID`] on failure.
    pub entity_spawn: extern "C" fn() -> FfiEntityId,
    /// Destroy an entity.  Returns `true` on success.
    pub entity_destroy: extern "C" fn(entity: FfiEntityId) -> bool,
    /// Check whether an entity handle is still valid.
    pub entity_is_alive: extern "C" fn(entity: FfiEntityId) -> bool,

    // ── Component access ────────────────────────────────────────────────
    /// Read a component's raw data as a byte slice.
    pub component_get_ptr:
        extern "C" fn(entity: FfiEntityId, type_id: FfiComponentTypeId, out_len: &mut u32) -> *mut u8,
    /// Write component data from a byte slice.
    pub component_set_ptr:
        extern "C" fn(entity: FfiEntityId, type_id: FfiComponentTypeId, data: *const u8, len: u32) -> bool,

    // ── Coroutines ──────────────────────────────────────────────────────
    /// Start a coroutine from an opaque enumerator pointer.
    pub coroutine_start: extern "C" fn(enumerator_ptr: *mut std::ffi::c_void) -> FfiCoroutineHandle,
    /// Cancel a running coroutine.
    pub coroutine_cancel: extern "C" fn(handle: FfiCoroutineHandle),
    /// Advance a coroutine and write the next yield instruction.
    pub coroutine_move_next:
        extern "C" fn(enumerator_ptr: *mut std::ffi::c_void, instruction_out: &mut FfiYieldInstruction) -> bool,

    // ── Async I/O ───────────────────────────────────────────────────────
    /// Check whether an async operation has completed.
    pub async_is_complete: extern "C" fn(handle: FfiAsyncHandle) -> bool,
    /// Begin an async image load.
    pub async_load_image:
        extern "C" fn(url: *const std::ffi::c_char, callback: FfiAsyncCallback, user_data: u64) -> FfiAsyncHandle,
    /// Begin an async HTTP GET.
    pub async_http_get:
        extern "C" fn(url: *const std::ffi::c_char, callback: FfiAsyncCallback, user_data: u64) -> FfiAsyncHandle,

    // ── Condition evaluation ────────────────────────────────────────────
    /// Evaluate a WaitUntil condition identified by `condition_id`.
    pub condition_check: extern "C" fn(condition_id: u64) -> bool,

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
