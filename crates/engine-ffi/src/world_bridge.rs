//! Bridge between FFI entity/component callbacks and the engine's `World`.
//!
//! Stores a raw pointer to the engine's [`World`] so that the
//! `extern "C"` registry callbacks can operate on it without requiring
//! `engine-core` or `engine-scene` to be directly coupled to `engine-ffi`.
//!
//! # Safety
//!
//! The world pointer is set once during engine initialisation and MUST
//! remain valid for the lifetime of the process.  Access is serialised
//! through a `Mutex`.

use std::sync::Mutex;

use crate::types::FfiEntityId;

// ---------------------------------------------------------------------------
// Global world pointer
// ---------------------------------------------------------------------------

/// Wrapper to make `*mut c_void` `Send + Sync` for static storage.
/// The pointer is set once during engine init and read-only thereafter,
/// so concurrent access through `Mutex` is safe.
struct RawWorldPtr(*mut std::ffi::c_void);

// SAFETY: the pointer is set once during engine init and only read
// (through Mutex locking) afterwards.
unsafe impl Send for RawWorldPtr {}
unsafe impl Sync for RawWorldPtr {}

impl std::fmt::Debug for RawWorldPtr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RawWorldPtr").finish()
    }
}

/// Erased pointer to the engine's `World` instance.
static WORLD_PTR: std::sync::OnceLock<Mutex<RawWorldPtr>> =
    std::sync::OnceLock::new();

/// Set the engine world pointer (called from the engine runtime).
///
/// # Safety
///
/// `ptr` must point to a valid, fully-initialised `World` that outlives
/// any FFI callback invocation.  Call exactly once during engine startup.
pub unsafe fn set_world_ptr(ptr: *mut std::ffi::c_void) {
    WORLD_PTR
        .set(Mutex::new(RawWorldPtr(ptr)))
        .expect("WORLD_PTR already set");
}

/// Execute a closure with a mutable `&mut World` reference.
///
/// Returns `None` if the world pointer has not been set yet.
pub fn with_world_mut<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&mut engine_scene::World) -> R,
{
    let lock = WORLD_PTR.get()?;
    let guard = lock.lock().ok()?;
    let ptr = guard.0;
    if ptr.is_null() {
        return None;
    }
    // SAFETY: the caller guarantees `ptr` is valid and uniquely accessible.
    let world = unsafe { &mut *ptr.cast::<engine_scene::World>() };
    Some(f(world))
}

/// Execute a closure with a shared `&World` reference.
pub fn with_world<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&engine_scene::World) -> R,
{
    let lock = WORLD_PTR.get()?;
    let guard = lock.lock().ok()?;
    let ptr = guard.0;
    if ptr.is_null() {
        return None;
    }
    // SAFETY: the caller guarantees `ptr` is valid.
    let world = unsafe { &*ptr.cast::<engine_scene::World>() };
    Some(f(world))
}

// ---------------------------------------------------------------------------
// Registry callbacks
// ---------------------------------------------------------------------------

use crate::registry;
use crate::types::{FfiAsyncHandle, FfiAsyncCallback, FfiComponentTypeId, FfiCoroutineHandle, FfiYieldInstruction};
use engine_scene::Entity;

pub extern "C" fn entity_spawn() -> FfiEntityId {
    with_world_mut(|w| {
        let e = w.create_entity();
        FfiEntityId { index: e.index(), generation: e.generation() }
    })
    .unwrap_or(FfiEntityId::INVALID)
}

pub extern "C" fn entity_destroy(entity: FfiEntityId) -> bool {
    with_world_mut(|w| {
        let e = Entity::new(entity.index, entity.generation);
        w.destroy_entity(e)
    })
    .unwrap_or(false)
}

pub extern "C" fn entity_is_alive(entity: FfiEntityId) -> bool {
    with_world(|w| {
        let e = Entity::new(entity.index, entity.generation);
        w.is_alive(e)
    })
    .unwrap_or(entity.index != u32::MAX)
}

// ---------------------------------------------------------------------------
// Initialisation helper
// ---------------------------------------------------------------------------

/// Populate the global [`FfiRegistry`] with callbacks that talk to a `World`.
///
/// Called once from `EngineRuntime::new()`.  `world_ptr` can be null;
/// entity/component operations will return sentinel values until the
/// world is set via [`set_world_ptr`].
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn populate_registry(world_ptr: *mut std::ffi::c_void) {
    if !world_ptr.is_null() {
        // SAFETY: world_ptr is non-null; the caller guarantees it points to a
        // valid World that outlives all FFI callbacks.
        unsafe { set_world_ptr(world_ptr) };
    }

    // Stub coroutine/async callbacks — these need the ILRuntime bridge
    // to be fully wired.  For now they return sentinel values.
    extern "C" fn coroutine_start(_p: *mut std::ffi::c_void) -> FfiCoroutineHandle {
        tracing::debug!("ffi coroutine_start: ILRuntime wiring pending");
        FfiCoroutineHandle::INVALID
    }
    extern "C" fn coroutine_cancel(_h: FfiCoroutineHandle) {}
    extern "C" fn coroutine_move_next(_p: *mut std::ffi::c_void, _o: &mut FfiYieldInstruction) -> bool { false }
    extern "C" fn async_is_complete(_h: FfiAsyncHandle) -> bool { false }
    extern "C" fn condition_check(_id: u64) -> bool { false }
    extern "C" fn dispatch_callbacks() { crate::r#async::dispatch_main_thread_callbacks(); }

    extern "C" fn component_get_ptr(
        _entity: FfiEntityId, _type_id: FfiComponentTypeId, out_len: &mut u32,
    ) -> *mut u8 {
        *out_len = 0;
        std::ptr::null_mut()
    }
    extern "C" fn component_set_ptr(
        _entity: FfiEntityId, _type_id: FfiComponentTypeId, _data: *const u8, _len: u32,
    ) -> bool {
        false
    }

    extern "C" fn async_load_image(
        _url: *const std::ffi::c_char, _cb: FfiAsyncCallback, _ud: u64,
    ) -> FfiAsyncHandle {
        FfiAsyncHandle(0)
    }
    extern "C" fn async_http_get(
        _url: *const std::ffi::c_char, _cb: FfiAsyncCallback, _ud: u64,
    ) -> FfiAsyncHandle {
        FfiAsyncHandle(0)
    }

    let reg = registry::FfiRegistry {
        entity_spawn,
        entity_destroy,
        entity_is_alive,
        component_get_ptr,
        component_set_ptr,
        coroutine_start,
        coroutine_cancel,
        coroutine_move_next,
        async_is_complete,
        async_load_image,
        async_http_get,
        condition_check,
        dispatch_main_thread_callbacks: dispatch_callbacks,
    };

    registry::register(reg).ok();
    tracing::info!("FFI world bridge initialised");
}
