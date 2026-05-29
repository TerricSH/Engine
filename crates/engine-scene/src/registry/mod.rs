//! Component and asset type extension registries.
//!
//! This module provides plugin-style registration for ECS component types and
//! asset types.  Subsystems (physics, animation, UI, audio, …) can add their
//! own component or asset types without editing core `engine-scene` files.

mod component;
mod asset;

pub use component::*;
pub use asset::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::{Component, ComponentStorageDyn, SparseSet};

    // --- Dummy component for testing ---

    struct DummyComponent(u32);

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
                has_script_binding: false,
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

        // All six core components should be present.
        assert!(reg.is_registered("engine.name"));
        assert!(reg.is_registered("engine.transform"));
        assert!(reg.is_registered("engine.renderable"));
        assert!(reg.is_registered("engine.camera"));
        assert!(reg.is_registered("engine.light"));
        assert!(reg.is_registered("engine.bounds"));

        // They should appear in the expected order.
        let ids: Vec<&str> = reg.iter().map(|e| e.meta.type_id).collect();
        assert_eq!(ids, vec![
            "engine.name",
            "engine.transform",
            "engine.renderable",
            "engine.camera",
            "engine.light",
            "engine.bounds",
        ]);
    }

    #[test]
    fn component_registry_create_storages() {
        let mut reg = ComponentRegistry::new();
        reg.register_core();

        let storages = reg.create_storages();
        assert_eq!(storages.len(), 6);
        assert!(storages.contains_key("engine.name"));
        assert!(storages.contains_key("engine.transform"));
        assert!(storages.contains_key("engine.renderable"));
        assert!(storages.contains_key("engine.camera"));
        assert!(storages.contains_key("engine.light"));
        assert!(storages.contains_key("engine.bounds"));

        // Each storage should be empty.
        for (_, storage) in &storages {
            assert_eq!(storage.len(), 0);
        }
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
            cooker: Some(|source: &[u8], output: &mut Vec<u8>| -> Result<(), String> {
                // Passthrough cooker for testing.
                output.extend_from_slice(source);
                Ok(())
            }),
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
