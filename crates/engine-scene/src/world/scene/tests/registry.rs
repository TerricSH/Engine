    #[test]
    fn strict_registry_load_rejects_missing_deserialize_hook() {
        let scene = scene_with_component(ExternalComponent::TYPE_ID);
        let mut registry = ComponentRegistry::new();
        registry
            .register(external_extension(None))
            .expect("register external component");
        let result = World::try_from_scene_with_registry(&scene, Arc::new(registry));
        let error = match result {
            Ok(_) => panic!("missing deserialize hook must fail strict loading"),
            Err(error) => error,
        };
        assert!(matches!(
            error.diagnostics.as_slice(),
            [SceneLoadDiagnostic::MissingDeserializeHook {
                component_type_id,
                ..
            }] if component_type_id == ExternalComponent::TYPE_ID
        ));
    }

    #[test]
    fn strict_registry_load_rejects_storage_insert_type_mismatch() {
        let scene = scene_with_component(ExternalComponent::TYPE_ID);
        let mut registry = ComponentRegistry::new();
        registry
            .register(external_extension(Some(deserialize_wrong_type)))
            .expect("register external component");
        let result = World::try_from_scene_with_registry(&scene, Arc::new(registry));
        let error = match result {
            Ok(_) => panic!("type-erased storage mismatch must fail strict loading"),
            Err(error) => error,
        };
        assert!(matches!(
            error.diagnostics.as_slice(),
            [SceneLoadDiagnostic::StorageInsertTypeMismatch {
                component_type_id,
                ..
            }] if component_type_id == ExternalComponent::TYPE_ID
        ));
    }

    #[test]
    fn strict_registry_load_rejects_storage_factory_type_mismatch_before_traversal() {
        let scene = scene_with_component(ExternalComponent::TYPE_ID);
        let mut registry = ComponentRegistry::new();
        let mut extension = external_extension(Some(deserialize_external));
        extension.storage_factory = wrong_storage;
        registry
            .register(extension)
            .expect("register external component");
        let result = World::try_from_scene_with_registry(&scene, Arc::new(registry));
        let error = match result {
            Ok(_) => panic!("mismatched storage factory must fail strict loading"),
            Err(error) => error,
        };
        assert!(matches!(
            error.diagnostics.as_slice(),
            [SceneLoadDiagnostic::StorageFactoryTypeMismatch {
                component_type_id,
                storage_type_id,
                ..
            }] if component_type_id == ExternalComponent::TYPE_ID
                && storage_type_id == WrongComponent::TYPE_ID
        ));
    }

    #[test]
    fn disabled_external_component_is_validated_but_not_instantiated() {
        let mut scene = scene_with_component(ExternalComponent::TYPE_ID);
        let original = scene.entities[0]
            .components
            .get_mut(ExternalComponent::TYPE_ID)
            .expect("external component");
        original.enabled = false;
        original.schema_version = SchemaVersion::new(2, 3, 4);
        let original = original.clone();

        let mut registry = ComponentRegistry::new();
        registry
            .register(external_extension(Some(deserialize_external)))
            .expect("register external component");
        let world = World::try_from_scene_with_registry(&scene, Arc::new(registry))
            .expect("known disabled component should validate");

        assert!(world.query::<ExternalComponent>().next().is_none());
        let roundtripped = world.to_scene();
        assert_eq!(
            roundtripped.entities[0]
                .components
                .get(ExternalComponent::TYPE_ID),
            Some(&original)
        );
    }

    #[test]
    fn strict_load_rejects_unknown_disabled_component() {
        let mut scene = scene_with_component("test.disabled_unknown");
        scene.entities[0]
            .components
            .get_mut("test.disabled_unknown")
            .expect("unknown component")
            .enabled = false;

        let result =
            World::try_from_scene_with_registry(&scene, Arc::new(ComponentRegistry::new()));
        let error = match result {
            Ok(_) => panic!("unknown disabled component must fail strict loading"),
            Err(error) => error,
        };
        assert!(matches!(
            error.diagnostics.as_slice(),
            [SceneLoadDiagnostic::UnknownComponent {
                component_type_id,
                ..
            }] if component_type_id == "test.disabled_unknown"
        ));
    }

    #[test]
    fn non_strict_roundtrip_preserves_unknown_disabled_component() {
        let mut scene = scene_with_component("test.disabled_unknown");
        let original = scene.entities[0]
            .components
            .get_mut("test.disabled_unknown")
            .expect("unknown component");
        original.enabled = false;
        original.schema_version = SchemaVersion::new(7, 8, 9);
        let original = original.clone();

        let roundtripped = World::from_scene(&scene).to_scene();
        assert_eq!(
            roundtripped.entities[0]
                .components
                .get("test.disabled_unknown"),
            Some(&original)
        );
    }

    #[test]
    fn roundtrip_preserves_scene_metadata_and_enabled_component_schema() {
        let mut scene = sample_scene();
        scene.schema_version = SchemaVersion::new(0, 9, 7);
        scene.engine_version = "9.8.7-test".to_string();
        scene.diagnostics_policy = crate::scene::DiagnosticsPolicy::EditorRepair;
        scene.dependencies = vec![AssetId::new("kept-explicit-dependency")];
        let renderable = scene.entities[1]
            .components
            .get_mut(Renderable::TYPE_ID)
            .expect("renderable component");
        renderable.schema_version = SchemaVersion::new(3, 2, 1);

        let roundtripped = World::from_scene(&scene).to_scene();
        assert_eq!(roundtripped.schema_version, scene.schema_version);
        assert_eq!(roundtripped.engine_version, scene.engine_version);
        assert_eq!(roundtripped.dependencies, scene.dependencies);
        assert_eq!(roundtripped.diagnostics_policy, scene.diagnostics_policy);
        assert_eq!(
            roundtripped.entities[1].components[Renderable::TYPE_ID].schema_version,
            SchemaVersion::new(3, 2, 1)
        );
    }
