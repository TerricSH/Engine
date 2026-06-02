//! Gate 14 — Object Pooling (G14-F07).
//!
//! Provides [`ObjectPool`] — a reusable pool of prefab instances that
//! minimises allocation overhead by pre-allocating and recycling entities.
//!
//! # Usage
//!
//! ```ignore
//! let mut pool = ObjectPool::new(&mut world, "prefabs/bullet.prefab", &prefab, 32);
//! if let Some(bullet) = pool.spawn(&mut world) {
//!     // position the bullet…
//!     pool.despawn(&mut world, bullet);
//! }
//! ```

use crate::prefab::Prefab;
use crate::prefab_instance::instantiate_prefab;
use crate::World;

/// Callback invoked when a pooled instance is spawned (activated).
pub type SpawnCallback = fn(world: &mut World, root_entity: crate::Entity);

/// Callback invoked when a pooled instance is despawned (deactivated).
pub type DespawnCallback = fn(world: &mut World, root_entity: crate::Entity);

/// A pool of reusable prefab instances.
///
/// On construction the pool pre-allocates `prealloc_size` instances of the
/// given prefab and immediately deactivates them.  [`spawn`](ObjectPool::spawn)
/// re-activates one from the pool (or allocates a new one if empty).
/// [`despawn`](ObjectPool::despawn) deactivates and returns it.
///
/// Optional [`on_spawn`](Self::on_spawn) and [`on_despawn`](Self::on_despawn)
/// callbacks allow subsystems (script, physics, animation, audio, UI) to
/// reset component state when an instance is recycled.
pub struct ObjectPool {
    /// Asset path of the prefab this pool manages.
    pub prefab_asset: String,
    /// Prefab asset used for instantiation (kept for creating new instances).
    prefab: Prefab,
    /// Entities that are currently active (spawned but not despawned).
    active: Vec<EntityPoolInfo>,
    /// Root entities that are available for reuse.
    inactive: Vec<EntityPoolInfo>,
    /// Number of instances pre-allocated at construction.
    prealloc_size: u32,
    /// Optional callback fired after an instance is activated.
    on_spawn: Option<SpawnCallback>,
    /// Optional callback fired before an instance is deactivated.
    on_despawn: Option<DespawnCallback>,
}

/// Metadata for a single pooled entity hierarchy.
#[derive(Clone, Debug)]
struct EntityPoolInfo {
    /// Root entity of the prefab instantiation.
    root: crate::Entity,
    /// All entities in the prefab instantiation (including root).
    all: Vec<crate::Entity>,
}

impl ObjectPool {
    /// Create a new object pool.
    ///
    /// Immediately pre-allocates `prealloc_size` instances of `prefab` and
    /// deactivates them so they are ready for reuse.
    ///
    /// # Panics
    /// Panics if the prefab has an empty hierarchy.
    pub fn new(
        world: &mut World,
        prefab_asset: impl Into<String>,
        prefab: &Prefab,
        prealloc_size: u32,
    ) -> Self {
        let asset: String = prefab_asset.into();
        let mut pool = Self {
            prefab_asset: asset,
            prefab: prefab.clone(),
            active: Vec::new(),
            inactive: Vec::new(),
            prealloc_size,
            on_spawn: None,
            on_despawn: None,
        };

        // Pre-allocate and immediately deactivate.
        for _ in 0..prealloc_size {
            let info = pool.instantiate_new(world);
            pool.deactivate_all(info.all.iter().copied(), world);
            pool.inactive.push(info);
        }

        pool
    }

    /// Spawn (activate) an instance from the pool.
    ///
    /// If an inactive instance is available it is returned; otherwise a new
    /// instance is created on demand.  Returns `None` only if instantiation
    /// of the prefab fails (which should not happen after construction).
    pub fn spawn(&mut self, world: &mut World) -> Option<crate::Entity> {
        let info = if let Some(info) = self.inactive.pop() {
            // Reactivate the existing instance.
            self.activate_all(info.all.iter().copied(), world);
            info
        } else {
            // Pool exhausted — create a new instance on demand.
            self.instantiate_new(world)
        };

        // Fire the on_spawn callback (e.g. to reset script / physics state).
        if let Some(cb) = self.on_spawn {
            cb(world, info.root);
        }

        let root = info.root;
        self.active.push(info);
        Some(root)
    }

