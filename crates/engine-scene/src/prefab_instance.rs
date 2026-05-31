//! Gate 14 — Prefab Runtime: instantiation and prefab instance tracking.
//!
//! Provides [`PrefabInstanceRef`] (an ECS component attached to every entity
//! spawned from a prefab), together with the instantiation functions that
//! convert a [`Prefab`] data asset into live entities in the [`World`].

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

use engine_serialize::{SchemaVersion, Value};
use serde::{Deserialize, Serialize};

use crate::component::Component;
use crate::prefab::Prefab;
use crate::{Entity, World};

// ── PrefabInstanceRef component ────────────────────────────────────────────

/// ECS component that marks an entity as having been spawned from a prefab.
///
/// Every entity produced by [`instantiate_prefab`] receives one of these so
/// that the override system and tooling can identify the original prefab asset
/// and instance group.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PrefabInstanceRef {
    /// Asset path of the source prefab (e.g. `"prefabs/enemies/goblin.prefab"`).
    pub source_asset: String,
    /// Unique identifier for this particular instantiation (all entities in
    /// the same instantiation share this ID).
    pub instance_id: String,
    /// Persistent ID of this specific entity within the prefab hierarchy.
    /// Used by the override system to target individual entities.
    pub entity_persistent_id: String,
    /// Schema version of the prefab that was instantiated.
    pub schema_version: SchemaVersion,
}

impl Component for PrefabInstanceRef {
    const TYPE_ID: &'static str = "engine.prefab_instance_ref";
}

// ── Instantiation result ───────────────────────────────────────────────────

/// The result of a single prefab instantiation.
pub struct PrefabInstantiateResult {
    /// The root entity (first entity in the hierarchy with no parent).
    pub root_entity: Entity,
    /// Every entity that was created, in hierarchy order.
    pub all_entities: Vec<Entity>,
}

// ── Instance ID generation ─────────────────────────────────────────────────

static INSTANCE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Generate a unique, monotonically-increasing instance identifier.
fn generate_instance_id() -> String {
    let count = INSTANCE_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("inst_{}", count)
}

// ── Instantiation ──────────────────────────────────────────────────────────

/// Instantiate a [`Prefab`] into the world.
///
/// For each entity in the prefab hierarchy:
///
/// 1. A new [`Entity`] is allocated.
/// 2. Components from the [`EntityRecord`] are created, with
///    [`Prefab.component_defaults`] overlaid on top.
/// 3. A [`PrefabInstanceRef`] is attached.
/// 4. Parent–child relationships are resolved via [`Transform.parent`].
///
/// Returns the root entity and the full list of created entities.
pub fn instantiate_prefab(world: &mut World, prefab: &Prefab) -> Result<PrefabInstantiateResult, String> {
    if prefab.hierarchy.is_empty() {
        return Err("Cannot instantiate a prefab with an empty hierarchy".to_string());
    }

    let instance_id = generate_instance_id();

    // ── Pass 1: allocate all entities ──────────────────────────────────
    let mut entity_map: BTreeMap<String, Entity> = BTreeMap::new();
    for record in &prefab.hierarchy {
        let entity = world.create_entity();
        entity_map.insert(record.persistent_id.clone(), entity);
    }

    // ── Pass 2: populate components ────────────────────────────────────
    for record in &prefab.hierarchy {
        let Some(&entity) = entity_map.get(&record.persistent_id) else {
            continue;
        };

        // Name component from EntityRecord.name.
        if let Some(ref name) = record.name {
            world.add_component(entity, crate::components::Name(name.clone()));
        }

        // Typed components from EntityRecord.components, overlaid with
        // component_defaults.
        for (comp_type_id, comp_record) in &record.components {
            if !comp_record.enabled {
                continue;
            }

            let merged_fields = merge_defaults(
                &comp_record.fields,
                prefab.component_defaults.get(comp_type_id),
            );

            world.populate_component(entity, comp_type_id, &merged_fields);
        }

        // PrefabInstanceRef on every entity.
        world.add_component(
            entity,
            PrefabInstanceRef {
                source_asset: prefab.source_asset.id.clone(),
                instance_id: instance_id.clone(),
                entity_persistent_id: record.persistent_id.clone(),
                schema_version: prefab.schema_version,
            },
        );
    }

    // ── Pass 3: resolve parent–child relationships ─────────────────────
    for record in &prefab.hierarchy {
        let Some(&entity) = entity_map.get(&record.persistent_id) else {
            continue;
        };
        if let Some(ref parent_pid) = record.parent {
            if let Some(&parent_entity) = entity_map.get(parent_pid) {
                if let Some(transform) = world.get_mut::<crate::components::Transform>(entity) {
                    transform.parent = Some(parent_entity);
                }
            }
        }
    }

    // ── Determine root entity ──────────────────────────────────────────
    let root_pid = prefab
        .hierarchy
        .iter()
        .find(|r| r.parent.is_none())
        .map(|r| &r.persistent_id)
        .or_else(|| prefab.hierarchy.first().map(|r| &r.persistent_id))
        .ok_or_else(|| "Prefab hierarchy is empty".to_string())?;

    let root_entity = entity_map
        .remove(root_pid)
        .ok_or_else(|| "Root entity not found in entity map".to_string())?;

    let mut all_entities: Vec<Entity> = entity_map.into_values().collect();
    all_entities.insert(0, root_entity);

    Ok(PrefabInstantiateResult {
        root_entity,
        all_entities,
    })
}

