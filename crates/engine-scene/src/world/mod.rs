use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use crate::component::ComponentStorageDyn;
use crate::registry::ComponentRegistry;
use crate::scene::{ComponentRecord, DiagnosticsPolicy, SceneSettings};
use crate::{components::Transform, Component, Entity, EntityManager, SparseSet};
use engine_serialize::{
    AssetId, ComponentTypeId, EngineVersion, PersistentId, SchemaVersion, Value,
};
use thiserror::Error;

pub(crate) mod merge;
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

/// Failure returned when creating a runtime entity with a persistent ID.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum PersistentEntityCreateError {
    #[error("persistent entity id cannot be empty")]
    EmptyId,
    #[error("persistent entity id '{0}' already exists")]
    DuplicateId(String),
    #[error("entity handle is stale")]
    StaleEntity,
    #[error("entity already has persistent id '{0}'")]
    AlreadyPersistent(String),
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
    /// Runtime world origin for periodic origin shifting (ENG-01 Phase 2).
    ///
    /// `Transform.translation` values (and every other f32 world-space runtime
    /// state) are stored **relative** to this origin: the logical position of
    /// an entity is `world_origin + translation`. The origin starts at zero,
    /// is advanced only by [`World::shift_world_origin`], and is intentionally
    /// *not* serialised — scene files store origin-relative coordinates, so a
    /// freshly loaded world always starts at a zero origin.
    pub(crate) world_origin: [f64; 3],
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
            world_origin: [0.0; 3],
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

    /// Mutate the scene-level settings of a live world.
    ///
    /// Changes take effect on the next extraction/system pass; they are not
    /// written back to the scene file the world was loaded from.
    pub fn scene_settings_mut(&mut self) -> &mut SceneSettings {
        &mut self.scene_settings
    }

    // ── World origin (ENG-01 Phase 2) ──────────────────────────────────

    /// Current runtime world origin.
    ///
    /// Every `Transform.translation` is stored **relative** to this origin;
    /// the logical position of an entity is `world_origin + translation`
    /// (resolved through its parent chain). The origin starts at zero and
    /// advances only through [`Self::shift_world_origin`]. It is runtime-only
    /// state: scene serialisation stores origin-relative coordinates, so
    /// loading a scene always starts at a zero origin.
    pub fn world_origin(&self) -> [f64; 3] {
        self.world_origin
    }

    /// Shift the world origin by `delta`, preserving logical positions.
    ///
    /// `delta` is subtracted from the translation of every root `Transform`
    /// (an entity whose parent chain carries no `Transform` above it —
    /// children follow their roots through the hierarchy) and added to
    /// [`Self::world_origin`], so the logical position
    /// `world_origin + world_position` of every entity is unchanged. Disabled
    /// entities are shifted too: their stored transforms are world-space
    /// state like any other.
    ///
    /// Only the ECS transforms and the origin are touched here. The host
    /// (e.g. `GameLoop`) is responsible for sweeping the remaining f32
    /// world-space runtime state — physics bodies, character controllers,
    /// navigation agents, and point gravity sources — in the same frame
    /// boundary so the shift is observed atomically.
    ///
    /// Returns the number of root transforms that were translated.
    pub fn shift_world_origin(&mut self, delta: [f64; 3]) -> usize {
        let offset = glam::Vec3::new(delta[0] as f32, delta[1] as f32, delta[2] as f32);
        // A root is an entity whose nearest ancestor chain carries no
        // Transform: either it has no parent at all, or its parent entity
        // has no Transform (treated as an identity root by extraction).
        let roots: Vec<Entity> = self
            .query_all::<Transform>()
            .filter_map(|(entity, transform)| match transform.parent {
                Some(parent) => (!self.has::<Transform>(parent)).then_some(entity),
                None => Some(entity),
            })
            .collect();
        let mut shifted = 0;
        for entity in roots {
            if let Some(transform) = self.get_mut::<Transform>(entity) {
                transform.translation -= offset;
                shifted += 1;
            }
        }
        self.world_origin = [
            self.world_origin[0] + delta[0],
            self.world_origin[1] + delta[1],
            self.world_origin[2] + delta[2],
        ];
        shifted
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

    /// Create a runtime entity and atomically assign its persistent scene ID.
    ///
    /// Empty and duplicate IDs are rejected before an ECS slot is allocated,
    /// leaving the World unchanged. The returned generational handle remains
    /// internal runtime state; integrations should retain `persistent_id` and
    /// resolve it again with [`Self::entity_by_persistent_id`].
    pub fn create_persistent_entity(
        &mut self,
        persistent_id: impl Into<String>,
    ) -> Result<Entity, PersistentEntityCreateError> {
        let persistent_id = persistent_id.into();
        if persistent_id.is_empty() {
            return Err(PersistentEntityCreateError::EmptyId);
        }
        if self.persistent_to_entity.contains_key(&persistent_id) {
            return Err(PersistentEntityCreateError::DuplicateId(persistent_id));
        }

        let entity = self.create_entity();
        let index = entity.index() as usize;
        if self.entity_to_persistent.len() <= index {
            self.entity_to_persistent.resize(index + 1, None);
        }
        debug_assert!(self.entity_to_persistent[index].is_none());
        self.entity_to_persistent[index] = Some(persistent_id.clone());
        self.persistent_to_entity.insert(persistent_id, entity);
        Ok(entity)
    }

    /// Assign a persistent scene ID to an already-live entity.
    ///
    /// This is the post-hoc counterpart of [`Self::create_persistent_entity`]
    /// for instantiation paths that allocate ECS entities before their
    /// script-visible IDs are known (for example prefab instantiation). Empty
    /// and duplicate IDs are rejected before any mapping changes, and a stale
    /// or already-identified entity is rejected as well.
    pub fn assign_persistent_id(
        &mut self,
        entity: Entity,
        persistent_id: impl Into<String>,
    ) -> Result<(), PersistentEntityCreateError> {
        let persistent_id = persistent_id.into();
        if persistent_id.is_empty() {
            return Err(PersistentEntityCreateError::EmptyId);
        }
        if self.persistent_to_entity.contains_key(&persistent_id) {
            return Err(PersistentEntityCreateError::DuplicateId(persistent_id));
        }
        if !self.entities.is_alive(entity) {
            return Err(PersistentEntityCreateError::StaleEntity);
        }
        let index = entity.index() as usize;
        if index < self.entity_to_persistent.len() && self.entity_to_persistent[index].is_some() {
            return Err(PersistentEntityCreateError::AlreadyPersistent(
                self.entity_to_persistent[index]
                    .clone()
                    .expect("checked to be Some"),
            ));
        }
        if self.entity_to_persistent.len() <= index {
            self.entity_to_persistent.resize(index + 1, None);
        }
        self.entity_to_persistent[index] = Some(persistent_id.clone());
        self.persistent_to_entity.insert(persistent_id, entity);
        Ok(())
    }

    /// Destroy an entity and all of its components.
    ///
    /// Returns `false` if the entity handle is stale.
    pub fn destroy_entity(&mut self, entity: Entity) -> bool {
        if !self.entities.is_alive(entity) {
            return false;
        }

        // Surviving children become roots. Leaving a stale generational
        // handle here would make hierarchy extraction fail after an otherwise
        // valid runtime entity destruction.
        let children = self
            .query_all::<Transform>()
            .filter_map(|(child, transform)| (transform.parent == Some(entity)).then_some(child))
            .collect::<Vec<_>>();
        for child in children {
            if let Some(transform) = self.get_mut::<Transform>(child) {
                transform.parent = None;
            }
        }
        if let Some(persistent_id) = self.persistent_id(entity).map(str::to_owned) {
            for parent in &mut self.entity_parents {
                if parent.as_deref() == Some(persistent_id.as_str()) {
                    *parent = None;
                }
            }
        }

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

    /// Resolve a persistent scene ID to its current live ECS handle.
    ///
    /// Handles are intentionally not stable across scene reloads. Editor and
    /// scripting hosts should retain the persistent ID and call this method
    /// again after every World replacement instead of caching an entity index.
    pub fn entity_by_persistent_id(&self, id: &str) -> Option<Entity> {
        self.persistent_to_entity
            .get(id)
            .copied()
            .filter(|entity| self.entities.is_alive(*entity))
    }

    /// Persistent ID of the entity's effective hierarchy parent, if any.
    ///
    /// Mirrors the serialization rule used by [`to_scene`](World::to_scene):
    /// an entity with a `Transform` resolves through its `parent` handle
    /// (a parent without a persistent ID yields `None`), while an entity
    /// without a `Transform` falls back to the persistent side-table link
    /// recorded at load/merge time.
    pub fn parent_persistent_id(&self, entity: Entity) -> Option<&str> {
        if !self.entities.is_alive(entity) {
            return None;
        }
        if self.has::<Transform>(entity) {
            return self
                .get::<Transform>(entity)
                .and_then(|transform| transform.parent)
                .and_then(|parent| self.persistent_id(parent));
        }
        let idx = entity.index() as usize;
        self.entity_parents
            .get(idx)
            .and_then(|parent| parent.as_deref())
    }

    /// Iterate every live entity that has a persistent scene identifier.
    ///
    /// Persistent ids, rather than generational ECS handles, are the stable
    /// identity boundary exposed to editor and scripting integrations.
    pub fn persistent_entities(&self) -> impl Iterator<Item = (&str, Entity)> + '_ {
        self.persistent_to_entity.iter().filter_map(|(id, entity)| {
            self.entities
                .is_alive(*entity)
                .then_some((id.as_str(), *entity))
        })
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
        self.world_origin = [0.0; 3];
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
}
