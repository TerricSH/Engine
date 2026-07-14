use std::ffi::CString;
use std::sync::atomic::{AtomicUsize, Ordering};

use engine_character::CharacterController;
use engine_ffi::types::{
    FfiComponentTypeId, FfiCoroutineHandle, FfiEntityId, FfiManagedCoroutineDescriptor,
    FfiYieldInstruction, FFI_COROUTINE_MOVE_COMPLETED, FFI_COROUTINE_MOVE_YIELDED,
    FFI_COROUTINE_READY, FFI_MANAGED_COROUTINE_ABI_VERSION,
};
use engine_ffi::world_bridge::{activate_world, deactivate_world, populate_registry};
use engine_scene::{ComponentRegistry, World, WorldSlot};
use libloading::Library;

type EntitySpawnFn = unsafe extern "C" fn() -> FfiEntityId;
type ComponentTypeIdFn = unsafe extern "C" fn(*const std::ffi::c_char) -> FfiComponentTypeId;
type ComponentGetFn =
    unsafe extern "C" fn(FfiEntityId, FfiComponentTypeId, *mut u8, u32, *mut u32) -> bool;
type ComponentSetFn = unsafe extern "C" fn(FfiEntityId, FfiComponentTypeId, *const u8, u32) -> bool;
type CoroutineStartFn =
    unsafe extern "C" fn(*const FfiManagedCoroutineDescriptor) -> FfiCoroutineHandle;

static COROUTINE_MOVES: AtomicUsize = AtomicUsize::new(0);
static COROUTINE_RELEASES: AtomicUsize = AtomicUsize::new(0);

extern "C" fn coroutine_move_next(
    _context: *mut std::ffi::c_void,
    instruction: *mut FfiYieldInstruction,
) -> u32 {
    if COROUTINE_MOVES.fetch_add(1, Ordering::SeqCst) == 0 {
        if !instruction.is_null() {
            // SAFETY: The scheduler supplies a writable output pointer.
            unsafe { *instruction = FfiYieldInstruction::next_frame() };
        }
        FFI_COROUTINE_MOVE_YIELDED
    } else {
        FFI_COROUTINE_MOVE_COMPLETED
    }
}

extern "C" fn coroutine_ready(
    _context: *mut std::ffi::c_void,
    _token: u64,
    _delta_seconds: f32,
) -> u32 {
    FFI_COROUTINE_READY
}

extern "C" fn coroutine_release(context: *mut std::ffi::c_void) {
    // SAFETY: The descriptor transfers this allocation to the scheduler.
    drop(unsafe { Box::from_raw(context.cast::<u8>()) });
    COROUTINE_RELEASES.fetch_add(1, Ordering::SeqCst);
}

#[test]
fn dynamically_loaded_cdylib_roundtrips_the_active_world() {
    let mut component_registry = ComponentRegistry::new();
    engine_character::register_character_extensions(&mut component_registry, None);

    let mut world = World::new();
    world.set_component_registry(component_registry.clone());
    let slot = WorldSlot::new();
    slot.replace(world);

    populate_registry();
    activate_world(&slot, &component_registry);
    engine_ffi::host_bridge::install_cdylib_registry(engine_ffi::registry::get())
        .expect("install host callbacks into the C# native library");
    let path = engine_ffi::host_bridge::loaded_cdylib_path().expect("retained cdylib path");

    // SAFETY: `path` is the library retained and ABI-validated by the host
    // bridge. Each resolved symbol uses the corresponding exported C ABI and
    // remains valid while both this handle and the retained host handle live.
    unsafe {
        let library = Library::new(path).expect("reopen installed engine_ffi cdylib");
        let spawn = library
            .get::<EntitySpawnFn>(b"ffi_entity_spawn\0")
            .expect("ffi_entity_spawn export");
        let component_type_id = library
            .get::<ComponentTypeIdFn>(b"ffi_component_type_id\0")
            .expect("ffi_component_type_id export");
        let component_get = library
            .get::<ComponentGetFn>(b"ffi_component_get\0")
            .expect("ffi_component_get export");
        let component_set = library
            .get::<ComponentSetFn>(b"ffi_component_set\0")
            .expect("ffi_component_set export");
        let coroutine_start = library
            .get::<CoroutineStartFn>(b"ffi_coroutine_start\0")
            .expect("ffi_coroutine_start export");

        let entity = spawn();
        assert_ne!(entity, FfiEntityId::INVALID);
        let canonical_name = CString::new("engine.character_controller").unwrap();
        let type_id = component_type_id(canonical_name.as_ptr());
        assert_ne!(type_id, FfiComponentTypeId::INVALID);

        let replacement = br#"{"height":{"Float32":4.5},"foot_ik_enabled":{"Bool":false}}"#;
        assert!(component_set(
            entity,
            type_id,
            replacement.as_ptr(),
            replacement.len() as u32,
        ));

        let entity = engine_scene::Entity::new(entity.index, entity.generation);
        assert_eq!(
            slot.with_world(|world| {
                let controller = world
                    .get::<CharacterController>(entity)
                    .expect("component inserted through cdylib callback");
                (controller.height, controller.foot_ik_enabled)
            }),
            Some((4.5, false))
        );

        let mut required = 0_u32;
        assert!(!component_get(
            FfiEntityId {
                index: entity.index(),
                generation: entity.generation(),
            },
            type_id,
            std::ptr::null_mut(),
            0,
            &mut required,
        ));
        assert!(required > 0);
        let mut json = vec![0_u8; required as usize];
        assert!(component_get(
            FfiEntityId {
                index: entity.index(),
                generation: entity.generation(),
            },
            type_id,
            json.as_mut_ptr(),
            json.len() as u32,
            &mut required,
        ));
        let fields: serde_json::Value = serde_json::from_slice(&json[..required as usize]).unwrap();
        assert_eq!(fields["height"]["Float32"], 4.5);
        assert_eq!(fields["foot_ik_enabled"]["Bool"], false);

        COROUTINE_MOVES.store(0, Ordering::SeqCst);
        COROUTINE_RELEASES.store(0, Ordering::SeqCst);
        let descriptor = FfiManagedCoroutineDescriptor {
            abi_version: FFI_MANAGED_COROUTINE_ABI_VERSION,
            struct_size: std::mem::size_of::<FfiManagedCoroutineDescriptor>() as u32,
            context: Box::into_raw(Box::new(0_u8)).cast(),
            move_next: Some(coroutine_move_next),
            readiness: Some(coroutine_ready),
            release: Some(coroutine_release),
        };
        let coroutine = coroutine_start(&descriptor);
        assert_ne!(coroutine, FfiCoroutineHandle::INVALID);
        engine_ffi::coroutine::tick_managed_coroutines(0.016);
        engine_ffi::coroutine::tick_managed_coroutines(0.016);
        assert_eq!(COROUTINE_MOVES.load(Ordering::SeqCst), 2);
        assert_eq!(COROUTINE_RELEASES.load(Ordering::SeqCst), 1);
    }

    assert!(deactivate_world(&slot));
}
