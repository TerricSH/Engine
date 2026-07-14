//! Bridge between FFI entity/component callbacks and the engine's `World`.
//!
//! Stores a weak reference to the engine's shared world slot so that the
//! `extern "C"` registry callbacks can operate on it without requiring
//! `engine-core` or `engine-scene` to be directly coupled to `engine-ffi`.

use std::sync::{LazyLock, Mutex, Once};

use crate::types::FfiEntityId;
use engine_scene::{ComponentRegistry, Entity, WeakWorldSlot, WorldSlot};

// ---------------------------------------------------------------------------
// Global active world slot
// ---------------------------------------------------------------------------

/// Non-owning reference to the most recently activated runtime world.
///
/// FFI calls temporarily upgrade this weak reference, keeping the slot alive
/// for the complete callback without extending a runtime's normal lifetime.
static ACTIVE_WORLD: LazyLock<Mutex<Option<WeakWorldSlot>>> = LazyLock::new(|| Mutex::new(None));
static ACTIVE_COROUTINE_RUNTIME: LazyLock<Mutex<Option<WeakWorldSlot>>> =
    LazyLock::new(|| Mutex::new(None));

fn lock_active_world() -> std::sync::MutexGuard<'static, Option<WeakWorldSlot>> {
    ACTIVE_WORLD
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn active_world_slot() -> Option<WorldSlot> {
    let weak = lock_active_world().as_ref()?.clone();
    weak.upgrade()
}

fn lock_coroutine_runtime() -> std::sync::MutexGuard<'static, Option<WeakWorldSlot>> {
    ACTIVE_COROUTINE_RUNTIME
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Select the runtime whose main-thread tick owns managed coroutines.
/// Switching runtimes releases every coroutine belonging to the old owner.
pub fn activate_coroutine_runtime(slot: &WorldSlot) {
    let changed = {
        let mut active = lock_coroutine_runtime();
        if active.as_ref().is_some_and(|weak| weak.ptr_eq_slot(slot)) {
            false
        } else {
            *active = Some(slot.downgrade());
            true
        }
    };
    if changed {
        crate::coroutine::clear_managed_coroutines();
    }
}

/// Release the managed coroutines owned by `slot` without affecting a newer
/// runtime that has since become active.
pub fn deactivate_coroutine_runtime(slot: &WorldSlot) -> bool {
    let deactivated = {
        let mut active = lock_coroutine_runtime();
        if active.as_ref().is_some_and(|weak| weak.ptr_eq_slot(slot)) {
            *active = None;
            true
        } else {
            false
        }
    };
    if deactivated {
        crate::coroutine::clear_managed_coroutines();
    }
    deactivated
}

/// Select `slot` and release its previous scene's managed coroutines.
pub fn reset_coroutine_runtime(slot: &WorldSlot) {
    {
        let mut active = lock_coroutine_runtime();
        *active = Some(slot.downgrade());
    }
    crate::coroutine::clear_managed_coroutines();
}

/// Make `slot` and its runtime component registry the process-wide FFI
/// binding.
///
/// Repeated activation is supported. The most recently activated live slot
/// wins, matching the existing single-active-runtime FFI contract. The active
/// type table is replaced while holding the same activation lock, so callers
/// observe the old world/type pair or the new pair, never a mixture.
pub fn activate_world(slot: &WorldSlot, component_registry: &ComponentRegistry) {
    // Coroutines hold managed objects associated with the previously active
    // scene/runtime. Release them before switching the process-wide binding.
    reset_coroutine_runtime(slot);
    let entries = component_registry
        .iter()
        .filter(|extension| {
            extension.meta.has_script_binding
                && extension.serialize.is_some()
                && extension.deserialize.is_some()
        })
        .map(|extension| (extension.meta.display_name, extension.meta.type_id))
        .collect::<Vec<_>>();

    let mut active = lock_active_world();
    crate::component::replace_component_types(&entries);
    *active = Some(slot.downgrade());
}

/// Stop routing FFI calls to `slot` when it is still the active world.
///
/// Returns `false` when another runtime has become active in the meantime.
pub fn deactivate_world(slot: &WorldSlot) -> bool {
    let deactivated = {
        let mut active = lock_active_world();
        if active.as_ref().is_some_and(|weak| weak.ptr_eq_slot(slot)) {
            crate::component::clear_component_types();
            *active = None;
            true
        } else {
            false
        }
    };
    if deactivated {
        deactivate_coroutine_runtime(slot);
    }
    deactivated
}