/// Load a prefab from an asset registry and instantiate it.
///
/// `registry` must implement [`PrefabLoad`] so that prefab assets can be
/// resolved by their string identifier.
pub fn instantiate_prefab_from_asset(
    world: &mut World,
    registry: &dyn PrefabLoad,
    asset_id: &str,
) -> Result<PrefabInstantiateResult, String> {
    let prefab = registry
        .load_prefab(asset_id)
        .ok_or_else(|| format!("Prefab asset '{asset_id}' not found in registry"))?;
    instantiate_prefab(world, prefab)
}

// ── PrefabLoad trait ───────────────────────────────────────────────────────

/// Trait for types that can resolve a prefab asset by identifier.
///
/// This allows `instantiate_prefab_from_asset` to work with any storage
/// backend (in-memory map, asset server, etc.).
pub trait PrefabLoad {
    /// Look up a prefab by its asset identifier.
    fn load_prefab(&self, asset_id: &str) -> Option<&Prefab>;
}

// ── Simple in-memory implementation ────────────────────────────────────────

/// A simple in-memory prefab registry backed by a `HashMap`.
///
/// Useful for tooling, testing, and embedded use-cases where a full asset
/// server is not available.
#[derive(Clone, Debug, Default)]
pub struct PrefabRegistry {
    prefabs: std::collections::HashMap<String, Prefab>,
}

impl PrefabRegistry {
    pub fn new() -> Self {
        Self {
            prefabs: std::collections::HashMap::new(),
        }
    }

    /// Register a prefab under the given asset identifier.
    pub fn register(&mut self, asset_id: impl Into<String>, prefab: Prefab) {
        self.prefabs.insert(asset_id.into(), prefab);
    }
}

impl PrefabLoad for PrefabRegistry {
    fn load_prefab(&self, asset_id: &str) -> Option<&Prefab> {
        self.prefabs.get(asset_id)
    }
}

// ── Internal helpers ───────────────────────────────────────────────────────

/// Overlay `defaults` on top of `record_fields`.
///
/// Fields present in both are overridden by the default; fields only in
/// `record_fields` are preserved.
fn merge_defaults(
    record_fields: &BTreeMap<String, Value>,
    defaults: Option<&BTreeMap<String, Value>>,
) -> BTreeMap<String, Value> {
    let mut merged = record_fields.clone();
    if let Some(defaults) = defaults {
        for (key, value) in defaults {
            merged.insert(key.clone(), value.clone());
        }
    }
    merged
}

