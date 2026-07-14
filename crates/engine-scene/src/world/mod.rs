use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use crate::component::ComponentStorageDyn;
use crate::registry::ComponentRegistry;
use crate::scene::{ComponentRecord, DiagnosticsPolicy, SceneSettings};
use crate::{Component, Entity, EntityManager, SparseSet};
use engine_serialize::{
    AssetId, ComponentTypeId, EngineVersion, PersistentId, SchemaVersion, Value,
};

pub(crate) mod scene;

// Entity only has a 32-bit generation field, so use a process-wide,
// full-period sequence of well-spaced initial generations for each World.
// The odd multiplier is a bijection over u32; adjacent World allocations are
// separated by billions of ordinary per-slot generation increments instead
// of both beginning at zero.
static NEXT_WORLD_GENERATION_ID: AtomicU32 = AtomicU32::new(1);
const WORLD_GENERATION_STRIDE: u32 = 0x9E37_79B9;

fn fresh_world_entity_manager() -> EntityManager {
    let id = NEXT_WORLD_GENERATION_ID.fetch_add(1, Ordering::Relaxed);
    EntityManager::with_initial_generation(id.wrapping_mul(WORLD_GENERATION_STRIDE))
}

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
    // Per-entity enabled/disabled state (true = enabled, processed by systems).
    pub(crate) enabled: Vec<bool>,
    // Disabled components are intentionally not instantiated into ECS
    // storages, but their complete scene records must survive a round-trip.
    pub(crate) disabled_components: Vec<BTreeMap<ComponentTypeId, ComponentRecord>>,
    // Keep the schema version attached to every loaded component record. The
    // ECS value can change at runtime without silently downgrading its schema
    // version when the scene is saved again.
    pub(crate) component_schema_versions: Vec<BTreeMap<ComponentTypeId, SchemaVersion>>,
    // Entity hierarchy is part of the scene contract even for entities that
    // do not currently have an enabled Transform component.
    pub(crate) entity_parents: Vec<Option<PersistentId>>,
    // Stored scene-level settings (preserved through round-trips).
    pub(crate) scene_settings: SceneSettings,
    pub(crate) scene_schema_version: SchemaVersion,
    pub(crate) scene_engine_version: EngineVersion,
    pub(crate) scene_id: String,
    pub(crate) scene_name: String,
    pub(crate) scene_dependencies: Vec<AssetId>,
    pub(crate) diagnostics_policy: DiagnosticsPolicy,
    /// Optional registry for serialize/deserialize hooks of external components.
    pub(crate) component_registry: Option<Arc<ComponentRegistry>>,
}

impl World {
    pub fn new() -> Self {
        Self {
            entities: fresh_world_entity_manager(),
            storages: BTreeMap::new(),
            persistent_to_entity: BTreeMap::new(),
            entity_to_persistent: Vec::new(),
            enabled: Vec::new(),
            disabled_components: Vec::new(),
            component_schema_versions: Vec::new(),
            entity_parents: Vec::new(),
            scene_settings: SceneSettings::default(),
            scene_schema_version: SchemaVersion::new(0, 1, 0),
            scene_engine_version: "0.1.0".to_string(),
            scene_id: "ecs-world".to_string(),
            scene_name: "ECS World".to_string(),
            scene_dependencies: Vec::new(),
            diagnostics_policy: DiagnosticsPolicy::Strict,
            component_registry: None,
        }
    }

    /// Attach a [`ComponentRegistry`] to enable serialization of registered
    /// component types (e.g. physics) through their extension hooks.
    pub fn set_component_registry(&mut self, registry: ComponentRegistry) {
        self.set_shared_component_registry(Arc::new(registry));
    }

    /// Attach a registry that can be safely shared by multiple worlds.
    pub fn set_shared_component_registry(&mut self, registry: Arc<ComponentRegistry>) {
        for (type_id, storage) in registry.create_storages() {
            // A malformed extension must not poison a typed core storage and
            // turn the next `add_component` into a downcast panic. The strict
            // scene-loading entry reports this mismatch structurally.
            if storage.type_id() == type_id {
                self.storages.entry(type_id).or_insert(storage);
            }
        }
        self.component_registry = Some(registry);
    }

