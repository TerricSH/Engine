use std::ffi::CString;

use engine_character::CharacterController;
use engine_ffi::component::{ffi_component_get, ffi_component_set, ffi_component_type_id};
use engine_ffi::types::FfiEntityId;
use engine_ffi::world_bridge::{activate_world, deactivate_world, populate_registry};
use engine_scene::{ComponentRegistry, World, WorldSlot};

#[test]
fn serializable_component_roundtrips_through_length_buffer_abi() {
    let mut component_registry = ComponentRegistry::new();
    engine_character::register_character_extensions(&mut component_registry, None);

    let mut world = World::new();
    world.set_component_registry(component_registry.clone());
    let entity = world.create_entity();
    world.add_component(entity, CharacterController::new());

    let slot = WorldSlot::new();
    slot.replace(world);
    populate_registry();
    activate_world(&slot, &component_registry);

    let canonical_name = CString::new("engine.character_controller").unwrap();
    let type_id = unsafe { ffi_component_type_id(canonical_name.as_ptr()) };
    assert_ne!(type_id.0, 0);

    let ffi_entity = FfiEntityId {
        index: entity.index(),
        generation: entity.generation(),
    };
    let mut required = 0_u32;
    assert!(!unsafe {
        ffi_component_get(ffi_entity, type_id, std::ptr::null_mut(), 0, &mut required)
    });
    assert!(required > 0);

    let mut json = vec![0_u8; required as usize];
    let mut written = 0_u32;
    assert!(unsafe {
        ffi_component_get(
            ffi_entity,
            type_id,
            json.as_mut_ptr(),
            json.len() as u32,
            &mut written,
        )
    });
    let fields: serde_json::Value = serde_json::from_slice(&json[..written as usize]).unwrap();
    assert!(fields.get("height").is_some());

    let replacement = br#"{"height":{"Float32":3.25}}"#;
    assert!(unsafe {
        ffi_component_set(
            ffi_entity,
            type_id,
            replacement.as_ptr(),
            replacement.len() as u32,
        )
    });
    assert_eq!(
        slot.with_world(|world| world.get::<CharacterController>(entity).unwrap().height),
        Some(3.25)
    );

    assert!(deactivate_world(&slot));
}