// ════════════════════════════════════════════════════════════════════════════
// Tests
// ════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prefab::Prefab;
    use crate::scene::{ComponentRecord, EntityRecord};
    use crate::World;
    use engine_serialize::{AssetId, SchemaVersion, Value};

    fn make_transform_record() -> ComponentRecord {
        let mut fields = BTreeMap::new();
        fields.insert("translation".to_string(), Value::Vec3([1.0, 2.0, 3.0]));
        fields.insert("rotation".to_string(), Value::Quat([0.0, 0.0, 0.0, 1.0]));
        fields.insert("scale".to_string(), Value::Vec3([2.0, 2.0, 2.0]));
        ComponentRecord {
            schema_version: SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields,
        }
    }

    #[test]
    fn instantiate_empty_prefab_fails() {
        let mut world = World::new();
        let prefab = Prefab::new(AssetId::new("prefabs/empty.prefab"));
        let result = instantiate_prefab(&mut world, &prefab);
        assert!(result.is_err(), "expected error for empty prefab");
    }

    #[test]
    fn instantiate_single_entity() {
        let mut world = World::new();
        let mut prefab = Prefab::new(AssetId::new("prefabs/single.prefab"));

        let mut components = BTreeMap::new();
        components.insert("engine.transform".to_string(), make_transform_record());

        prefab.add_entity(EntityRecord {
            persistent_id: "ent-root".to_string(),
            parent: None,
            name: Some("Root".to_string()),
            enabled: true,
            components,
        });

        let result = instantiate_prefab(&mut world, &prefab).expect("instantiate");
        assert_eq!(result.all_entities.len(), 1);
        assert_eq!(result.root_entity, result.all_entities[0]);
        assert!(world.is_alive(result.root_entity));

        // Verify PrefabInstanceRef
        let instance_ref = world
            .get::<PrefabInstanceRef>(result.root_entity)
            .expect("should have PrefabInstanceRef");
        assert_eq!(instance_ref.source_asset, "prefabs/single.prefab");
        assert_eq!(instance_ref.schema_version, prefab.schema_version);

        // Verify Name component
        let name = world
            .get::<crate::components::Name>(result.root_entity)
            .expect("should have Name");
        assert_eq!(name.0, "Root");

        // Verify Transform component
        let transform = world
            .get::<crate::components::Transform>(result.root_entity)
            .expect("should have Transform");
        assert_eq!(transform.translation.x, 1.0);
        assert_eq!(transform.translation.y, 2.0);
        assert_eq!(transform.translation.z, 3.0);
        assert_eq!(transform.scale.x, 2.0);
    }

    #[test]
    fn instantiate_hierarchy_with_parent_child() {
        let mut world = World::new();
        let mut prefab = Prefab::new(AssetId::new("prefabs/hierarchy.prefab"));

        let mut parent_components = BTreeMap::new();
        parent_components.insert("engine.transform".to_string(), make_transform_record());

        let mut child_components = BTreeMap::new();
        let mut child_fields = BTreeMap::new();
        child_fields.insert("translation".to_string(), Value::Vec3([10.0, 0.0, 0.0]));
        child_fields.insert("rotation".to_string(), Value::Quat([0.0, 0.0, 0.0, 1.0]));
        child_fields.insert("scale".to_string(), Value::Vec3([1.0, 1.0, 1.0]));
        child_components.insert(
            "engine.transform".to_string(),
            ComponentRecord {
                schema_version: SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: child_fields,
            },
        );

        prefab
            .add_entity(EntityRecord {
                persistent_id: "ent-parent".to_string(),
                parent: None,
                name: Some("Parent".to_string()),
                enabled: true,
                components: parent_components,
            })
            .add_entity(EntityRecord {
                persistent_id: "ent-child".to_string(),
                parent: Some("ent-parent".to_string()),
                name: Some("Child".to_string()),
                enabled: true,
                components: child_components,
            });

        let result = instantiate_prefab(&mut world, &prefab).expect("instantiate");
        assert_eq!(result.all_entities.len(), 2);

        // Parent has no parent link
        let parent_transform = world
            .get::<crate::components::Transform>(result.root_entity)
            .expect("parent transform");
        assert!(parent_transform.parent.is_none());

        // Find child entity and check its parent
        let child_entity = result
            .all_entities
            .iter()
            .find(|&&e| {
                world
                    .get::<crate::components::Name>(e)
                    .map(|n| n.0 == "Child")
                    .unwrap_or(false)
            })
            .expect("child entity");

        let child_transform = world
            .get::<crate::components::Transform>(*child_entity)
            .expect("child transform");
        assert_eq!(child_transform.parent, Some(result.root_entity));
    }

    #[test]
    fn instantiate_applies_component_defaults() {
        let mut world = World::new();
        let mut prefab = Prefab::new(AssetId::new("prefabs/defaults.prefab"));

        // Add entity with a transform that has scale=1.
        let mut fields = BTreeMap::new();
        fields.insert("translation".to_string(), Value::Vec3([0.0; 3]));
        fields.insert("rotation".to_string(), Value::Quat([0.0, 0.0, 0.0, 1.0]));
        fields.insert("scale".to_string(), Value::Vec3([1.0, 1.0, 1.0]));
        let mut components = BTreeMap::new();
        components.insert(
            "engine.transform".to_string(),
            ComponentRecord {
                schema_version: SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields,
            },
        );

        prefab.add_entity(EntityRecord {
            persistent_id: "ent-root".to_string(),
            parent: None,
            name: Some("WithDefaults".to_string()),
            enabled: true,
            components,
        });

        // Set component default that overrides scale
        prefab.set_default(
            "engine.transform",
            "scale",
            Value::Vec3([5.0, 5.0, 5.0]),
        );

        let result = instantiate_prefab(&mut world, &prefab).expect("instantiate");
        let transform = world
            .get::<crate::components::Transform>(result.root_entity)
            .expect("transform");
        assert_eq!(transform.scale.x, 5.0);
        assert_eq!(transform.scale.y, 5.0);
        assert_eq!(transform.scale.z, 5.0);
        // Translation should keep its original value
        assert_eq!(transform.translation.x, 0.0);
    }

    #[test]
    fn prefab_instance_ref_on_all_entities() {
        let mut world = World::new();
        let mut prefab = Prefab::new(AssetId::new("prefabs/inst_refs.prefab"));

        for i in 0..3 {
            let mut components = BTreeMap::new();
            components.insert("engine.transform".to_string(), make_transform_record());
            prefab.add_entity(EntityRecord {
                persistent_id: format!("ent-{i}"),
                parent: if i == 0 { None } else { Some("ent-0".to_string()) },
                name: Some(format!("Entity {i}")),
                enabled: true,
                components,
            });
        }

        let result = instantiate_prefab(&mut world, &prefab).expect("instantiate");
        assert_eq!(result.all_entities.len(), 3);

        // All entities should have PrefabInstanceRef with same instance_id
        let instance_id = world
            .get::<PrefabInstanceRef>(result.all_entities[0])
            .unwrap()
            .instance_id
            .clone();

        for &entity in &result.all_entities {
            let instance_ref = world.get::<PrefabInstanceRef>(entity).expect("PrefabInstanceRef");
            assert_eq!(instance_ref.instance_id, instance_id);
            assert_eq!(instance_ref.source_asset, "prefabs/inst_refs.prefab");
        }
    }

    #[test]
    fn instantiate_from_asset_registry() {
        let mut world = World::new();
        let mut registry = PrefabRegistry::new();

        let mut prefab = Prefab::new(AssetId::new("prefabs/from_asset.prefab"));
        let mut components = BTreeMap::new();
        components.insert("engine.transform".to_string(), make_transform_record());
        prefab.add_entity(EntityRecord {
            persistent_id: "ent-root".to_string(),
            parent: None,
            name: Some("FromAsset".to_string()),
            enabled: true,
            components,
        });

        registry.register("prefabs/from_asset.prefab", prefab);

        let result =
            instantiate_prefab_from_asset(&mut world, &registry, "prefabs/from_asset.prefab")
                .expect("instantiate from asset");
        assert_eq!(result.all_entities.len(), 1);

        let name = world
            .get::<crate::components::Name>(result.root_entity)
            .expect("Name");
        assert_eq!(name.0, "FromAsset");
    }
}
