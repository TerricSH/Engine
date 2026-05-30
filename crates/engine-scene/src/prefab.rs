//! Prefab seed — minimal reusable entity template (Gate 11, G11-F05).
//!
//! A prefab is a self-contained entity hierarchy snapshot that can be
//! instantiated into a scene.  This seed implementation stores:
//!
//! - A source asset identifier (logical path in the asset registry).
//! - An entity hierarchy snapshot (list of `EntityRecord`).
//! - Component default overrides (keyed by component type, field name, value).
//! - A version field for forward compatibility.
//!
//! Full override semantics (nested prefab composition, field-level override
//! resolution, runtime property propagation) are owned by Gate 14.

use std::collections::BTreeMap;

use engine_serialize::{AssetId, ComponentTypeId, SchemaVersion};
use serde::{Deserialize, Serialize};

use crate::EntityRecord;

/// Contract identifier for serialized prefab files.
pub const PREFAB_CONTRACT: &str = "Prefab-v0.1.0";

/// Current prefab schema version.
pub const PREFAB_SCHEMA_VERSION: SchemaVersion = SchemaVersion::new(0, 1, 0);

/// A prefab seed: a reusable entity hierarchy with component defaults.
///
/// Fields are versioned from the start.  The `source_asset` links back to the
/// original asset file in the registry.  `component_defaults` allow prefab
/// authors to override specific component fields per type without replacing
/// the entire component record.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Prefab {
    /// Prefab schema version for forward-compat deserialization.
    pub schema_version: SchemaVersion,
    /// Prefab semantic version (data format, not engine version).
    pub prefab_version: String,
    /// Source asset identifier (logical path in the asset registry).
    pub source_asset: AssetId,
    /// Entity hierarchy snapshot — the entities to instantiate.
    #[serde(default)]
    pub hierarchy: Vec<EntityRecord>,
    /// Component default overrides: component_type → { field_name → value }.
    /// Applied when instantiating the prefab; fields not mentioned keep their
    /// defaults from `EntityRecord`.
    #[serde(default)]
    pub component_defaults: BTreeMap<ComponentTypeId, BTreeMap<String, engine_serialize::Value>>,
}

impl Prefab {
    /// Create a new prefab from a source asset.
    pub fn new(source_asset: AssetId) -> Self {
        Self {
            schema_version: PREFAB_SCHEMA_VERSION,
            prefab_version: "0.1.0".to_string(),
            source_asset,
            hierarchy: Vec::new(),
            component_defaults: BTreeMap::new(),
        }
    }

    /// Add an entity to the prefab's hierarchy.
    pub fn add_entity(&mut self, entity: EntityRecord) -> &mut Self {
        self.hierarchy.push(entity);
        self
    }

    /// Set a component default override.
    pub fn set_default(
        &mut self,
        component_type: impl Into<ComponentTypeId>,
        field: impl Into<String>,
        value: engine_serialize::Value,
    ) -> &mut Self {
        self.component_defaults
            .entry(component_type.into())
            .or_default()
            .insert(field.into(), value);
        self
    }

    /// Number of entities in the prefab hierarchy.
    pub fn entity_count(&self) -> usize {
        self.hierarchy.len()
    }

    /// Whether the prefab has no entities.
    pub fn is_empty(&self) -> bool {
        self.hierarchy.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
use crate::EntityRecord;

    fn sample_entity(name: &str) -> EntityRecord {
        let mut components = BTreeMap::new();
        components.insert(
            "engine.transform".to_string(),
            ComponentRecord {
                schema_version: SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: BTreeMap::new(),
            },
        );
        EntityRecord {
            persistent_id: format!("ent-{name}"),
            parent: None,
            name: Some(name.to_string()),
            enabled: true,
            components,
        }
    }

    #[test]
    fn prefab_new_creates_empty() {
        let prefab = Prefab::new(AssetId::new("prefabs/box.prefab"));
        assert_eq!(prefab.source_asset.id, "prefabs/box.prefab");
        assert_eq!(prefab.schema_version, PREFAB_SCHEMA_VERSION);
        assert!(prefab.is_empty());
        assert_eq!(prefab.entity_count(), 0);
    }

    #[test]
    fn prefab_add_entities() {
        let mut prefab = Prefab::new(AssetId::new("prefabs/tank.prefab"));
        prefab
            .add_entity(sample_entity("hull"))
            .add_entity(sample_entity("turret"))
            .add_entity(sample_entity("wheel_L"))
            .add_entity(sample_entity("wheel_R"));
        assert_eq!(prefab.entity_count(), 4);
        assert!(!prefab.is_empty());
    }

    #[test]
    fn prefab_set_defaults() {
        let mut prefab = Prefab::new(AssetId::new("prefabs/light.prefab"));
        prefab.set_default(
            "engine.light",
            "intensity",
            engine_serialize::Value::Float32(2.0),
        );
        let defaults = prefab.component_defaults.get("engine.light").unwrap();
        assert_eq!(
            defaults.get("intensity"),
            Some(&engine_serialize::Value::Float32(2.0))
        );
    }

    #[test]
    fn prefab_serde_bincode_roundtrip() {
        let mut prefab = Prefab::new(AssetId::new("prefabs/player.prefab"));
        prefab
            .add_entity(sample_entity("body"))
            .add_entity(sample_entity("camera"));
        prefab.set_default(
            "engine.renderable",
            "cast_shadows",
            engine_serialize::Value::Bool(true),
        );

        let bytes = ron::ser::to_string(&prefab).expect("serialize");
        let restored: Prefab = ron::de::from_str(&bytes).expect("deserialize");

        assert_eq!(prefab.source_asset, restored.source_asset);
        assert_eq!(prefab.entity_count(), restored.entity_count());
        assert_eq!(
            prefab.component_defaults.len(),
            restored.component_defaults.len()
        );
    }

    #[test]
    fn prefab_contract_is_stable() {
        assert_eq!(PREFAB_CONTRACT, "Prefab-v0.1.0");
        assert_eq!(PREFAB_SCHEMA_VERSION, SchemaVersion::new(0, 1, 0));
    }
}