    /// Despawn (deactivate) an instance and return it to the pool.
    ///
    /// Returns `false` if the entity was not found among the active entries
    /// (i.e. it does not belong to this pool or is already despawned).
    pub fn despawn(&mut self, world: &mut World, entity: crate::Entity) -> bool {
        let pos = self.active.iter().position(|info| info.root == entity);
        match pos {
            Some(idx) => {
                let info = self.active.remove(idx);
                // Fire the on_despawn callback before deactivating.
                if let Some(cb) = self.on_despawn {
                    cb(world, info.root);
                }
                self.deactivate_all(info.all.iter().copied(), world);
                self.inactive.push(info);
                true
            }
            None => false,
        }
    }

    /// Reset the pool: deactivate all active entities, returning them to the
    /// inactive pool.
    pub fn reset_all(&mut self, world: &mut World) {
        let active: Vec<_> = self.active.drain(..).collect();
        for info in active {
            self.deactivate_all(info.all.iter().copied(), world);
            self.inactive.push(info);
        }
    }

    /// Number of currently active (spawned) instances.
    pub fn active_count(&self) -> usize {
        self.active.len()
    }

    /// Set a callback that fires when an instance is spawned (activated).
    /// Useful for resetting script, physics, animation, audio, or UI state.
    pub fn set_on_spawn(&mut self, callback: SpawnCallback) {
        self.on_spawn = Some(callback);
    }

    /// Set a callback that fires when an instance is despawned (deactivated).
    pub fn set_on_despawn(&mut self, callback: DespawnCallback) {
        self.on_despawn = Some(callback);
    }

    /// Number of available (inactive) instances.
    pub fn inactive_count(&self) -> usize {
        self.inactive.len()
    }

    /// Total number of instances managed by the pool.
    pub fn total_count(&self) -> usize {
        self.active.len() + self.inactive.len()
    }

    /// The pre-allocation size configured at construction.
    pub fn prealloc_size(&self) -> u32 {
        self.prealloc_size
    }

    // ── Internal helpers ──────────────────────────────────────────────

    /// Instantiate the prefab and return pool info for the new instance.
    fn instantiate_new(&self, world: &mut World) -> EntityPoolInfo {
        let result = instantiate_prefab(world, &self.prefab, None)
            .expect("ObjectPool: prefab instantiation failed");
        EntityPoolInfo {
            root: result.root_entity,
            all: result.all_entities,
        }
    }

    /// Activate (enable) all entities in an iterator.
    fn activate_all(&self, entities: impl Iterator<Item = crate::Entity>, world: &mut World) {
        for entity in entities {
            world.set_enabled(entity, true);
        }
    }