    /// Return the installed shared component registry, if any.
    pub fn component_registry(&self) -> Option<&Arc<ComponentRegistry>> {
        self.component_registry.as_ref()
    }

    /// Access the scene-level settings (ambient, gravity, camera defaults etc.).
    pub fn scene_settings(&self) -> &SceneSettings {
        &self.scene_settings
    }

    // ── Entity management ─────────────────────────────────────────────

    /// Create a new entity and return its handle.
    pub fn create_entity(&mut self) -> Entity {
        let entity = self.entities.allocate();
        let idx = entity.index() as usize;
        if self.enabled.len() <= idx {
            self.enabled.resize(idx + 1, true);
        } else {
            self.enabled[idx] = true;
        }
        if self.disabled_components.len() <= idx {
            self.disabled_components.resize_with(idx + 1, BTreeMap::new);
        } else {
            self.disabled_components[idx].clear();
        }
        if self.component_schema_versions.len() <= idx {
            self.component_schema_versions
                .resize_with(idx + 1, BTreeMap::new);
        } else {
            self.component_schema_versions[idx].clear();
        }
        if self.entity_parents.len() <= idx {
            self.entity_parents.resize(idx + 1, None);
        } else {
            self.entity_parents[idx] = None;
        }
        entity
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
        if idx < self.disabled_components.len() {
            self.disabled_components[idx].clear();
        }
        if idx < self.component_schema_versions.len() {
            self.component_schema_versions[idx].clear();
        }
        if idx < self.entity_parents.len() {
            self.entity_parents[idx] = None;
        }
        true
    }

    /// Returns `true` if the entity handle is still alive.
    pub fn is_alive(&self, entity: Entity) -> bool {
        self.entities.is_alive(entity)
    }

    /// Enable or disable an entity.
    ///
    /// Disabled entities are preserved in the world but are not processed by
    /// systems (they are effectively "inactive").
    pub fn set_enabled(&mut self, entity: Entity, enabled: bool) {
        if !self.entities.is_alive(entity) {
            return;
        }
        let idx = entity.index() as usize;
        if idx < self.enabled.len() {
            self.enabled[idx] = enabled;
        }
    }

    /// Returns `true` if the entity is enabled.
    ///
    /// Newly created entities are enabled by default.  Returns `false` for
    /// stale entity handles.
    pub fn is_enabled(&self, entity: Entity) -> bool {
        if !self.entities.is_alive(entity) {
            return false;
        }
        let idx = entity.index() as usize;
        idx < self.enabled.len() && self.enabled[idx]
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
        if !self.entities.is_alive(entity) {
            return None;
        }
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
        self.remove_disabled_component_record(entity, T::TYPE_ID);
    }

