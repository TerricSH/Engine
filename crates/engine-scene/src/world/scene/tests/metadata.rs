    #[test]
    fn entity_record_parent_is_applied_and_preserved() {
        let mut scene = sample_scene();
        let parent_id = scene.entities[0].persistent_id.clone();
        scene.entities[1].parent = Some(parent_id.clone());
        scene.entities[1].components.insert(
            Transform::TYPE_ID.to_string(),
            ComponentRecord {
                schema_version: SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: BTreeMap::new(),
            },
        );

        let world = World::from_scene(&scene);
        let child = world
            .persistent_to_entity
            .get(&scene.entities[1].persistent_id)
            .copied()
            .expect("child entity");
        let parent = world
            .persistent_to_entity
            .get(&parent_id)
            .copied()
            .expect("parent entity");
        assert_eq!(
            world.get::<Transform>(child).and_then(|t| t.parent),
            Some(parent)
        );

        let roundtripped = world.to_scene();
        assert_eq!(roundtripped.entities[1].parent, Some(parent_id));
    }

    #[test]
    fn prefab_instance_linkage_survives_strict_scene_world_roundtrip() {
        let mut scene = sample_scene();
        scene.entities[1].components.insert(
            PrefabInstanceRef::TYPE_ID.to_string(),
            ComponentRecord {
                schema_version: SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: BTreeMap::from([
                    (
                        "source_asset".into(),
                        Value::Asset(AssetId::new("prefab-crate")),
                    ),
                    (
                        "instance_id".into(),
                        Value::Str("prefab-instance-crate".into()),
                    ),
                    (
                        "entity_persistent_id".into(),
                        Value::Str("crate-root".into()),
                    ),
                    ("schema_major".into(), Value::UInt(0)),
                    ("schema_minor".into(), Value::UInt(1)),
                    ("schema_patch".into(), Value::UInt(0)),
                ]),
            },
        );
        let mut registry = ComponentRegistry::new();
        registry.register_core();
        let world = World::try_from_scene_with_registry(&scene, Arc::new(registry)).unwrap();
        let linkage = world
            .query::<PrefabInstanceRef>()
            .next()
            .map(|(_, linkage)| linkage)
            .expect("prefab linkage materialized");
        assert_eq!(linkage.source_asset, "prefab-crate");
        assert_eq!(linkage.entity_persistent_id, "crate-root");

        let roundtripped = world.to_scene();
        assert_eq!(
            roundtripped.entities[1]
                .components
                .get(PrefabInstanceRef::TYPE_ID),
            scene.entities[1].components.get(PrefabInstanceRef::TYPE_ID)
        );
    }