    /// Deactivate (disable) all entities in an iterator.
    fn deactivate_all(&self, entities: impl Iterator<Item = crate::Entity>, world: &mut World) {
        for entity in entities {
            world.set_enabled(entity, false);
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Tests
// ════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prefab_instance::PrefabInstanceRef;
    use crate::scene::{ComponentRecord, EntityRecord};
    use engine_serialize::{AssetId, SchemaVersion};
    use std::collections::BTreeMap;

    fn make_transform_record() -> ComponentRecord {
        let mut fields = BTreeMap::new();
        fields.insert(
            "translation".to_string(),
            engine_serialize::Value::Vec3([0.0; 3]),
        );
        fields.insert(
            "rotation".to_string(),
            engine_serialize::Value::Quat([0.0, 0.0, 0.0, 1.0]),
        );
        fields.insert(
            "scale".to_string(),
            engine_serialize::Value::Vec3([1.0, 1.0, 1.0]),
        );
        ComponentRecord {
            schema_version: SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields,
        }
    }

    fn sample_prefab() -> Prefab {
        let mut prefab = Prefab::new(AssetId::new("prefabs/pool_item.prefab"));
        let mut components = BTreeMap::new();
        components.insert("engine.transform".to_string(), make_transform_record());
        prefab.add_entity(EntityRecord {
            persistent_id: "ent-pool_root".to_string(),
            parent: None,
            name: Some("PoolItem".to_string()),
            enabled: true,
            components,
        });
        prefab
    }

    #[test]
    fn pool_preallocates_expected_count() {
        let mut world = World::new();
        let prefab = sample_prefab();
        let pool = ObjectPool::new(&mut world, "prefabs/pool_item.prefab", &prefab, 5);

        assert_eq!(pool.inactive_count(), 5);
        assert_eq!(pool.active_count(), 0);
        assert_eq!(pool.total_count(), 5);
        assert_eq!(pool.prealloc_size(), 5);

        // All pre-allocated entities should be disabled
        // (we can't easily check this without spawning them first)
    }

    #[test]
    fn pool_spawn_returns_entity_from_inactive() {
        let mut world = World::new();
        let prefab = sample_prefab();
        let mut pool = ObjectPool::new(&mut world, "prefabs/pool_item.prefab", &prefab, 3);

        let entity = pool.spawn(&mut world).expect("should spawn");
        assert!(world.is_alive(entity));
        assert!(world.is_enabled(entity));
        assert_eq!(pool.active_count(), 1);
        assert_eq!(pool.inactive_count(), 2);

        // Entity should have PrefabInstanceRef
        assert!(world.get::<PrefabInstanceRef>(entity).is_some());
    }

    #[test]
    fn pool_spawn_and_despawn_cycle() {
        let mut world = World::new();
        let prefab = sample_prefab();
        let mut pool = ObjectPool::new(&mut world, "prefabs/pool_item.prefab", &prefab, 2);

        let entity = pool.spawn(&mut world).expect("spawn");
        assert!(world.is_enabled(entity));
        assert_eq!(pool.active_count(), 1);

        // Despawn
        let ok = pool.despawn(&mut world, entity);
        assert!(ok);
        assert!(!world.is_enabled(entity));
        assert_eq!(pool.active_count(), 0);
        assert_eq!(pool.inactive_count(), 2);

        // Re-spawn (should reuse the despawned entity)
        let entity2 = pool.spawn(&mut world).expect("spawn again");
        assert!(world.is_enabled(entity2));
        assert_eq!(pool.active_count(), 1);
        assert_eq!(pool.inactive_count(), 1);
    }

    #[test]
    fn pool_despawn_unknown_entity_returns_false() {
        let mut world = World::new();
        let prefab = sample_prefab();
        let mut pool = ObjectPool::new(&mut world, "prefabs/pool_item.prefab", &prefab, 1);

        let unknown = world.create_entity();
        assert!(!pool.despawn(&mut world, unknown));
    }

    #[test]
    fn pool_spawn_from_empty_pool_creates_new() {
        let mut world = World::new();
        let prefab = sample_prefab();
        // Preallocate 0 so the pool starts empty.
        let mut pool = ObjectPool::new(&mut world, "prefabs/pool_item.prefab", &prefab, 0);

        assert_eq!(pool.total_count(), 0);

        let entity = pool.spawn(&mut world).expect("should spawn new");
        assert!(world.is_alive(entity));
        assert!(world.is_enabled(entity));
        // The pool should now have 1 active and 0 inactive
        assert_eq!(pool.active_count(), 1);
        assert_eq!(pool.inactive_count(), 0);
        assert_eq!(pool.total_count(), 1);
    }

    #[test]
    fn pool_reset_all_deactivates_all_active() {
        let mut world = World::new();
        let prefab = sample_prefab();
        let mut pool = ObjectPool::new(&mut world, "prefabs/pool_item.prefab", &prefab, 3);

        // Spawn all 3
        let e1 = pool.spawn(&mut world).unwrap();
        let e2 = pool.spawn(&mut world).unwrap();
        let e3 = pool.spawn(&mut world).unwrap();

        assert_eq!(pool.active_count(), 3);
        assert_eq!(pool.inactive_count(), 0);

        // Reset
        pool.reset_all(&mut world);

        assert_eq!(pool.active_count(), 0);
        assert_eq!(pool.inactive_count(), 3);

        // All original entities should now be disabled
        assert!(!world.is_enabled(e1));
        assert!(!world.is_enabled(e2));
        assert!(!world.is_enabled(e3));

        // They should be reusable
        let _e4 = pool.spawn(&mut world).unwrap();
        assert_eq!(pool.active_count(), 1);
        assert_eq!(pool.inactive_count(), 2);
    }

    #[test]
    fn pool_spawn_maintains_prefab_instance_ref() {
        let mut world = World::new();
        let prefab = sample_prefab();
        let mut pool = ObjectPool::new(&mut world, "prefabs/pool_item.prefab", &prefab, 1);

        let entity = pool.spawn(&mut world).unwrap();
        let instance_ref = world.get::<PrefabInstanceRef>(entity).unwrap();
        assert_eq!(instance_ref.source_asset, "prefabs/pool_item.prefab");
    }
}
