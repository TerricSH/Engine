#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{Name, Transform};

    #[test]
    fn world_create_and_destroy_entity() {
        let mut world = World::new();
        let e = world.create_entity();
        assert!(world.is_alive(e));
        assert_eq!(world.alive_count(), 1);

        assert!(world.destroy_entity(e));
        assert!(!world.is_alive(e));
        assert_eq!(world.alive_count(), 0);
    }

    #[test]
    fn world_destroy_stale_returns_false() {
        let mut world = World::new();
        let e = world.create_entity();
        assert!(world.destroy_entity(e));
        assert!(!world.destroy_entity(e)); // stale
    }

    #[test]
    fn world_add_and_get_component() {
        let mut world = World::new();
        let e = world.create_entity();
        world.add_component(e, Name("Test".to_string()));
        assert!(world.has::<Name>(e));
        assert_eq!(world.get::<Name>(e).unwrap().0, "Test");
    }

    #[test]
    fn world_remove_component() {
        let mut world = World::new();
        let e = world.create_entity();
        world.add_component(e, Name("Test".to_string()));
        let removed = world.remove_component::<Name>(e);
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().0, "Test");
        assert!(!world.has::<Name>(e));
    }

    #[test]
    fn world_get_mut_component() {
        let mut world = World::new();
        let e = world.create_entity();
        world.add_component(e, Name("Before".to_string()));
        if let Some(name) = world.get_mut::<Name>(e) {
            name.0 = "After".to_string();
        }
        assert_eq!(world.get::<Name>(e).unwrap().0, "After");
    }

    #[test]
    fn world_query_components() {
        let mut world = World::new();
        let e1 = world.create_entity();
        let e2 = world.create_entity();
        world.add_component(e1, Name("First".to_string()));
        world.add_component(e2, Name("Second".to_string()));

        let names: Vec<_> = world.query::<Name>().map(|(_, n)| n.0.clone()).collect();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"First".to_string()));
        assert!(names.contains(&"Second".to_string()));
    }

    #[test]
    fn runtime_queries_exclude_disabled_entities() {
        let mut world = World::new();
        let enabled = world.create_entity();
        let disabled = world.create_entity();
        world.add_component(enabled, Name("Enabled".to_string()));
        world.add_component(disabled, Name("Disabled".to_string()));
        world.set_enabled(disabled, false);

        let queried: Vec<_> = world.query::<Name>().map(|(entity, _)| entity).collect();
        assert_eq!(queried, vec![enabled]);

        for (_, name) in world.query_mut::<Name>() {
            name.0.push_str(" runtime");
        }
        assert_eq!(world.get::<Name>(enabled).unwrap().0, "Enabled runtime");
        assert_eq!(world.get::<Name>(disabled).unwrap().0, "Disabled");
    }

    #[test]
    fn all_queries_include_disabled_entities_for_tooling() {
        let mut world = World::new();
        let enabled = world.create_entity();
        let disabled = world.create_entity();
        world.add_component(enabled, Name("Enabled".to_string()));
        world.add_component(disabled, Name("Disabled".to_string()));
        world.set_enabled(disabled, false);

        let queried: Vec<_> = world
            .query_all::<Name>()
            .map(|(entity, _)| entity)
            .collect();
        assert_eq!(queried, vec![enabled, disabled]);

        for (_, name) in world.query_all_mut::<Name>() {
            name.0.push_str(" tooling");
        }
        assert_eq!(world.get::<Name>(enabled).unwrap().0, "Enabled tooling");
        assert_eq!(world.get::<Name>(disabled).unwrap().0, "Disabled tooling");
    }

    #[test]
    fn world_clear() {
        let mut world = World::new();
        let e = world.create_entity();
        world.add_component(e, Name("X".to_string()));
        world.clear();
        assert_eq!(world.alive_count(), 0);
        assert!(world.query::<Name>().next().is_none());
    }

    #[test]
    fn world_destroy_entity_removes_components() {
        let mut world = World::new();
        let e = world.create_entity();
        world.add_component(e, Name("Gone".to_string()));
        assert!(world.destroy_entity(e));
        assert!(world.get::<Name>(e).is_none());
    }

    #[test]
    fn world_storage_for_unknown_type_returns_none() {
        let world = World::new();
        assert!(world.storage_for("nonexistent.type").is_none());
    }

    #[test]
    fn world_storage_for_known_type() {
        let mut world = World::new();
        let e = world.create_entity();
        world.add_component(e, Name("Test".to_string()));
        let storage = world.storage_for(Name::TYPE_ID);
        assert!(storage.is_some());
        assert_eq!(storage.unwrap().type_id(), Name::TYPE_ID);
        assert_eq!(storage.unwrap().len(), 1);
    }

    #[test]
    fn world_multiple_components_per_entity() {
        let mut world = World::new();
        let e = world.create_entity();
        world.add_component(e, Name("Multi".to_string()));
        world.add_component(
            e,
            Transform {
                translation: glam::Vec3::new(1.0, 2.0, 3.0),
                ..Default::default()
            },
        );

        assert!(world.has::<Name>(e));
        assert!(world.has::<Transform>(e));
        assert_eq!(world.get::<Transform>(e).unwrap().translation.x, 1.0);
    }

    #[test]
    fn persistent_id_resolves_current_live_entity() {
        let mut world = World::from_scene(&crate::sample_scene());
        let entity = world
            .entity_by_persistent_id("cube-01")
            .expect("sample entity");
        assert_eq!(world.persistent_id(entity), Some("cube-01"));

        assert!(world.destroy_entity(entity));
        assert!(world.entity_by_persistent_id("cube-01").is_none());
        assert!(world.entity_by_persistent_id("missing").is_none());
    }

    #[test]
    fn create_persistent_entity_rejects_empty_and_duplicate_ids_transactionally() {
        let mut world = World::new();

        assert_eq!(
            world.create_persistent_entity(""),
            Err(PersistentEntityCreateError::EmptyId)
        );
        assert_eq!(world.alive_count(), 0);
        assert!(world.persistent_entities().next().is_none());

        let first = world
            .create_persistent_entity("runtime-entity")
            .expect("valid persistent entity");
        let alive_before_duplicate = world.alive_count();
        assert_eq!(
            world.create_persistent_entity("runtime-entity"),
            Err(PersistentEntityCreateError::DuplicateId(
                "runtime-entity".into()
            ))
        );
        assert_eq!(world.alive_count(), alive_before_duplicate);
        assert_eq!(world.entity_by_persistent_id("runtime-entity"), Some(first));
        assert_eq!(world.persistent_id(first), Some("runtime-entity"));
    }

    #[test]
    fn create_persistent_entity_maintains_mappings_when_handles_are_recycled() {
        let mut world = World::new();
        let old = world
            .create_persistent_entity("old-id")
            .expect("first persistent entity");
        assert!(world.destroy_entity(old));
        assert!(world.entity_by_persistent_id("old-id").is_none());

        let recycled = world
            .create_persistent_entity("new-id")
            .expect("recycled persistent entity");

        assert_eq!(recycled.index(), old.index());
        assert_ne!(recycled.generation(), old.generation());
        assert_eq!(world.persistent_id(recycled), Some("new-id"));
        assert_eq!(world.entity_by_persistent_id("new-id"), Some(recycled));
        assert!(world.entity_by_persistent_id("old-id").is_none());
        assert_eq!(
            world
                .persistent_entities()
                .map(|(id, entity)| (id.to_owned(), entity))
                .collect::<Vec<_>>(),
            vec![("new-id".into(), recycled)]
        );
    }

    #[test]
    fn assign_persistent_id_binds_live_entities_and_rejects_conflicts() {
        let mut world = World::new();
        let entity = world.create_entity();
        let other = world.create_entity();

        assert_eq!(
            world.assign_persistent_id(entity, ""),
            Err(PersistentEntityCreateError::EmptyId)
        );
        assert!(world.persistent_id(entity).is_none());

        world
            .assign_persistent_id(entity, "spawned-01")
            .expect("first assignment succeeds");
        assert_eq!(world.persistent_id(entity), Some("spawned-01"));
        assert_eq!(world.entity_by_persistent_id("spawned-01"), Some(entity));

        assert_eq!(
            world.assign_persistent_id(other, "spawned-01"),
            Err(PersistentEntityCreateError::DuplicateId(
                "spawned-01".into()
            ))
        );
        assert_eq!(
            world.assign_persistent_id(entity, "spawned-02"),
            Err(PersistentEntityCreateError::AlreadyPersistent(
                "spawned-01".into()
            ))
        );
        assert!(world.persistent_id(other).is_none());

        assert!(world.destroy_entity(other));
        assert_eq!(
            world.assign_persistent_id(other, "stale"),
            Err(PersistentEntityCreateError::StaleEntity)
        );
        assert!(world.entity_by_persistent_id("stale").is_none());

        // Destroying the entity releases its ID for a recycled handle.
        assert!(world.destroy_entity(entity));
        let recycled = world.create_entity();
        world
            .assign_persistent_id(recycled, "spawned-01")
            .expect("destroyed entity releases its persistent id");
        assert_eq!(world.entity_by_persistent_id("spawned-01"), Some(recycled));
    }

    #[test]
    fn persistent_entity_iteration_and_parent_cleanup_survive_destroy() {
        let mut world = World::from_scene(&crate::sample_scene());
        let parent = world
            .entity_by_persistent_id("camera-main")
            .expect("sample camera");
        let child = world
            .entity_by_persistent_id("cube-01")
            .expect("sample cube");
        if !world.has::<Transform>(child) {
            world.add_component(child, Transform::default());
        }
        world.get_mut::<Transform>(child).unwrap().parent = Some(parent);

        let ids = world
            .persistent_entities()
            .map(|(id, _)| id.to_owned())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["camera-main", "cube-01"]);

        assert!(world.destroy_entity(parent));
        assert_eq!(world.get::<Transform>(child).unwrap().parent, None);
        assert_eq!(
            world
                .persistent_entities()
                .map(|(id, _)| id)
                .collect::<Vec<_>>(),
            vec!["cube-01"]
        );
    }

    #[test]
    fn world_origin_shift_preserves_logical_positions_across_hierarchies() {
        let mut world = World::new();
        let root = world.create_entity();
        let child = world.create_entity();
        let grandchild = world.create_entity();
        world.add_component(
            root,
            Transform {
                translation: glam::Vec3::new(9000.0, 10.0, -500.0),
                ..Default::default()
            },
        );
        world.add_component(
            child,
            Transform {
                translation: glam::Vec3::new(1.0, 2.0, 3.0),
                parent: Some(root),
                ..Default::default()
            },
        );
        world.add_component(
            grandchild,
            Transform {
                translation: glam::Vec3::new(-4.0, 0.5, 8.0),
                parent: Some(child),
                ..Default::default()
            },
        );
        // A disabled entity is world-space state too and must shift.
        let disabled = world.create_entity();
        world.add_component(
            disabled,
            Transform {
                translation: glam::Vec3::new(8500.0, 0.0, 0.0),
                ..Default::default()
            },
        );
        world.set_enabled(disabled, false);

        let delta = [9000.0, 10.0, -500.0];
        let shifted = world.shift_world_origin(delta);
        assert_eq!(shifted, 2);
        assert_eq!(world.world_origin(), delta);

        assert_eq!(
            world.get::<Transform>(root).unwrap().translation,
            glam::Vec3::ZERO
        );
        // Local translations below a root are untouched: children follow
        // their root through the hierarchy.
        assert_eq!(
            world.get::<Transform>(child).unwrap().translation,
            glam::Vec3::new(1.0, 2.0, 3.0)
        );
        assert_eq!(
            world.get::<Transform>(grandchild).unwrap().translation,
            glam::Vec3::new(-4.0, 0.5, 8.0)
        );
        assert_eq!(
            world.get::<Transform>(disabled).unwrap().translation,
            glam::Vec3::new(-500.0, -10.0, 500.0)
        );
    }

    #[test]
    fn world_origin_shift_treats_transformless_parent_as_identity_root() {
        let mut world = World::new();
        let folder = world.create_entity();
        let entity = world.create_entity();
        world.add_component(
            entity,
            Transform {
                translation: glam::Vec3::new(100.0, 0.0, 0.0),
                parent: Some(folder),
                ..Default::default()
            },
        );

        let shifted = world.shift_world_origin([100.0, 0.0, 0.0]);
        assert_eq!(shifted, 1);
        assert_eq!(
            world.get::<Transform>(entity).unwrap().translation,
            glam::Vec3::ZERO
        );
    }

    #[test]
    fn world_origin_accumulates_and_clear_resets_it() {
        let mut world = World::new();
        let entity = world.create_entity();
        world.add_component(entity, Transform::default());

        world.shift_world_origin([1000.0, 0.0, 0.0]);
        world.shift_world_origin([0.0, -250.0, 500.0]);
        assert_eq!(world.world_origin(), [1000.0, -250.0, 500.0]);
        assert_eq!(
            world.get::<Transform>(entity).unwrap().translation,
            glam::Vec3::new(-1000.0, 250.0, -500.0)
        );

        world.clear();
        assert_eq!(world.world_origin(), [0.0; 3]);
    }

    #[test]
    fn save_restore_origin_does_not_translate_relative_transforms_twice() {
        let mut world = World::new();
        let entity = world
            .create_persistent_entity("saved")
            .expect("persistent entity");
        world.add_component(
            entity,
            Transform {
                translation: glam::Vec3::new(12.0, 3.0, -4.0),
                ..Transform::default()
            },
        );

        world
            .restore_world_origin([1000.0, 0.0, 250.0])
            .expect("valid origin");
        assert_eq!(world.world_origin(), [1000.0, 0.0, 250.0]);
        assert_eq!(
            world.get::<Transform>(entity).unwrap().translation,
            glam::Vec3::new(12.0, 3.0, -4.0)
        );
        assert!(world.restore_world_origin([f64::NAN, 0.0, 0.0]).is_err());
    }
}