    /// Remove a typed component from an entity.
    ///
    /// Returns the component if it existed, `None` otherwise.
    pub fn remove_component<T: Component>(&mut self, entity: Entity) -> Option<T> {
        if !self.entities.is_alive(entity) {
            return None;
        }
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
        if !self.entities.is_alive(entity) {
            return None;
        }
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

    /// Borrow a component by its type ID string (type-erased).
    pub fn get_any(&self, entity: Entity, type_id: &str) -> Option<&dyn std::any::Any> {
        if !self.entities.is_alive(entity) {
            return None;
        }
        self.storages.get(type_id)?.get_any(entity)
    }

    /// Insert a type-erased component by its type ID string.
    ///
    /// Returns `false` if the storage doesn't exist or the type doesn't match.
    pub fn set_any(
        &mut self,
        entity: Entity,
        type_id: &str,
        component: Box<dyn std::any::Any>,
    ) -> bool {
        if !self.entities.is_alive(entity) {
            return false;
        }
        let Some(storage) = self.storages.get_mut(type_id) else {
            return false;
        };
        let inserted = storage.insert_any(entity, component).is_ok();
        if inserted {
            self.remove_disabled_component_record(entity, type_id);
        }
        inserted
    }

    /// Serialise a component to JSON via its registered extension hooks.
    ///
    /// Returns `None` if the entity has no such component, the type has no
    /// [`ComponentRegistry`] entry, or the type has no serialise hook.
    pub fn serialize_component(&self, entity: Entity, type_id: &str) -> Option<String> {
        if !self.entities.is_alive(entity) {
            return None;
        }
        let registry = self.component_registry.as_ref()?;
        let ext = registry.get(type_id)?;
        let ser_fn = ext.serialize?;
        let any_ref = self.storages.get(type_id)?.get_any(entity)?;
        let fields = ser_fn(any_ref);
        serde_json::to_string(&fields).ok()
    }

    /// Deserialise a component from JSON via its registered extension hooks.
    ///
    /// Returns `false` if the component cannot be deserialised (no registry
    /// entry, no deserialise hook, or JSON doesn't match the schema).
    pub fn deserialize_component(&mut self, entity: Entity, type_id: &str, json: &str) -> bool {
        if !self.entities.is_alive(entity) {
            return false;
        }
        let registry = match self.component_registry.as_ref() {
            Some(r) => r,
            None => return false,
        };
        let ext = match registry.get(type_id) {
            Some(e) => e,
            None => return false,
        };
        let de_fn = match ext.deserialize {
            Some(f) => f,
            None => return false,
        };
        let fields: BTreeMap<String, Value> = match serde_json::from_str(json) {
            Ok(f) => f,
            Err(_) => return false,
        };
        let component = de_fn(&fields);
        let static_key = ext.meta.type_id;
        let storage = self
            .storages
            .entry(static_key)
            .or_insert_with(|| (ext.storage_factory)());
        let inserted = storage.insert_any(entity, component).is_ok();
        if inserted {
            self.remove_disabled_component_record(entity, type_id);
        }
        inserted
    }

    /// Mutably borrow a component by type.
    pub fn get_mut<T: Component>(&mut self, entity: Entity) -> Option<&mut T> {
        if !self.entities.is_alive(entity) {
            return None;
        }
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
        if !self.entities.is_alive(entity) {
            return false;
        }
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

    /// Iterate over enabled entities that have component `T`.
    ///
    /// Stored entity generations are preserved, so recycled indices cannot
    /// appear under stale handles.
    pub fn query<T: Component>(&self) -> impl Iterator<Item = (Entity, &T)> + '_ {
        if let Some(storage) = self.storages.get(T::TYPE_ID) {
            let set = storage
                .as_any()
                .downcast_ref::<SparseSet<T>>()
                .expect("storage type mismatch");
            let items: Vec<_> = set
                .iter()
                .filter(|(entity, _)| self.is_enabled(*entity))
                .collect();
            items.into_iter()
        } else {
            vec![].into_iter()
        }
    }

    /// Iterate over all entities that have component `T`, including disabled
    /// entities. Serialization and hierarchy tooling use this explicitly;
    /// runtime systems should normally use [`query`](Self::query).
    pub fn query_all<T: Component>(&self) -> impl Iterator<Item = (Entity, &T)> + '_ {
        if let Some(storage) = self.storages.get(T::TYPE_ID) {
            let set = storage
                .as_any()
                .downcast_ref::<SparseSet<T>>()
                .expect("storage type mismatch");
            let items: Vec<_> = set.iter().collect();
            items.into_iter()
        } else {
            vec![].into_iter()
        }
    }