/// Execute a closure with a mutable `&mut World` reference.
///
/// Returns `None` if the world pointer has not been set yet.
pub fn with_world_mut<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&mut engine_scene::World) -> R,
{
    active_world_slot()?.with_world_mut(f)
}

/// Execute a closure with a shared `&World` reference.
pub fn with_world<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&engine_scene::World) -> R,
{
    active_world_slot()?.with_world(f)
}

/// Snapshot an active world slot and one component type mapping under the
/// activation lock. The returned strong slot keeps that exact world binding
/// alive even if another runtime activates immediately afterwards.
fn active_world_component(
    type_id: crate::types::FfiComponentTypeId,
) -> Option<(WorldSlot, &'static str)> {
    let active = lock_active_world();
    let weak = active.as_ref()?.clone();
    let engine_type_id = crate::component::lookup_engine_type_id(type_id)?;
    Some((weak.upgrade()?, engine_type_id))
}

// ---------------------------------------------------------------------------
// Registry callbacks
// ---------------------------------------------------------------------------

use crate::registry;
use crate::types::{
    FfiAsyncCallback, FfiAsyncHandle, FfiComponentTypeId, FfiCoroutineHandle,
    FfiManagedCoroutineDescriptor,
};
pub extern "C" fn entity_spawn() -> FfiEntityId {
    with_world_mut(|w| {
        let e = w.create_entity();
        FfiEntityId {
            index: e.index(),
            generation: e.generation(),
        }
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
    .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Initialisation helper
// ---------------------------------------------------------------------------

/// Populate the global [`FfiRegistry`] with callbacks that talk to a `World`.
///
/// Called from `EngineRuntime::new()`. Repeated calls are idempotent; entity
/// and component operations return sentinel values until a world is activated.
pub fn populate_registry() {
    static POPULATE: Once = Once::new();
    POPULATE.call_once(populate_registry_once);
}

fn populate_registry_once() {
    unsafe extern "C" fn coroutine_start(
        descriptor: *const FfiManagedCoroutineDescriptor,
    ) -> FfiCoroutineHandle {
        // SAFETY: Ownership and pointer validity follow the exported start ABI.
        unsafe { crate::coroutine::schedule_managed_coroutine(descriptor) }
    }
    extern "C" fn coroutine_cancel(handle: FfiCoroutineHandle) {
        crate::coroutine::cancel_managed_coroutine(handle);
    }
    extern "C" fn async_is_complete(handle: FfiAsyncHandle) -> bool {
        crate::r#async::host_async_is_complete(handle)
    }
    extern "C" fn dispatch_callbacks() {
        crate::r#async::dispatch_main_thread_callbacks();
    }

    // Thread-local buffer for FFI component data transfer.
    // Reused across calls. Only one FFI call executes per thread at a time.
    std::thread_local! {
        static FFI_BUF: std::cell::RefCell<Vec<u8>> = const { std::cell::RefCell::new(Vec::new()) };
    }

    extern "C" fn component_type_id(name: *const std::ffi::c_char) -> FfiComponentTypeId {
        // SAFETY: This callback has the same C-string contract as the exported
        // lookup function. It deliberately bypasses registry routing to avoid
        // recursively calling itself in the host process.
        unsafe { crate::component::lookup_component_type_ptr(name) }
    }

    extern "C" fn component_type_count() -> u32 {
        crate::component::component_type_count()
    }

    extern "C" fn component_get_ptr(
        entity: FfiEntityId,
        type_id: FfiComponentTypeId,
        out_len: &mut u32,
    ) -> *mut u8 {
        let (world_slot, engine_type_id) = match active_world_component(type_id) {
            Some(binding) => binding,
            None => {
                *out_len = 0;
                return std::ptr::null_mut();
            }
        };

        let json = world_slot.with_world(|world| {
            let e = Entity::new(entity.index, entity.generation);
            if !world.is_alive(e) {
                return None;
            }
            world.serialize_component(e, engine_type_id)
        });

        match json.flatten() {
            Some(s) => {
                let bytes = s.into_bytes();
                let len = bytes.len();
                FFI_BUF.with(|buf| {
                    *buf.borrow_mut() = bytes;
                });
                // SAFETY: FFI_BUF lives for the rest of this call; the C# caller
                // must copy the data before calling any other FFI function.
                let ptr = FFI_BUF.with(|buf| buf.borrow().as_ptr()) as *mut u8;
                *out_len = len as u32;
                ptr
            }
            None => {
                *out_len = 0;
                std::ptr::null_mut()
            }
        }
    }

    extern "C" fn component_set_ptr(
        entity: FfiEntityId,
        type_id: FfiComponentTypeId,
        data: *const u8,
        len: u32,
    ) -> bool {
        let (world_slot, engine_type_id) = match active_world_component(type_id) {
            Some(binding) => binding,
            None => return false,
        };
        if data.is_null() || len == 0 {
            return false;
        }
        // SAFETY: caller guarantees data points to valid memory of at least len bytes.
        let json = unsafe {
            let slice = std::slice::from_raw_parts(data, len as usize);
            std::str::from_utf8(slice).unwrap_or("")
        };
        if json.is_empty() {
            return false;
        }

        world_slot
            .with_world_mut(|world| {
                let e = Entity::new(entity.index, entity.generation);
                if !world.is_alive(e) {
                    return false;
                }
                world.deserialize_component(e, engine_type_id, json)
            })
            .unwrap_or(false)
    }

    extern "C" fn async_load_image(
        url: *const std::ffi::c_char,
        callback: FfiAsyncCallback,
        user_data: u64,
    ) -> FfiAsyncHandle {
        crate::r#async::host_async_load_image(url, callback, user_data)
    }
    extern "C" fn async_http_get(
        url: *const std::ffi::c_char,
        callback: FfiAsyncCallback,
        user_data: u64,
    ) -> FfiAsyncHandle {
        crate::r#async::host_async_http_get(url, callback, user_data)
    }

    let reg = registry::FfiRegistry {
        entity_spawn,
        entity_destroy,
        entity_is_alive,
        component_type_id,
        component_type_count,
        component_get_ptr,
        component_set_ptr,
        coroutine_start,
        coroutine_cancel,
        async_is_complete,
        async_load_image,
        async_http_get,
        dispatch_main_thread_callbacks: dispatch_callbacks,
    };

    registry::register(reg).ok();
    tracing::info!("FFI world bridge initialised");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn serial_test() -> std::sync::MutexGuard<'static, ()> {
        crate::component::lock_component_registry_for_test()
    }

    #[test]
    fn activated_world_receives_ffi_entity_operations() {
        let _guard = serial_test();
        let slot = WorldSlot::new();
        slot.replace(engine_scene::World::new());
        activate_world(&slot, &ComponentRegistry::new());

        let entity = entity_spawn();
        assert_ne!(entity, FfiEntityId::INVALID);
        assert_eq!(slot.with_world(engine_scene::World::alive_count), Some(1));

        assert!(deactivate_world(&slot));
    }

    #[test]
    fn replacing_world_in_active_slot_routes_to_new_world() {
        let _guard = serial_test();
        let slot = WorldSlot::new();
        let mut first = engine_scene::World::new();
        first.create_entity();
        slot.replace(first);
        activate_world(&slot, &ComponentRegistry::new());

        slot.replace(engine_scene::World::new());
        let entity = entity_spawn();
        assert_ne!(entity, FfiEntityId::INVALID);
        assert_eq!(slot.with_world(engine_scene::World::alive_count), Some(1));

        assert!(deactivate_world(&slot));
    }

    #[test]
    fn deactivating_an_old_runtime_does_not_clear_the_new_one() {
        let _guard = serial_test();
        let old = WorldSlot::new();
        old.replace(engine_scene::World::new());
        let current = WorldSlot::new();
        current.replace(engine_scene::World::new());

        activate_world(&old, &ComponentRegistry::new());
        activate_world(&current, &ComponentRegistry::new());
        assert!(!deactivate_world(&old));

        assert_ne!(entity_spawn(), FfiEntityId::INVALID);
        assert_eq!(
            current.with_world(engine_scene::World::alive_count),
            Some(1)
        );
        assert!(deactivate_world(&current));
    }

    #[test]
    fn coroutine_owner_cleanup_does_not_affect_a_newer_runtime() {
        let _guard = serial_test();
        let old = WorldSlot::new();
        let current = WorldSlot::new();

        activate_coroutine_runtime(&old);
        activate_coroutine_runtime(&current);
        assert!(!deactivate_coroutine_runtime(&old));
        assert!(deactivate_coroutine_runtime(&current));
    }

    #[test]
    fn dropping_active_slot_makes_ffi_world_unavailable() {
        let _guard = serial_test();
        let slot = WorldSlot::new();
        slot.replace(engine_scene::World::new());
        activate_world(&slot, &ComponentRegistry::new());
        drop(slot);

        assert_eq!(entity_spawn(), FfiEntityId::INVALID);
        *lock_active_world() = None;
    }
}
