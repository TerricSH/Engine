//! Gate 14 — Prefab Override System (G14-F03).
//!
//! Allows individual prefab instances to override specific component fields
//! at edit time.  Overrides are stored as [`OverrideRecord`]s keyed by
//! (instance_id, entity_persistent_id, component_type, property_path).

use engine_serialize::Value;
use serde::{Deserialize, Serialize};

use crate::prefab_instance::PrefabInstanceRef;
use crate::World;

// ── OverrideRecord ─────────────────────────────────────────────────────────

/// A single field-level override applied to a prefab instance entity.
///
/// Exactly one override exists per (instance, entity, component, property)
/// combination so that the value can be efficiently resolved and reverted.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct OverrideRecord {
    /// Identifies the prefab instantiation (every entity from the same
    /// `instantiate_prefab` call shares this).
    pub instance_id: String,
    /// The `persistent_id` of the entity within the prefab hierarchy.
    pub entity_persistent_id: String,
    /// Component type identifier (e.g. `"engine.transform"`).
    pub component_type: String,
    /// Dot-separated property path within the component (e.g. `"scale"`).
    pub property_path: String,
    /// The overridden value.
    pub value: Value,
}

// ── OverrideSet ────────────────────────────────────────────────────────────

/// An ordered collection of overrides.
///
/// Insertion order is preserved so that serialised edits round-trip
/// deterministically.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct OverrideSet(Vec<OverrideRecord>);

impl OverrideSet {
    pub fn new() -> Self {
        Self(Vec::new())
    }

    /// Add an override record.
    ///
    /// If a record with the same (instance_id, entity, component, property)
    /// already exists it is replaced.
    pub fn add(&mut self, record: OverrideRecord) {
        if let Some(existing) = self.0.iter_mut().find(|r| {
            r.instance_id == record.instance_id
                && r.entity_persistent_id == record.entity_persistent_id
                && r.component_type == record.component_type
                && r.property_path == record.property_path
        }) {
            *existing = record;
        } else {
            self.0.push(record);
        }
    }

    /// Remove all overrides for a given instance.
    pub fn remove_instance(&mut self, instance_id: &str) {
        self.0.retain(|r| r.instance_id != instance_id);
    }

    /// Number of override records.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns `true` if there are no overrides.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Iterate over all records.
    pub fn iter(&self) -> impl Iterator<Item = &OverrideRecord> {
        self.0.iter()
    }

    /// Iterate over all records for a given instance.
    pub fn iter_instance<'a>(&'a self, instance_id: &'a str) -> impl Iterator<Item = &'a OverrideRecord> + 'a {
        self.0.iter().filter(move |r| r.instance_id == instance_id)
    }
}

impl From<Vec<OverrideRecord>> for OverrideSet {
    fn from(records: Vec<OverrideRecord>) -> Self {
        Self(records)
    }
}

// ── Resolution ─────────────────────────────────────────────────────────────

/// Look up an override by exact match on all five key fields.
pub fn resolve_override<'a>(
    overrides: &'a OverrideSet,
    instance_id: &str,
    entity_id: &str,
    component_type: &str,
    property_path: &str,
) -> Option<&'a Value> {
    overrides
        .0
        .iter()
        .find(|r| {
            r.instance_id == instance_id
                && r.entity_persistent_id == entity_id
                && r.component_type == component_type
                && r.property_path == property_path
        })
        .map(|r| &r.value)
}

// ── Apply overrides to a world ─────────────────────────────────────────────

/// Apply all overrides in the set to the live entities in `world`.
///
/// Each [`OverrideRecord`] is resolved by finding the entity that has both a
/// matching `PrefabInstanceRef.instance_id` and the matching persistent ID,
/// then writing the override value into the correct component field.
///
/// If no matching entity or component is found, the override is silently
/// skipped (it may reference an entity that has not been instantiated yet).
pub fn apply_overrides(world: &mut World, overrides: &OverrideSet) {
    for record in &overrides.0 {
        // Find the target entity by matching BOTH instance_id AND entity_persistent_id.
        let target_entity = world
            .query::<PrefabInstanceRef>()
            .find(|(_, inst_ref)| {
                inst_ref.instance_id == record.instance_id
                    && inst_ref.entity_persistent_id == record.entity_persistent_id
            })
            .map(|(entity, _)| entity);

        let Some(entity) = target_entity else {
            continue;
        };

        // Look up the current component via the type-erased path and set the
        // overridden field using serialize/deserialize round-trip.
        //
        // For known core component types we apply directly; for external
        // (registered) types we use the component registry hooks.
        apply_field_override(world, entity, &record.component_type, &record.property_path, &record.value);
    }
}

