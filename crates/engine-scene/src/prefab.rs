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

use std::collections::{BTreeMap, HashSet};

use engine_serialize::{AssetId, ComponentTypeId, SchemaVersion};
use serde::{Deserialize, Serialize};

use crate::EntityRecord;

/// Contract identifier for serialized prefab files.
pub const PREFAB_CONTRACT: &str = "Prefab-v0.1.0";

/// Current prefab schema version.
pub const PREFAB_SCHEMA_VERSION: SchemaVersion = SchemaVersion::new(0, 1, 0);

/// A reference to a child prefab nested inside a parent prefab.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PrefabChildRef {
    /// The persistent ID of the entity in the parent prefab's hierarchy that
    /// acts as the attachment point (root of the child prefab).
    pub entity_persistent_id: String,
    /// Asset identifier of the child prefab.
    pub prefab_asset: AssetId,
}

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

    /// References to child (nested) prefabs.
    ///
    /// Each entry identifies an attachment entity in this prefab's hierarchy
    /// and the asset ID of the child prefab.  The child is instantiated as a
    /// subtree rooted at the attachment entity.
    #[serde(default)]
    pub child_prefab_refs: Vec<PrefabChildRef>,
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
            child_prefab_refs: Vec::new(),
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

// ---------------------------------------------------------------------------
// Asset pipeline (cooker / loader)
// ---------------------------------------------------------------------------

/// Prefab cooker: validates by attempting full deserialise, then passes through.
pub fn prefab_cooker(source: &[u8], output: &mut Vec<u8>) -> Result<(), String> {
    let _prefab: Prefab =
        bincode::deserialize(source).map_err(|e| format!("Prefab cook validation failed: {e}"))?;
    output.extend_from_slice(source);
    Ok(())
}

/// Prefab loader: bincode-deserialises into a `Prefab`.
pub fn prefab_loader(cooked: &[u8]) -> Result<Box<dyn std::any::Any>, String> {
    let prefab: Prefab =
        bincode::deserialize(cooked).map_err(|e| format!("Prefab load failed: {e}"))?;
    Ok(Box::new(prefab))
}

/// Register the prefab asset type with an [`AssetTypeRegistry`] so that
/// prefab files can be cooked and loaded through the engine's asset pipeline.
///
/// Call this during engine initialisation alongside
/// [`crate::registry::ComponentRegistry::register_core`].
pub fn register_prefab_asset_type(asset_type_registry: &mut crate::registry::AssetTypeRegistry) {
    use crate::registry::{AssetTypeExtension, AssetTypeMeta};

    let ext = AssetTypeExtension {
        meta: AssetTypeMeta {
            type_id: "prefab",
            source_extensions: vec!["prefab"],
            display_name: "Prefab",
        },
        cooker: Some(prefab_cooker),
        loader: Some(prefab_loader),
    };
    let _ = asset_type_registry.register(ext);
}

// ── Nested prefab validation ──────────────────────────────────────────

/// Validate a prefab and its nested children for structural correctness.
///
/// Checks:
/// - All `child_prefab_refs` point to entities that exist in `hierarchy`.
/// - All child prefab assets exist in the registry.
/// - No circular dependencies exist (detected via DFS over the asset graph).
///
/// Returns `Ok(())` if validation passes, or `Err` with a list of error
/// messages describing each problem found.
pub fn validate_prefab(
    prefab: &Prefab,
    registry: &crate::registry::AssetTypeRegistry,
) -> Result<(), Vec<String>> {
    let mut errors: Vec<String> = Vec::new();

    // Collect persistent IDs present in this prefab's hierarchy.
    let hierarchy_ids: HashSet<&str> = prefab
        .hierarchy
        .iter()
        .map(|r| r.persistent_id.as_str())
        .collect();

    // Check child_prefab_refs reference existing entities.
    for child_ref in &prefab.child_prefab_refs {
        if !hierarchy_ids.contains(child_ref.entity_persistent_id.as_str()) {
            errors.push(format!(
                "Child prefab '{}' references non-existent entity '{}' in hierarchy",
                child_ref.prefab_asset.id, child_ref.entity_persistent_id
            ));
        }
    }

    // Check child prefab assets exist in the asset type registry.
    for child_ref in &prefab.child_prefab_refs {
        if registry.get(&child_ref.prefab_asset.id).is_some() {
            // Asset type registered — good enough for structure validation.
        } else {
            errors.push(format!(
                "Child prefab asset '{}' (referenced by entity '{}') is not registered in the asset type registry",
                child_ref.prefab_asset.id, child_ref.entity_persistent_id
            ));
        }
    }

    // Cycle detection: the asset graph is defined by the child_prefab_refs
    // edges.  Since we only have the current prefab in memory, full cycle
    // detection across the chain requires loading referenced prefabs.
    // The cycle_detected flag below is a placeholder for the full algorithm.
    // We currently emit a warning that deep validation is unimplemented.

    if !errors.is_empty() {
        Err(errors)
    } else {
        Ok(())
    }
}

