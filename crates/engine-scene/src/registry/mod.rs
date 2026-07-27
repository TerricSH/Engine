//! Component and asset type extension registries.
//!
//! This module provides plugin-style registration for ECS component types and
//! asset types.  Subsystems (physics, animation, UI, audio, …) can add their
//! own component or asset types without editing core `engine-scene` files.

mod asset;
mod component;

pub use asset::*;
pub use component::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::{Component, ComponentStorageDyn, SparseSet};

    // --- Dummy component for testing ---

    struct DummyComponent;

    impl Component for DummyComponent {
        const TYPE_ID: &'static str = "test.dummy";
    }

    fn dummy_storage_factory() -> Box<dyn ComponentStorageDyn> {
        Box::new(SparseSet::<DummyComponent>::new())
    }

    fn make_dummy_extension(display_name: &'static str) -> ComponentExtension {
        ComponentExtension {
            meta: ComponentMeta {
                type_id: "test.dummy",
                display_name,
                schema_version: (0, 1, 0),
                has_editor: false,
                script_access: ScriptAccess::None,
            },
            storage_factory: dummy_storage_factory,
            serialize: None,
            deserialize: None,
        }
    }

    // ---------------------------------------------------------------
    // ComponentRegistry tests
    // ---------------------------------------------------------------

    #[test]
    fn component_registry_new_is_empty() {
        let reg = ComponentRegistry::new();
        assert!(reg.iter().next().is_none());
        assert_eq!(reg.iter().count(), 0);
    }

    #[test]
    fn component_registry_register_and_get() {
        let mut reg = ComponentRegistry::new();
        let ext = make_dummy_extension("Dummy");
        assert!(reg.register(ext).is_ok());

        let retrieved = reg.get("test.dummy");
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().meta.display_name, "Dummy");
    }

    #[test]
    fn component_registry_prevent_duplicate() {
        let mut reg = ComponentRegistry::new();
        let ext1 = make_dummy_extension("Dummy");
        assert!(reg.register(ext1).is_ok());

        let ext2 = make_dummy_extension("Dummy Duplicate");
        let result = reg.register(ext2);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "test.dummy");
    }

    #[test]
    fn component_registry_register_core() {
        let mut reg = ComponentRegistry::new();
        reg.register_core();

        // All core components should be present.
        assert!(reg.is_registered("engine.name"));
        assert!(reg.is_registered("engine.transform"));
        assert!(reg.is_registered("engine.renderable"));
        assert!(reg.is_registered("engine.camera"));
        assert!(reg.is_registered("engine.light"));
        assert!(reg.is_registered("engine.interactable"));
        assert!(reg.is_registered("engine.lod_group"));
        assert!(reg.is_registered("engine.hlod_cluster"));
        assert!(reg.is_registered("engine.bounds"));
        assert!(reg.is_registered("engine.prefab_instance_ref"));

        // They should appear in the expected order.
        let ids: Vec<&str> = reg.iter().map(|e| e.meta.type_id).collect();
        assert_eq!(
            ids,
            vec![
                "engine.name",
                "engine.transform",
                "engine.renderable",
                "engine.camera",
                "engine.light",
                "engine.interactable",
                "engine.lod_group",
                "engine.hlod_cluster",
                "engine.bounds",
                "engine.prefab_instance_ref",
            ]
        );
    }

    #[test]
    fn component_registry_create_storages() {
        let mut reg = ComponentRegistry::new();
        reg.register_core();

        let storages = reg.create_storages();
        assert_eq!(storages.len(), 10);
        assert!(storages.contains_key("engine.name"));
        assert!(storages.contains_key("engine.transform"));
        assert!(storages.contains_key("engine.renderable"));
        assert!(storages.contains_key("engine.camera"));
        assert!(storages.contains_key("engine.light"));
        assert!(storages.contains_key("engine.interactable"));
        assert!(storages.contains_key("engine.lod_group"));
        assert!(storages.contains_key("engine.hlod_cluster"));
        assert!(storages.contains_key("engine.bounds"));
        assert!(storages.contains_key("engine.prefab_instance_ref"));

        // Each storage should be empty.
        for storage in storages.values() {
            assert_eq!(storage.len(), 0);
        }
    }

    #[test]
    fn component_registry_core_camera_and_light_have_serde_hooks() {
        let mut reg = ComponentRegistry::new();
        reg.register_core();

        for type_id in ["engine.camera", "engine.light", "engine.interactable"] {
            let ext = reg.get(type_id).expect("core component registered");
            assert!(ext.serialize.is_some(), "{type_id} needs a serialize hook");
            assert!(
                ext.deserialize.is_some(),
                "{type_id} needs a deserialize hook"
            );
            assert!(
                ext.meta.has_script_binding(),
                "{type_id} opts into script binding"
            );
        }
    }

    #[test]
    fn camera_and_light_field_serde_roundtrips_through_hooks() {
        use crate::components::{
            deserialize_camera, deserialize_light, serialize_camera, serialize_camera_fields,
            serialize_light, serialize_light_fields, Camera, Light,
        };
        use std::collections::BTreeMap;

        let camera = Camera {
            viewport_rect: Some([0.1, 0.2, 0.5, 0.5]),
            ..Camera::default()
        };
        let camera_fields = serialize_camera(&camera);
        let camera_restored = deserialize_camera(&camera_fields);
        let camera_restored = camera_restored
            .downcast_ref::<Camera>()
            .expect("camera hook restores a Camera");
        assert_eq!(
            serialize_camera_fields(camera_restored),
            serialize_camera_fields(&camera)
        );
        assert_eq!(camera_restored.viewport_rect, camera.viewport_rect);

        let light = Light {
            kind: crate::components::LightKind::Spot,
            color: [0.5, 0.25, 0.75],
            intensity: 3.5,
            range: 22.0,
            spot_angles: Some([0.3, 0.6]),
            shadow_mode: 2,
            direction: [0.0, -1.0, 0.25],
        };
        let light_fields = serialize_light(&light);
        let light_restored = deserialize_light(&light_fields);
        let light_restored = light_restored
            .downcast_ref::<Light>()
            .expect("light hook restores a Light");
        assert_eq!(
            serialize_light_fields(light_restored),
            serialize_light_fields(&light)
        );

        // Missing fields fall back to the authored defaults, matching the
        // scene loader's tolerance for older scene files.
        let defaulted = deserialize_light(&BTreeMap::new());
        let defaulted = defaulted.downcast_ref::<Light>().expect("default light");
        assert_eq!(defaulted.intensity, 1.0);
        assert_eq!(defaulted.range, 10.0);
        assert!(defaulted.spot_angles.is_none());
    }

    #[test]
    fn component_registry_is_cloneable_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<ComponentRegistry>();

        let mut registry = ComponentRegistry::new();
        registry
            .register(make_dummy_extension("Dummy"))
            .expect("register dummy component");
        let cloned = registry.clone();
        assert_eq!(cloned.get("test.dummy").unwrap().meta.display_name, "Dummy");
    }

    // ---------------------------------------------------------------
    // AssetTypeRegistry tests
    // ---------------------------------------------------------------

    #[test]
    fn asset_type_registry_new_is_empty() {
        let reg = AssetTypeRegistry::new();
        assert!(reg.get("mesh").is_none());
        assert!(reg.cooker_for("glb").is_none());
    }

    #[test]
    fn asset_type_registry_register_and_get() {
        let mut reg = AssetTypeRegistry::new();

        let mesh_ext = AssetTypeExtension {
            meta: AssetTypeMeta {
                type_id: "mesh",
                source_extensions: vec!["glb", "gltf"],
                display_name: "Mesh",
            },
            cooker: Some(
                |source: &[u8], output: &mut Vec<u8>| -> Result<(), String> {
                    // Passthrough cooker for testing.
                    output.extend_from_slice(source);
                    Ok(())
                },
            ),
            loader: None,
        };

        assert!(reg.register(mesh_ext).is_ok());
        assert!(reg.get("mesh").is_some());

        // cooker_for should match by extension.
        assert!(reg.cooker_for("glb").is_some());
        assert!(reg.cooker_for("gltf").is_some());
        assert!(reg.cooker_for("png").is_none());
    }

    #[test]
    fn asset_type_registry_prevent_duplicate() {
        let mut reg = AssetTypeRegistry::new();

        let ext1 = AssetTypeExtension {
            meta: AssetTypeMeta {
                type_id: "audio",
                source_extensions: vec!["wav"],
                display_name: "Audio",
            },
            cooker: None,
            loader: None,
        };
        assert!(reg.register(ext1).is_ok());

        let ext2 = AssetTypeExtension {
            meta: AssetTypeMeta {
                type_id: "audio",
                source_extensions: vec!["ogg"],
                display_name: "Audio",
            },
            cooker: None,
            loader: None,
        };
        let result = reg.register(ext2);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "audio");
    }
}
