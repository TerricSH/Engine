//! Transactional object pooling for reusable prefab instances.

use std::panic::{catch_unwind, AssertUnwindSafe};

use crate::prefab::Prefab;
use crate::prefab_instance::{
    instantiate_prefab, validate_prefab_for_instantiation, PrefabInstantiateError,
};
use crate::{Entity, World};

/// Callback invoked when a pooled instance is activated.
pub type SpawnCallback = fn(world: &mut World, root_entity: Entity);

/// Callback invoked before a pooled instance is deactivated.
pub type DespawnCallback = fn(world: &mut World, root_entity: Entity);

/// A recoverable object-pool operation failure.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ObjectPoolError {
    #[error(transparent)]
    Prefab(#[from] PrefabInstantiateError),
    #[error("pooled instance rooted at {root:?} is no longer alive in this World")]
    InvalidatedInstance { root: Entity },
    #[error("{phase} callback panicked for pooled instance rooted at {root:?}")]
    CallbackPanicked { phase: &'static str, root: Entity },
}

/// A pool of reusable prefab hierarchies.
///
/// Construction and growth propagate prefab failures instead of panicking.
/// A failed preallocation rolls back every entity created by that constructor.
pub struct ObjectPool {
    /// Asset path of the prefab this pool manages.
    pub prefab_asset: String,
    prefab: Prefab,
    active: Vec<EntityPoolInfo>,
    inactive: Vec<EntityPoolInfo>,
    prealloc_size: u32,
    on_spawn: Option<SpawnCallback>,
    on_despawn: Option<DespawnCallback>,
}

#[derive(Clone, Debug)]
struct EntityPoolInfo {
    root: Entity,
    all: Vec<Entity>,
}

impl ObjectPool {
    /// Create a pool and transactionally preallocate `prealloc_size` instances.
    pub fn new(
        world: &mut World,
        prefab_asset: impl Into<String>,
        prefab: &Prefab,
        prealloc_size: u32,
    ) -> Result<Self, ObjectPoolError> {
        // Validate even when prealloc_size is zero so an invalid pool cannot be
        // constructed and fail much later on its first spawn.
        validate_prefab_for_instantiation(world, prefab, None)?;

        let mut pool = Self {
            prefab_asset: prefab_asset.into(),
            prefab: prefab.clone(),
            active: Vec::new(),
            inactive: Vec::new(),
            prealloc_size,
            on_spawn: None,
            on_despawn: None,
        };

        for _ in 0..prealloc_size {
            match pool.instantiate_new(world) {
                Ok(info) => {
                    Self::set_enabled(&info, world, false);
                    pool.inactive.push(info);
                }
                Err(error) => {
                    for info in pool.inactive.drain(..).rev() {
                        Self::destroy_instance(&info, world);
                    }
                    return Err(error);
                }
            }
        }

        Ok(pool)
    }

    /// Activate an existing instance or transactionally instantiate a new one.
    pub fn spawn(&mut self, world: &mut World) -> Result<Entity, ObjectPoolError> {
        if let Some(info) = self.inactive.last() {
            if !Self::is_alive(info, world) {
                return Err(ObjectPoolError::InvalidatedInstance { root: info.root });
            }
        }

        let info = if let Some(info) = self.inactive.pop() {
            Self::set_enabled(&info, world, true);
            info
        } else {
            self.instantiate_new(world)?
        };

        if let Some(callback) = self.on_spawn {
            if catch_unwind(AssertUnwindSafe(|| callback(world, info.root))).is_err() {
                Self::destroy_instance(&info, world);
                return Err(ObjectPoolError::CallbackPanicked {
                    phase: "spawn",
                    root: info.root,
                });
            }
        }
        if !Self::is_alive(&info, world) {
            Self::destroy_instance(&info, world);
            return Err(ObjectPoolError::InvalidatedInstance { root: info.root });
        }

        let root = info.root;
        self.active.push(info);
        Ok(root)
    }

    /// Deactivate an active instance and return it to the pool.
    ///
    /// Returns `Ok(false)` when `entity` is not an active root in this pool.
    pub fn despawn(&mut self, world: &mut World, entity: Entity) -> Result<bool, ObjectPoolError> {
        let Some(index) = self.active.iter().position(|info| info.root == entity) else {
            return Ok(false);
        };
        let info = self.active.remove(index);
        if !Self::is_alive(&info, world) {
            Self::destroy_instance(&info, world);
            return Err(ObjectPoolError::InvalidatedInstance { root: info.root });
        }

        if let Some(callback) = self.on_despawn {
            if catch_unwind(AssertUnwindSafe(|| callback(world, info.root))).is_err() {
                Self::destroy_instance(&info, world);
                return Err(ObjectPoolError::CallbackPanicked {
                    phase: "despawn",
                    root: info.root,
                });
            }
        }
        if !Self::is_alive(&info, world) {
            Self::destroy_instance(&info, world);
            return Err(ObjectPoolError::InvalidatedInstance { root: info.root });
        }

        Self::set_enabled(&info, world, false);
        self.inactive.push(info);
        Ok(true)
    }

    /// Deactivate every active instance.
    pub fn reset_all(&mut self, world: &mut World) -> Result<(), ObjectPoolError> {
        let active = std::mem::take(&mut self.active);
        let mut first_error = None;
        for info in active {
            if Self::is_alive(&info, world) {
                Self::set_enabled(&info, world, false);
                self.inactive.push(info);
            } else {
                Self::destroy_instance(&info, world);
                first_error.get_or_insert(ObjectPoolError::InvalidatedInstance { root: info.root });
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    pub fn active_count(&self) -> usize {
        self.active.len()
    }

    pub fn inactive_count(&self) -> usize {
        self.inactive.len()
    }

    pub fn total_count(&self) -> usize {
        self.active.len() + self.inactive.len()
    }

    pub fn prealloc_size(&self) -> u32 {
        self.prealloc_size
    }

    pub fn set_on_spawn(&mut self, callback: SpawnCallback) {
        self.on_spawn = Some(callback);
    }

    pub fn set_on_despawn(&mut self, callback: DespawnCallback) {
        self.on_despawn = Some(callback);
    }

    fn instantiate_new(&self, world: &mut World) -> Result<EntityPoolInfo, ObjectPoolError> {
        let result = instantiate_prefab(world, &self.prefab, None)?;
        Ok(EntityPoolInfo {
            root: result.root_entity,
            all: result.all_entities,
        })
    }

    fn is_alive(info: &EntityPoolInfo, world: &World) -> bool {
        world.is_alive(info.root) && info.all.iter().all(|entity| world.is_alive(*entity))
    }

    fn set_enabled(info: &EntityPoolInfo, world: &mut World, enabled: bool) {
        for entity in &info.all {
            world.set_enabled(*entity, enabled);
        }
    }

    fn destroy_instance(info: &EntityPoolInfo, world: &mut World) {
        for entity in info.all.iter().rev() {
            let _ = world.destroy_entity(*entity);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use engine_serialize::{AssetId, SchemaVersion, Value};

    use super::*;
    use crate::prefab_instance::PrefabInstanceRef;
    use crate::scene::{ComponentRecord, EntityRecord};

    fn transform_record() -> ComponentRecord {
        let mut fields = BTreeMap::new();
        fields.insert("translation".to_string(), Value::Vec3([0.0; 3]));
        fields.insert("rotation".to_string(), Value::Quat([0.0, 0.0, 0.0, 1.0]));
        fields.insert("scale".to_string(), Value::Vec3([1.0; 3]));
        ComponentRecord {
            schema_version: SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields,
        }
    }

    fn sample_prefab() -> Prefab {
        let mut prefab = Prefab::new(AssetId::new("prefabs/pool_item.prefab"));
        let mut components = BTreeMap::new();
        components.insert("engine.transform".to_string(), transform_record());
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
    fn pool_preallocates_and_reuses_instances() {
        let mut world = World::new();
        let prefab = sample_prefab();
        let mut pool = ObjectPool::new(&mut world, "prefabs/pool_item.prefab", &prefab, 2)
            .expect("valid pool");

        assert_eq!(pool.inactive_count(), 2);
        let entity = pool.spawn(&mut world).expect("spawn");
        assert!(world.is_enabled(entity));
        assert!(world.get::<PrefabInstanceRef>(entity).is_some());
        assert!(pool.despawn(&mut world, entity).expect("despawn"));
        assert!(!world.is_enabled(entity));
        assert_eq!(pool.inactive_count(), 2);
        assert_eq!(pool.spawn(&mut world).expect("reuse"), entity);
    }

    #[test]
    fn invalid_prefab_is_rejected_even_without_preallocation() {
        let mut world = World::new();
        let prefab = Prefab::new(AssetId::new("prefabs/empty.prefab"));
        let alive_before = world.alive_count();

        let error = ObjectPool::new(&mut world, "prefabs/empty.prefab", &prefab, 0)
            .err()
            .expect("empty prefab must fail");

        assert!(matches!(
            error,
            ObjectPoolError::Prefab(PrefabInstantiateError::EmptyHierarchy { .. })
        ));
        assert_eq!(world.alive_count(), alive_before);
    }

    #[test]
    fn externally_destroyed_inactive_instance_fails_without_panic() {
        let mut world = World::new();
        let prefab = sample_prefab();
        let mut pool = ObjectPool::new(&mut world, "prefabs/pool_item.prefab", &prefab, 1)
            .expect("valid pool");
        let entity = pool.spawn(&mut world).expect("spawn");
        pool.despawn(&mut world, entity).expect("despawn");
        assert!(world.destroy_entity(entity));

        assert!(matches!(
            pool.spawn(&mut world),
            Err(ObjectPoolError::InvalidatedInstance { root }) if root == entity
        ));
    }

    #[test]
    fn reset_deactivates_all_active_instances() {
        let mut world = World::new();
        let prefab = sample_prefab();
        let mut pool = ObjectPool::new(&mut world, "prefabs/pool_item.prefab", &prefab, 2)
            .expect("valid pool");
        let first = pool.spawn(&mut world).expect("first");
        let second = pool.spawn(&mut world).expect("second");

        pool.reset_all(&mut world).expect("reset");

        assert!(!world.is_enabled(first));
        assert!(!world.is_enabled(second));
        assert_eq!(pool.active_count(), 0);
        assert_eq!(pool.inactive_count(), 2);
    }

    fn panic_on_spawn(_: &mut World, _: Entity) {
        panic!("callback failure");
    }

    #[test]
    fn callback_panic_is_contained_and_instance_is_removed() {
        let mut world = World::new();
        let prefab = sample_prefab();
        let mut pool = ObjectPool::new(&mut world, "prefabs/pool_item.prefab", &prefab, 1)
            .expect("valid pool");
        pool.set_on_spawn(panic_on_spawn);

        assert!(matches!(
            pool.spawn(&mut world),
            Err(ObjectPoolError::CallbackPanicked { phase: "spawn", .. })
        ));
        assert_eq!(pool.total_count(), 0);
        assert_eq!(world.alive_count(), 0);
    }
}