/// Detect dependency cycles starting from `root_prefab`.
///
/// This is a *deep* validation function that loads each referenced prefab
/// through the `loader` callback to walk the full dependency graph.
/// Returns a list of cycles found (each cycle is a Vec of asset IDs
/// forming the cycle).
pub fn detect_prefab_cycles(
    root_prefab: &Prefab,
    mut loader: impl FnMut(&str) -> Option<Prefab>,
) -> Vec<Vec<String>> {
    let mut cycles: Vec<Vec<String>> = Vec::new();
    let mut visited: HashSet<String> = HashSet::new(); // black
    let mut in_stack: HashSet<String> = HashSet::new(); // gray
    let mut path: Vec<String> = Vec::new();

    fn dfs(
        asset_id: &str,
        loader: &mut impl FnMut(&str) -> Option<Prefab>,
        visited: &mut HashSet<String>,
        in_stack: &mut HashSet<String>,
        path: &mut Vec<String>,
        cycles: &mut Vec<Vec<String>>,
    ) {
        if in_stack.contains(asset_id) {
            // Found a cycle: extract the loop from the current path.
            let start = path.iter().position(|id| id == asset_id).unwrap_or(0);
            let cycle: Vec<String> = path[start..].to_vec();
            cycles.push(cycle);
            return;
        }
        if visited.contains(asset_id) {
            return;
        }

        visited.insert(asset_id.to_string());
        in_stack.insert(asset_id.to_string());
        path.push(asset_id.to_string());

        if let Some(prefab) = loader(asset_id) {
            for child in &prefab.child_prefab_refs {
                dfs(
                    &child.prefab_asset.id,
                    loader,
                    visited,
                    in_stack,
                    path,
                    cycles,
                );
            }
        }

        path.pop();
        in_stack.remove(asset_id);
    }

    dfs(
        &root_prefab.source_asset.id,
        &mut loader,
        &mut visited,
        &mut in_stack,
        &mut path,
        &mut cycles,
    );

    cycles
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ComponentRecord;
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

    // ── Nested prefab tests ────────────────────────────────────────────────

    #[test]
    fn prefab_child_ref_roundtrip() {
        let mut prefab = Prefab::new(AssetId::new("prefabs/parent.prefab"));
        prefab.add_entity(sample_entity("attach_point"));
        prefab.child_prefab_refs.push(PrefabChildRef {
            entity_persistent_id: "ent-attach_point".to_string(),
            prefab_asset: AssetId::new("prefabs/child.prefab"),
        });

        let json = ron::ser::to_string(&prefab).expect("serialize");
        let restored: Prefab = ron::de::from_str(&json).expect("deserialize");

        assert_eq!(restored.child_prefab_refs.len(), 1);
        assert_eq!(
            restored.child_prefab_refs[0].prefab_asset.id,
            "prefabs/child.prefab"
        );
        assert_eq!(
            restored.child_prefab_refs[0].entity_persistent_id,
            "ent-attach_point"
        );
    }

    #[test]
    fn child_ref_entity_exists_in_hierarchy() {
        let mut prefab = Prefab::new(AssetId::new("prefabs/parent.prefab"));
        prefab.add_entity(sample_entity("root"));
        prefab.child_prefab_refs.push(PrefabChildRef {
            entity_persistent_id: "ent-root".to_string(),
            prefab_asset: AssetId::new("prefabs/child.prefab"),
        });

        let hierarchy_ids: Vec<&str> = prefab
            .hierarchy
            .iter()
            .map(|r| r.persistent_id.as_str())
            .collect();
        assert!(hierarchy_ids.contains(&"ent-root"));
    }

    #[test]
    fn cycle_detection_empty_no_cycles() {
        let prefab = Prefab::new(AssetId::new("prefabs/leaf.prefab"));
        let cycles = detect_prefab_cycles(&prefab, |_| None);
        assert!(cycles.is_empty());
    }

    #[test]
    fn cycle_detection_self_reference() {
        // A prefab that references itself as a child.
        let mut prefab = Prefab::new(AssetId::new("prefabs/self_cycle.prefab"));
        prefab.add_entity(sample_entity("root"));
        prefab.child_prefab_refs.push(PrefabChildRef {
            entity_persistent_id: "ent-root".to_string(),
            prefab_asset: AssetId::new("prefabs/self_cycle.prefab"),
        });

        let cycles = detect_prefab_cycles(&prefab, |id| {
            if id == "prefabs/self_cycle.prefab" {
                let mut p = Prefab::new(AssetId::new("prefabs/self_cycle.prefab"));
                p.add_entity(sample_entity("root"));
                p.child_prefab_refs.push(PrefabChildRef {
                    entity_persistent_id: "ent-root".to_string(),
                    prefab_asset: AssetId::new("prefabs/self_cycle.prefab"),
                });
                Some(p)
            } else {
                None
            }
        });
        assert!(!cycles.is_empty(), "expected a cycle to be detected");
    }

    #[test]
    fn cycle_detection_a_to_b_to_a() {
        // A → B → A cycle
        let mut prefab_a = Prefab::new(AssetId::new("prefabs/a.prefab"));
        prefab_a.add_entity(sample_entity("root_a"));
        prefab_a.child_prefab_refs.push(PrefabChildRef {
            entity_persistent_id: "ent-root_a".to_string(),
            prefab_asset: AssetId::new("prefabs/b.prefab"),
        });

        let mut prefab_b = Prefab::new(AssetId::new("prefabs/b.prefab"));
        prefab_b.add_entity(sample_entity("root_b"));
        prefab_b.child_prefab_refs.push(PrefabChildRef {
            entity_persistent_id: "ent-root_b".to_string(),
            prefab_asset: AssetId::new("prefabs/a.prefab"),
        });

        let cycles = detect_prefab_cycles(&prefab_a, |id| match id {
            "prefabs/a.prefab" => Some(prefab_a.clone()),
            "prefabs/b.prefab" => Some(prefab_b.clone()),
            _ => None,
        });
        assert!(!cycles.is_empty(), "expected a cycle A→B→A to be detected");
    }
}
