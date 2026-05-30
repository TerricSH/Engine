use std::collections::BTreeMap;

use crate::component::ComponentStorageDyn;
use crate::scene::SceneSettings;
use crate::{Component, Entity, EntityManager, SparseSet};

pub(crate) mod scene;

/// The ECS World — owns all entities and component storages.
///
/// Provides typed component access, entity lifecycle management, and
/// bidirectional conversion to/from [`Scene`] for serialisation.
pub struct World {
    pub(crate) entities: EntityManager,
    pub(crate) storages: BTreeMap<&'static str, Box<dyn ComponentStorageDyn>>,
    // Mapping for Scene ↔ World conversion.
    pub(crate) persistent_to_entity: BTreeMap<String, Entity>,
    pub(crate) entity_to_persistent: Vec<Option<String>>,
    // Stored scene-level settings (preserved through round-trips).
    pub(crate) scene_settings: SceneSettings,
    pub(crate) scene_id: String,
    pub(crate) scene_name: String,
}

impl World {
    pub fn new() -> Self {
        Self {
            entities: EntityManager::new(),
            storages: BTreeMap::new(),
            persistent_to_entity: BTreeMap::new(),
            entity_to_persistent: Vec::new(),
            scene_settings: SceneSettings::default(),
            scene_id: "ecs-world".to_string(),
            scene_name: "ECS World".to_string(),
        }
    }

    // ── Entity management ─────────────────────────────────────────────

    /// Create a new entity and return its handle.
    pub fn create_entity(&mut self) -> Entity {
        self.entities.allocate()
    }

    /// Destroy an entity and all of its components.
    ///
    /// Returns `false` if the entity handle is stale.
    pub fn destroy_entity(&mut self, entity: Entity) -> bool {
        if !self.entities.free(entity) {
            return false;
        }
        // Remove the entity from all storages.
        for (_, storage) in self.storages.iter_mut() {
            storage.remove(entity);
        }
        // Clean up persistent_id mapping if present.
        let idx = entity.index() as usize;
        if idx < self.entity_to_persistent.len() {
            if let Some(ref pid) = self.entity_to_persistent[idx] {
                self.persistent_to_entity.remove(pid);
            }
            self.entity_to_persistent[idx] = None;
        }
        true
    }

    /// Returns `true` if the entity handle is still alive.
    pub fn is_alive(&self, entity: Entity) -> bool {
        self.entities.is_alive(entity)
    }

    /// Number of live entities.
    pub fn alive_count(&self) -> usize {
        self.entities.alive_count()
    }

    /// Get the persistent ID for an entity, if one was assigned via [`from_scene`](World::from_scene).
    ///
    /// Returns `None` for entities created directly via [`create_entity`](World::create_entity)
    /// without a corresponding persistent ID.
    pub fn persistent_id(&self, entity: Entity) -> Option<&str> {
        let idx = entity.index() as usize;
        if idx < self.entity_to_persistent.len() {
            self.entity_to_persistent[idx].as_deref()
        } else {
            None
        }
    }

    // ── Component management ──────────────────────────────────────────

    /// Add a typed component to an entity.
    ///
    /// # Panics
    /// Panics if the entity is stale.
    pub fn add_component<T: Component>(&mut self, entity: Entity, component: T) {
        assert!(
            self.is_alive(entity),
            "cannot add component to stale entity"
        );
        let storage = self
            .storages
            .entry(T::TYPE_ID)
            .or_insert_with(|| Box::new(SparseSet::<T>::new()));
        storage
            .as_any_mut()
            .downcast_mut::<SparseSet<T>>()
            .expect("storage type mismatch")
            .insert(entity, component);
    }

    /// Remove a typed component from an entity.
    ///
    /// Returns the component if it existed, `None` otherwise.
    pub fn remove_component<T: Component>(&mut self, entity: Entity) -> Option<T> {
        if let Some(storage) = self.storages.get_mut(T::TYPE_ID) {
            storage
                .as_any_mut()
                .downcast_mut::<SparseSet<T>>()
                .expect("storage type mismatch")
                .remove(entity)
        } else {
            None
        }
    }

    /// Borrow a component by type.
    pub fn get<T: Component>(&self, entity: Entity) -> Option<&T> {
        if let Some(storage) = self.storages.get(T::TYPE_ID) {
            storage
                .as_any()
                .downcast_ref::<SparseSet<T>>()
                .expect("storage type mismatch")
                .get(entity)
        } else {
            None
        }
    }

    /// Mutably borrow a component by type.
    pub fn get_mut<T: Component>(&mut self, entity: Entity) -> Option<&mut T> {
        if let Some(storage) = self.storages.get_mut(T::TYPE_ID) {
            storage
                .as_any_mut()
                .downcast_mut::<SparseSet<T>>()
                .expect("storage type mismatch")
                .get_mut(entity)
        } else {
            None
        }
    }

    /// Returns `true` if the entity has a component of type `T`.
    pub fn has<T: Component>(&self, entity: Entity) -> bool {
        if let Some(storage) = self.storages.get(T::TYPE_ID) {
            storage
                .as_any()
                .downcast_ref::<SparseSet<T>>()
                .expect("storage type mismatch")
                .contains(entity)
        } else {
            false
        }
    }

    // ── Query helpers ─────────────────────────────────────────────────

    /// Iterate over all entities that have component `T`.
    ///
    /// Note: iteration does not validate entity liveness; stale entities may
    /// appear.  Use [`World::is_alive`] to filter if needed.
    pub fn query<T: Component>(&self) -> impl Iterator<Item = (Entity, &T)> + '_ {
        if let Some(storage) = self.storages.get(T::TYPE_ID) {
            let set = storage
                .as_any()
                .downcast_ref::<SparseSet<T>>()
                .expect("storage type mismatch");
            // Wrap into a boxed iterator for type-erased return.
            // We need to copy the entity index from dense entries and
            // reconstruct Entity handles (without generation tracking here).
            let items: Vec<_> = set.iter().collect();
            items.into_iter()
        } else {
            vec![].into_iter()
        }
    }

    /// Mutably iterate over all entities that have component `T`.
    pub fn query_mut<T: Component>(&mut self) -> impl Iterator<Item = (Entity, &mut T)> + '_ {
        if let Some(storage) = self.storages.get_mut(T::TYPE_ID) {
            let set = storage
                .as_any_mut()
                .downcast_mut::<SparseSet<T>>()
                .expect("storage type mismatch");
            let items: Vec<_> = set.iter_mut().collect();
            items.into_iter()
        } else {
            vec![].into_iter()
        }
    }

    // ── Dynamic storage access (for future Gate 9 extensions) ─────────

    /// Access a storage by its `type_id` string.
    pub fn storage_for(&self, type_id: &str) -> Option<&dyn ComponentStorageDyn> {
        self.storages.get(type_id).map(|b| b.as_ref())
    }

    // ── Clear ─────────────────────────────────────────────────────────

    /// Remove all entities and components.
    pub fn clear(&mut self) {
        self.entities = EntityManager::new();
        self.storages.clear();
        self.persistent_to_entity.clear();
        self.entity_to_persistent.clear();
        self.scene_settings = SceneSettings::default();
        self.scene_id = "ecs-world".to_string();
        self.scene_name = "ECS World".to_string();
    }
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

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
}
