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

/// Invalid runtime origin supplied while restoring a save-game snapshot.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum WorldOriginRestoreError {
    #[error("world origin must contain only finite values")]
    NonFinite,
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

    /// Restore an already-rebased world's runtime origin without translating
    /// any component data.
    ///
    /// This is intentionally distinct from [`Self::shift_world_origin`].
    /// Save-game snapshots store both the origin and every origin-relative
    /// component value, so translating the values again during restore would
    /// double-apply the shift. Normal scene loading should never call this.
    pub fn restore_world_origin(
        &mut self,
        origin: [f64; 3],
    ) -> Result<(), WorldOriginRestoreError> {
        if !origin.iter().all(|value| value.is_finite()) {
            return Err(WorldOriginRestoreError::NonFinite);
        }
        self.world_origin = origin;
        Ok(())
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
include!("tests/lifecycle.rs");
#[cfg(test)]
include!("tests/world.rs");