/// Revert all overrides for a given prefab instance.
///
/// This removes every [`OverrideRecord`] whose `instance_id` matches, and
/// resets the affected component fields back to the values stored in the
/// original prefab's `component_defaults` (or the entity record's original
/// values if no default exists).
///
/// The current implementation resets by re-instantiating the default field
/// value from the component type definition.  For the known core types this
/// means setting the field to its zero/default value.  A full implementation
/// would store the original values alongside the override.
pub fn revert_overrides(world: &mut World, overrides: &mut OverrideSet, instance_id: &str) {
    // Collect the records to revert, then remove them.
    let reverted: Vec<OverrideRecord> = overrides
        .0
        .iter()
        .filter(|r| r.instance_id == instance_id)
        .cloned()
        .collect();

    overrides.remove_instance(instance_id);

    // For each reverted record we re-apply a "default" value.
    // In a production system this would restore the original pre-override
    // value that was snapshotted when the override was first applied.
    for record in &reverted {
        let target_entity = world
            .query::<PrefabInstanceRef>()
            .find(|(_, inst_ref)| {
                inst_ref.instance_id == record.instance_id
                    && inst_ref.entity_persistent_id == record.entity_persistent_id
            })
            .map(|(entity, _)| entity);

        let Some(entity) = target_entity else {
            continue;
        };

        // Reset to the zero/default value for the field.
        let default_val = default_value_for_field(&record.property_path);
        apply_field_override(
            world,
            entity,
            &record.component_type,
            &record.property_path,
            &default_val,
        );
    }
}

// ── Internal helpers ───────────────────────────────────────────────────────

