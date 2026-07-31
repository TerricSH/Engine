    #[test]
    fn world_from_scene_roundtrip() {
        let scene = sample_scene();
        let world = World::from_scene(&scene);
        assert_eq!(world.alive_count(), 2);

        // Verify Name components
        let names: Vec<_> = world.query::<Name>().map(|(_, n)| n.0.clone()).collect();
        assert!(names.contains(&"Main Camera".to_string()));
        assert!(names.contains(&"Cube".to_string()));

        // Verify Camera component
        let cameras: Vec<_> = world.query::<Camera>().collect();
        assert_eq!(cameras.len(), 1);

        // Verify Renderable component
        let renderables: Vec<_> = world.query::<Renderable>().collect();
        assert_eq!(renderables.len(), 1);
        assert_eq!(renderables[0].1.mesh_asset, "mesh-cube");
        assert_eq!(renderables[0].1.material_asset, "mat-default");
    }

    #[test]
    fn world_scene_roundtrip_preserves_entity_enabled_state() {
        let mut scene = sample_scene();
        scene.entities[0].enabled = false;

        let world = World::from_scene(&scene);
        let entity = world
            .persistent_to_entity
            .get(&scene.entities[0].persistent_id)
            .copied()
            .expect("scene entity should be mapped");
        assert!(!world.is_enabled(entity));

        let roundtripped = world.to_scene();
        let record = roundtripped
            .entities
            .iter()
            .find(|record| record.persistent_id == scene.entities[0].persistent_id)
            .expect("disabled entity should remain serialized");
        assert!(!record.enabled);
    }

    #[test]
    fn world_to_scene_uses_recycled_entity_generation() {
        let mut world = World::from_scene(&sample_scene());
        let first_id = world.entity_to_persistent[0]
            .clone()
            .expect("first scene entity should have an id");
        let first = world.persistent_to_entity[&first_id];
        assert!(world.destroy_entity(first));

        let recycled = world.create_entity();
        assert_eq!(recycled.index(), first.index());
        assert_ne!(recycled.generation(), first.generation());
        let recycled_id = "recycled-entity".to_string();
        world.entity_to_persistent[recycled.index() as usize] = Some(recycled_id.clone());
        world
            .persistent_to_entity
            .insert(recycled_id.clone(), recycled);
        world.add_component(recycled, Name("Recycled".to_string()));

        let roundtripped = world.to_scene();
        assert!(roundtripped
            .entities
            .iter()
            .any(|record| record.persistent_id == recycled_id));
    }

    #[test]
    fn world_to_scene_roundtrip() {
        let scene = sample_scene();
        let world = World::from_scene(&scene);
        let scene_back = world.to_scene();

        // The round-tripped scene should have the same number of entities.
        assert_eq!(scene_back.entities.len(), scene.entities.len());

        // Check entity persistent_ids are preserved.
        for orig_entity in &scene.entities {
            let found = scene_back
                .entities
                .iter()
                .any(|e| e.persistent_id == orig_entity.persistent_id);
            assert!(found, "missing entity {}", orig_entity.persistent_id);
        }

        // Check that typed components round-trip.
        for entity in &scene_back.entities {
            if entity.persistent_id == "camera-main" {
                assert!(entity.components.contains_key("engine.camera"));
            }
            if entity.persistent_id == "cube-01" {
                assert!(entity.components.contains_key("engine.renderable"));
                let renderable = &entity.components["engine.renderable"];
                let mesh = renderable.fields.get("mesh");
                assert!(matches!(mesh, Some(Value::Asset(a)) if a.id == "mesh-cube"));
            }
        }
    }

    #[test]
    fn world_from_scene_to_scene_preserves_renderable_fields() {
        let scene = sample_scene();
        let world = World::from_scene(&scene);
        let scene_back = world.to_scene();

        let cube = scene_back
            .entities
            .iter()
            .find(|e| e.persistent_id == "cube-01")
            .expect("cube-01 should exist");

        let r = &cube.components["engine.renderable"];
        assert_eq!(
            r.fields.get("mesh"),
            Some(&Value::Asset(AssetId::new("mesh-cube")))
        );
        assert_eq!(
            r.fields.get("material"),
            Some(&Value::Asset(AssetId::new("mat-default")))
        );
        assert_eq!(r.fields.get("visible"), Some(&Value::Bool(true)));
        assert_eq!(
            r.fields.get("render_layer"),
            Some(&Value::Str("Default".to_string()))
        );
        assert_eq!(r.fields.get("cast_shadows"), Some(&Value::Bool(true)));
    }

    #[test]
    fn world_scene_roundtrip_with_extraction() {
        // Verify that a scene converted to world and back still produces
        // valid extraction output (the existing extraction path still works).
        let scene = sample_scene();
        let world = World::from_scene(&scene);
        let scene_back = world.to_scene();

        // The round-tripped scene should be structurally valid for validation
        // and extraction (no duplicate IDs, valid camera, etc.)
        let diagnostics = crate::validation::validate_scene(&scene_back);
        assert!(
            diagnostics.is_empty(),
            "round-tripped scene has validation errors: {:?}",
            diagnostics
        );

        let result = crate::extraction::extract_renderer_input_from_world(&world, 42);
        assert!(
            result.is_ok(),
            "round-tripped scene extraction failed: {:?}",
            result
        );
        let input = result.unwrap();
        assert_eq!(input.frame_index, 42);
        assert_eq!(input.drawables.len(), 1);
        assert_eq!(input.views.len(), 1);
    }

    #[test]
    fn registry_aware_load_restores_and_roundtrips_external_component() {
        let scene = scene_with_component(ExternalComponent::TYPE_ID);
        let mut registry = ComponentRegistry::new();
        registry
            .register(external_extension(Some(deserialize_external)))
            .expect("register external component");
        let registry = Arc::new(registry);

        let report = World::from_scene_with_registry(&scene, Arc::clone(&registry));
        assert!(report.is_success(), "{:?}", report.diagnostics);
        let world = report.world;
        assert!(Arc::ptr_eq(
            world.component_registry().expect("registry installed"),
            &registry
        ));
        assert_eq!(
            world
                .query::<ExternalComponent>()
                .next()
                .map(|(_, c)| c.value),
            Some(42)
        );

        let roundtripped_scene = world.to_scene();
        let record = roundtripped_scene
            .entities
            .iter()
            .find_map(|entity| entity.components.get(ExternalComponent::TYPE_ID))
            .expect("external component serialized");
        assert_eq!(record.fields.get("value"), Some(&Value::UInt(42)));

        let restored = World::try_from_scene_with_registry(&roundtripped_scene, registry)
            .expect("roundtripped external component should load");
        assert_eq!(
            restored
                .query::<ExternalComponent>()
                .next()
                .map(|(_, c)| c.value),
            Some(42)
        );
    }

    #[test]
    fn strict_registry_load_rejects_unknown_component() {
        let scene = scene_with_component("test.unknown");
        let result =
            World::try_from_scene_with_registry(&scene, Arc::new(ComponentRegistry::new()));
        let error = match result {
            Ok(_) => panic!("unknown external component must fail strict loading"),
            Err(error) => error,
        };
        assert!(matches!(
            error.diagnostics.as_slice(),
            [SceneLoadDiagnostic::UnknownComponent {
                component_type_id,
                ..
            }] if component_type_id == "test.unknown"
        ));
    }
