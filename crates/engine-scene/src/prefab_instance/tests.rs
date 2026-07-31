#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::component::{ComponentStorageDyn, SparseSet};
    use crate::prefab::PrefabChildRef;
    use crate::registry::{ComponentExtension, ComponentMeta, ComponentRegistry};
    use crate::{ComponentRecord, EntityRecord};
    use engine_serialize::AssetId;

    fn transform_record() -> ComponentRecord {
        let mut fields = BTreeMap::new();
        fields.insert("translation".to_string(), Value::Vec3([1.0, 2.0, 3.0]));
        fields.insert("rotation".to_string(), Value::Quat([0.0, 0.0, 0.0, 1.0]));
        fields.insert("scale".to_string(), Value::Vec3([1.0, 1.0, 1.0]));
        ComponentRecord {
            schema_version: SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields,
        }
    }

    fn entity(id: &str, parent: Option<&str>) -> EntityRecord {
        let mut components = BTreeMap::new();
        components.insert(Transform::TYPE_ID.to_string(), transform_record());
        EntityRecord {
            persistent_id: id.to_string(),
            parent: parent.map(str::to_string),
            name: Some(id.to_string()),
            enabled: true,
            components,
        }
    }

    fn prefab(asset_id: &str, id: &str) -> Prefab {
        let mut prefab = Prefab::new(AssetId::new(asset_id));
        prefab.add_entity(entity(id, None));
        prefab
    }

    #[test]
    fn scene_only_components_are_collected_instead_of_materialised() {
        let mut script_components = BTreeMap::new();
        script_components.insert(Transform::TYPE_ID.to_string(), transform_record());
        script_components.insert(
            "engine.script".to_string(),
            ComponentRecord {
                schema_version: SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: BTreeMap::from([
                    (
                        "assembly_id".to_string(),
                        engine_serialize::Value::Str("GameScripts".to_string()),
                    ),
                    (
                        "class_name".to_string(),
                        engine_serialize::Value::Str("Game.Enemy".to_string()),
                    ),
                ]),
            },
        );
        let mut prefab = Prefab::new(AssetId::new("prefabs/scripted.prefab"));
        prefab.add_entity(EntityRecord {
            persistent_id: "enemy".to_string(),
            parent: None,
            name: Some("Enemy".to_string()),
            enabled: true,
            components: script_components,
        });

        let mut world = World::new();
        let result = instantiate_prefab(&mut world, &prefab, None).unwrap();
        assert_eq!(result.all_entities.len(), 1);
        assert_eq!(result.scene_only_components.len(), 1);
        let (entity, type_id, record) = &result.scene_only_components[0];
        assert_eq!(*entity, result.root_entity);
        assert_eq!(type_id, "engine.script");
        assert!(record.enabled);
        // The script metadata never entered ECS storage, and its presence did
        // not fail the strict constructibility validation.
        assert!(world.get_any(result.root_entity, "engine.script").is_none());
        assert!(world.get::<Transform>(result.root_entity).is_some());
    }

    #[test]
    fn empty_prefab_fails_before_allocating() {
        let mut world = World::new();
        let prefab = Prefab::new(AssetId::new("prefabs/empty.prefab"));
        let error = instantiate_prefab(&mut world, &prefab, None).unwrap_err();
        assert!(matches!(
            error,
            PrefabInstantiateError::EmptyHierarchy { .. }
        ));
        assert_eq!(world.alive_count(), 0);
    }

    #[test]
    fn instantiates_hierarchy_and_defaults() {
        let mut world = World::new();
        let mut prefab = prefab("prefabs/hierarchy.prefab", "root");
        prefab.add_entity(entity("child", Some("root")));
        prefab.set_default(Transform::TYPE_ID, "scale", Value::Vec3([5.0; 3]));

        let result = instantiate_prefab(&mut world, &prefab, None).unwrap();
        assert_eq!(result.all_entities.len(), 2);
        let child = result
            .all_entities
            .iter()
            .copied()
            .find(|entity| {
                world
                    .get::<PrefabInstanceRef>(*entity)
                    .is_some_and(|reference| reference.entity_persistent_id == "child")
            })
            .unwrap();
        let transform = world.get::<Transform>(child).unwrap();
        assert_eq!(transform.parent, Some(result.root_entity));
        assert_eq!(transform.scale, glam::Vec3::splat(5.0));
    }

    #[test]
    fn nested_prefab_is_loaded_and_attached() {
        let mut parent = prefab("prefabs/parent.prefab", "parent-root");
        parent.child_prefab_refs.push(PrefabChildRef {
            entity_persistent_id: "parent-root".to_string(),
            prefab_asset: AssetId::new("prefabs/child.prefab"),
        });
        let child = prefab("prefabs/child.prefab", "child-root");
        let mut registry = PrefabRegistry::new();
        registry.register("prefabs/parent.prefab", parent);
        registry.register("prefabs/child.prefab", child);

        let mut world = World::new();
        let result =
            instantiate_prefab_from_asset(&mut world, &registry, "prefabs/parent.prefab").unwrap();
        assert_eq!(result.all_entities.len(), 2);
        let child_entity = result
            .all_entities
            .iter()
            .copied()
            .find(|entity| {
                world
                    .get::<PrefabInstanceRef>(*entity)
                    .is_some_and(|reference| reference.source_asset == "prefabs/child.prefab")
            })
            .unwrap();
        assert_eq!(
            world.get::<Transform>(child_entity).unwrap().parent,
            Some(result.root_entity)
        );
    }

    #[test]
    fn self_cycle_is_rejected_without_allocating() {
        let mut root = prefab("prefabs/self.prefab", "root");
        root.child_prefab_refs.push(PrefabChildRef {
            entity_persistent_id: "root".to_string(),
            prefab_asset: AssetId::new("prefabs/self.prefab"),
        });
        let mut registry = PrefabRegistry::new();
        registry.register("prefabs/self.prefab", root);
        let mut world = World::new();

        let error = instantiate_prefab_from_asset(&mut world, &registry, "prefabs/self.prefab")
            .unwrap_err();
        assert!(matches!(
            error,
            PrefabInstantiateError::DependencyCycle { .. }
        ));
        assert_eq!(world.alive_count(), 0);
    }

    #[test]
    fn indirect_cycle_is_rejected_without_allocating() {
        let mut a = prefab("prefabs/a.prefab", "a");
        a.child_prefab_refs.push(PrefabChildRef {
            entity_persistent_id: "a".to_string(),
            prefab_asset: AssetId::new("prefabs/b.prefab"),
        });
        let mut b = prefab("prefabs/b.prefab", "b");
        b.child_prefab_refs.push(PrefabChildRef {
            entity_persistent_id: "b".to_string(),
            prefab_asset: AssetId::new("prefabs/a.prefab"),
        });
        let mut registry = PrefabRegistry::new();
        registry.register("prefabs/a.prefab", a);
        registry.register("prefabs/b.prefab", b);
        let mut world = World::new();

        let error =
            instantiate_prefab_from_asset(&mut world, &registry, "prefabs/a.prefab").unwrap_err();
        assert!(matches!(
            error,
            PrefabInstantiateError::DependencyCycle { .. }
        ));
        assert_eq!(world.alive_count(), 0);
    }

    #[test]
    fn missing_child_is_a_structured_error() {
        let mut root = prefab("prefabs/root.prefab", "root");
        root.child_prefab_refs.push(PrefabChildRef {
            entity_persistent_id: "root".to_string(),
            prefab_asset: AssetId::new("prefabs/missing.prefab"),
        });
        let registry = PrefabRegistry::new();
        let mut world = World::new();

        let error = instantiate_prefab(&mut world, &root, Some(&registry)).unwrap_err();
        assert!(matches!(
            error,
            PrefabInstantiateError::MissingChildPrefab { child_asset_id, .. }
                if child_asset_id == "prefabs/missing.prefab"
        ));
        assert_eq!(world.alive_count(), 0);
    }

    #[test]
    fn duplicate_persistent_id_is_rejected() {
        let mut duplicate = prefab("prefabs/duplicate.prefab", "same");
        duplicate.add_entity(entity("same", Some("same")));
        let mut world = World::new();

        let error = instantiate_prefab(&mut world, &duplicate, None).unwrap_err();
        assert!(matches!(
            error,
            PrefabInstantiateError::DuplicatePersistentId { persistent_id, .. }
                if persistent_id == "same"
        ));
        assert_eq!(world.alive_count(), 0);
    }

    #[derive(Debug)]
    struct FlakyComponent;

    impl Component for FlakyComponent {
        const TYPE_ID: &'static str = "test.flaky_prefab_component";
    }

    static FLAKY_DESERIALIZE_CALLS: AtomicUsize = AtomicUsize::new(0);

    fn flaky_storage() -> Box<dyn ComponentStorageDyn> {
        Box::new(SparseSet::<FlakyComponent>::new())
    }

    fn flaky_deserialize(_: &BTreeMap<String, Value>) -> Box<dyn Any> {
        if FLAKY_DESERIALIZE_CALLS.fetch_add(1, Ordering::SeqCst) == 0 {
            Box::new(FlakyComponent)
        } else {
            Box::new("wrong component type".to_string())
        }
    }

    #[test]
    fn component_failure_rolls_back_every_created_entity() {
        FLAKY_DESERIALIZE_CALLS.store(0, Ordering::SeqCst);
        let mut registry = ComponentRegistry::new();
        registry
            .register(ComponentExtension {
                meta: ComponentMeta {
                    type_id: FlakyComponent::TYPE_ID,
                    display_name: "Flaky",
                    schema_version: (0, 1, 0),
                    has_editor: false,
                    script_access: crate::ScriptAccess::None,
                },
                storage_factory: flaky_storage,
                serialize: None,
                deserialize: Some(flaky_deserialize),
            })
            .unwrap();

        let mut world = World::new();
        world.set_component_registry(registry);
        let survivor = world.create_entity();
        world.add_component(survivor, Name("survivor".to_string()));
        let baseline = world.alive_count();

        let mut failing = prefab("prefabs/failing.prefab", "root");
        failing.hierarchy[0].components.insert(
            FlakyComponent::TYPE_ID.to_string(),
            ComponentRecord {
                schema_version: SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: BTreeMap::new(),
            },
        );

        let error = instantiate_prefab(&mut world, &failing, None).unwrap_err();
        assert!(matches!(
            error,
            PrefabInstantiateError::ComponentNotConstructible { component_type_id, .. }
                if component_type_id == FlakyComponent::TYPE_ID
        ));
        assert_eq!(world.alive_count(), baseline);
        assert!(world.is_alive(survivor));
        assert_eq!(world.get::<Name>(survivor).unwrap().0, "survivor");
    }

    #[test]
    fn maximum_nesting_depth_is_enforced() {
        let mut registry = PrefabRegistry::new();
        for depth in 0..=(MAX_PREFAB_NESTING_DEPTH + 1) {
            let asset_id = format!("prefabs/depth-{depth}.prefab");
            let mut current = prefab(&asset_id, &format!("root-{depth}"));
            if depth <= MAX_PREFAB_NESTING_DEPTH {
                current.child_prefab_refs.push(PrefabChildRef {
                    entity_persistent_id: format!("root-{depth}"),
                    prefab_asset: AssetId::new(format!("prefabs/depth-{}.prefab", depth + 1)),
                });
            }
            registry.register(asset_id, current);
        }
        let mut world = World::new();
        let error = instantiate_prefab_from_asset(&mut world, &registry, "prefabs/depth-0.prefab")
            .unwrap_err();
        assert!(matches!(
            error,
            PrefabInstantiateError::MaximumDepthExceeded { .. }
        ));
        assert_eq!(world.alive_count(), 0);
    }
}