/// Apply a single field override to a component on the given entity.
fn apply_field_override(
    world: &mut World,
    entity: crate::Entity,
    component_type: &str,
    property_path: &str,
    value: &Value,
) {
    match component_type {
        "engine.transform" => {
            if let Some(transform) = world.get_mut::<crate::components::Transform>(entity) {
                match property_path {
                    "translation" => {
                        if let Value::Vec3(v) = value {
                            transform.translation = glam::Vec3::from(*v);
                        }
                    }
                    "rotation" => {
                        if let Value::Quat(q) = value {
                            transform.rotation = glam::Quat::from_array(*q);
                        }
                    }
                    "scale" => {
                        if let Value::Vec3(v) = value {
                            transform.scale = glam::Vec3::from(*v);
                        }
                    }
                    _ => {}
                }
            }
        }
        "engine.name" => {
            if let Some(name) = world.get_mut::<crate::components::Name>(entity) {
                if property_path == "name" {
                    if let Value::Str(s) = value {
                        name.0 = s.clone();
                    }
                }
            }
        }
        "engine.renderable" => {
            if let Some(renderable) = world.get_mut::<crate::components::Renderable>(entity) {
                match property_path {
                    "visible" => {
                        if let Value::Bool(v) = value {
                            renderable.visible = *v;
                        }
                    }
                    "cast_shadows" => {
                        if let Value::Bool(v) = value {
                            renderable.cast_shadows = *v;
                        }
                    }
                    "render_layer" => {
                        if let Value::Str(s) = value {
                            renderable.render_layer = s.clone();
                        }
                    }
                    "mesh" => {
                        if let Value::Asset(a) = value {
                            renderable.mesh_asset = a.id.clone();
                        }
                    }
                    "material" => {
                        if let Value::Asset(a) = value {
                            renderable.material_asset = a.id.clone();
                        }
                    }
                    _ => {}
                }
            }
        }
        "engine.camera" => {
            if let Some(camera) = world.get_mut::<crate::components::Camera>(entity) {
                match property_path {
                    "near" => {
                        if let Value::Float32(v) = value {
                            camera.near = *v;
                        }
                    }
                    "far" => {
                        if let Value::Float32(v) = value {
                            camera.far = *v;
                        }
                    }
                    "fov_y" => {
                        if let Value::Float32(v) = value {
                            camera.fov_y = *v;
                        }
                    }
                    "priority" => {
                        if let Value::Int(v) = value {
                            camera.priority = *v as i32;
                        }
                    }
                    _ => {}
                }
            }
        }
        "engine.light" => {
            if let Some(light) = world.get_mut::<crate::components::Light>(entity) {
                match property_path {
                    "intensity" => {
                        if let Value::Float32(v) = value {
                            light.intensity = *v;
                        }
                    }
                    "color" => {
                        if let Value::Vec3(v) = value {
                            light.color = *v;
                        }
                    }
                    "range" => {
                        if let Value::Float32(v) = value {
                            light.range = *v;
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {
            // External component types: round-trip via registry hooks.
            if let Some(ref registry) = world.component_registry {
                if let Some(ext) = registry.get(component_type) {
                    if let Some(ser_fn) = ext.serialize {
                        if let Some(any_ref) = world.get_any(entity, component_type) {
                            let mut fields = ser_fn(any_ref);
                            fields.insert(property_path.to_string(), value.clone());
                            if let Some(de_fn) = ext.deserialize {
                                let component = de_fn(&fields);
                                // The storage for this type should already exist.
                                if let Some(storage) = world.storages.get_mut(component_type) {
                                    let _ = storage.insert_any(entity, component);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Return a sensible "zero" default value for known field property paths.
fn default_value_for_field(property_path: &str) -> Value {
    // NOTE: In a real engine these would come from the component's schema.
    match property_path {
        "translation" | "scale" => Value::Vec3([0.0, 0.0, 0.0]),
        "rotation" => Value::Quat([0.0, 0.0, 0.0, 1.0]),
        "visible" | "cast_shadows" => Value::Bool(false),
        "intensity" | "near" | "far" | "fov_y" | "range" => Value::Float32(0.0),
        "priority" => Value::Int(0),
        _ => Value::Bool(false),
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Tests
// ════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prefab_instance::instantiate_prefab;
    use crate::prefab::Prefab;
    use crate::scene::{ComponentRecord, EntityRecord};
    use crate::World;
    use engine_serialize::{AssetId, SchemaVersion};
    use std::collections::BTreeMap;

    fn sample_world_with_instance() -> (World, String) {
        let mut world = World::new();
        let mut prefab = Prefab::new(AssetId::new("prefabs/test.prefab"));

        let mut fields = BTreeMap::new();
        fields.insert("translation".to_string(), Value::Vec3([1.0, 2.0, 3.0]));
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
            persistent_id: "ent-test".to_string(),
            parent: None,
            name: Some("TestEntity".to_string()),
            enabled: true,
            components,
        });

        let result = instantiate_prefab(&mut world, &prefab).unwrap();
        let instance_id = world
            .get::<PrefabInstanceRef>(result.root_entity)
            .unwrap()
            .instance_id
            .clone();
        (world, instance_id)
    }

    #[test]
    fn resolve_override_found() {
        let mut overrides = OverrideSet::new();
        overrides.add(OverrideRecord {
            instance_id: "inst_0".to_string(),
            entity_persistent_id: "ent-test".to_string(),
            component_type: "engine.transform".to_string(),
            property_path: "scale".to_string(),
            value: Value::Vec3([2.0, 2.0, 2.0]),
        });

        let value = resolve_override(&overrides, "inst_0", "ent-test", "engine.transform", "scale");
        assert!(value.is_some());
        assert_eq!(value.unwrap(), &Value::Vec3([2.0, 2.0, 2.0]));
    }

    #[test]
    fn resolve_override_not_found() {
        let overrides = OverrideSet::new();
        let value = resolve_override(&overrides, "inst_0", "ent-test", "engine.transform", "scale");
        assert!(value.is_none());
    }

    #[test]
    fn resolve_override_wrong_instance() {
        let mut overrides = OverrideSet::new();
        overrides.add(OverrideRecord {
            instance_id: "inst_a".to_string(),
            entity_persistent_id: "ent-test".to_string(),
            component_type: "engine.transform".to_string(),
            property_path: "scale".to_string(),
            value: Value::Vec3([2.0, 2.0, 2.0]),
        });

        // Different instance_id should not match.
        let value = resolve_override(&overrides, "inst_b", "ent-test", "engine.transform", "scale");
        assert!(value.is_none());
    }

    #[test]
    fn apply_override_changes_component() {
        let (mut world, instance_id) = sample_world_with_instance();

        let mut overrides = OverrideSet::new();
        overrides.add(OverrideRecord {
            instance_id,
            entity_persistent_id: "ent-test".to_string(),
            component_type: "engine.transform".to_string(),
            property_path: "scale".to_string(),
            value: Value::Vec3([10.0, 20.0, 30.0]),
        });

        apply_overrides(&mut world, &overrides);

        // Verify transform scale changed
        let entity = world
            .query::<PrefabInstanceRef>()
            .next()
            .unwrap()
            .0;
        let transform = world.get::<crate::components::Transform>(entity).unwrap();
        assert_eq!(transform.scale.x, 10.0);
        assert_eq!(transform.scale.y, 20.0);
        assert_eq!(transform.scale.z, 30.0);
        // Translation should NOT have changed
        assert_eq!(transform.translation.x, 1.0);
    }

    #[test]
    fn revert_overrides_restores_defaults() {
        let (mut world, instance_id) = sample_world_with_instance();

        let mut overrides = OverrideSet::new();
        overrides.add(OverrideRecord {
            instance_id: instance_id.clone(),
            entity_persistent_id: "ent-test".to_string(),
            component_type: "engine.transform".to_string(),
            property_path: "scale".to_string(),
            value: Value::Vec3([99.0, 99.0, 99.0]),
        });

        apply_overrides(&mut world, &overrides);

        // Verify override was applied
        let entity = world.query::<PrefabInstanceRef>().next().unwrap().0;
        let transform = world.get::<crate::components::Transform>(entity).unwrap();
        assert_eq!(transform.scale.x, 99.0);

        // Revert
        revert_overrides(&mut world, &mut overrides, &instance_id);

        // Verify override is removed from set
        assert!(overrides.is_empty());

        // Verify field reverted to default
        let transform = world.get::<crate::components::Transform>(entity).unwrap();
        assert_eq!(transform.scale.x, 0.0); // default default_value_for_field("scale")
    }

    #[test]
    fn overrideset_add_replaces_existing() {
        let mut overrides = OverrideSet::new();
        overrides.add(OverrideRecord {
            instance_id: "inst_0".to_string(),
            entity_persistent_id: "ent-1".to_string(),
            component_type: "engine.transform".to_string(),
            property_path: "scale".to_string(),
            value: Value::Vec3([1.0, 1.0, 1.0]),
        });
        overrides.add(OverrideRecord {
            instance_id: "inst_0".to_string(),
            entity_persistent_id: "ent-1".to_string(),
            component_type: "engine.transform".to_string(),
            property_path: "scale".to_string(),
            value: Value::Vec3([2.0, 2.0, 2.0]),
        });

        assert_eq!(overrides.len(), 1);
        assert_eq!(
            overrides.iter().next().unwrap().value,
            Value::Vec3([2.0, 2.0, 2.0])
        );
    }

    #[test]
    fn overrideset_remove_instance() {
        let mut overrides = OverrideSet::new();
        overrides.add(OverrideRecord {
            instance_id: "inst_a".to_string(),
            entity_persistent_id: "e1".to_string(),
            component_type: "t".to_string(),
            property_path: "p".to_string(),
            value: Value::Bool(true),
        });
        overrides.add(OverrideRecord {
            instance_id: "inst_b".to_string(),
            entity_persistent_id: "e1".to_string(),
            component_type: "t".to_string(),
            property_path: "p".to_string(),
            value: Value::Bool(false),
        });

        overrides.remove_instance("inst_a");
        assert_eq!(overrides.len(), 1);
        assert_eq!(overrides.iter().next().unwrap().instance_id, "inst_b");
    }
}