    /// Mutably iterate over enabled entities that have component `T`.
    pub fn query_mut<T: Component>(&mut self) -> impl Iterator<Item = (Entity, &mut T)> + '_ {
        let enabled = &self.enabled;
        if let Some(storage) = self.storages.get_mut(T::TYPE_ID) {
            let set = storage
                .as_any_mut()
                .downcast_mut::<SparseSet<T>>()
                .expect("storage type mismatch");
            let items: Vec<_> = set
                .iter_mut()
                .filter(|(entity, _)| {
                    enabled
                        .get(entity.index() as usize)
                        .copied()
                        .unwrap_or(false)
                })
                .collect();
            items.into_iter()
        } else {
            vec![].into_iter()
        }
    }

    /// Mutably iterate over all entities that have component `T`, including
    /// disabled entities.
    pub fn query_all_mut<T: Component>(&mut self) -> impl Iterator<Item = (Entity, &mut T)> + '_ {
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

    fn remove_disabled_component_record(&mut self, entity: Entity, type_id: &str) {
        let idx = entity.index() as usize;
        if self.entities.is_alive(entity) && idx < self.disabled_components.len() {
            self.disabled_components[idx].remove(type_id);
        }
    }

    // ── Clear ─────────────────────────────────────────────────────────

    /// Remove all entities and components.
    pub fn clear(&mut self) {
        // A fresh generation domain prevents pre-clear handles from aliasing
        // newly allocated entities with the same slot index.
        self.entities = fresh_world_entity_manager();
        self.storages.clear();
        self.persistent_to_entity.clear();
        self.entity_to_persistent.clear();
        self.enabled.clear();
        self.disabled_components.clear();
        self.component_schema_versions.clear();
        self.entity_parents.clear();
        self.scene_settings = SceneSettings::default();
        self.scene_schema_version = SchemaVersion::new(0, 1, 0);
        self.scene_engine_version = "0.1.0".to_string();
        self.scene_id = "ecs-world".to_string();
        self.scene_name = "ECS World".to_string();
        self.scene_dependencies.clear();
        self.diagnostics_policy = DiagnosticsPolicy::Strict;
    }
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;

    struct TestComponent(u32);

    impl Component for TestComponent {
        const TYPE_ID: &'static str = "test.world_generation";
    }

    #[test]
    fn stale_world_handle_cannot_access_or_mutate_recycled_entity() {
        let mut world = World::new();
        let old = world.create_entity();
        world.add_component(old, TestComponent(1));
        assert!(world.destroy_entity(old));

        let current = world.create_entity();
        assert_eq!(current.index(), old.index());
        assert_ne!(current.generation(), old.generation());
        world.add_component(current, TestComponent(2));

        assert!(world.get::<TestComponent>(old).is_none());
        assert!(world.get_mut::<TestComponent>(old).is_none());
        assert!(!world.has::<TestComponent>(old));
        assert!(world.remove_component::<TestComponent>(old).is_none());
        world.set_enabled(old, false);

        assert!(world.is_enabled(current));
        assert_eq!(
            world.get::<TestComponent>(current).map(|value| value.0),
            Some(2)
        );
        assert_eq!(
            world.query::<TestComponent>().next().map(|item| item.0),
            Some(current)
        );
    }

    #[test]
    fn entity_handles_do_not_alias_across_worlds() {
        let mut first = World::new();
        let old = first.create_entity();
        let mut second = World::new();
        let current = second.create_entity();

        assert_eq!(old.index(), current.index());
        assert_ne!(old.generation(), current.generation());
        assert!(!second.is_alive(old));
    }

    #[test]
    fn clear_invalidates_handles_before_slots_are_reused() {
        let mut world = World::new();
        let old = world.create_entity();
        world.clear();
        let current = world.create_entity();

        assert_eq!(old.index(), current.index());
        assert_ne!(old.generation(), current.generation());
        assert!(!world.is_alive(old));
        assert!(world.is_alive(current));
    }

    #[test]
    fn guessed_generation_for_free_slot_is_rejected_by_world() {
        let mut world = World::new();
        let old = world.create_entity();
        assert!(world.destroy_entity(old));
        let guessed = Entity::new(old.index(), old.generation() + 1);

        assert!(!world.is_alive(guessed));
        assert!(world.get::<TestComponent>(guessed).is_none());
        assert!(!world.set_any(guessed, TestComponent::TYPE_ID, Box::new(TestComponent(3))));
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
}
