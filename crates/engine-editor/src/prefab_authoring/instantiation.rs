use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use engine_asset::AssetRegistry;
use engine_scene::{
    validate_prefab_structure, Component, ComponentRecord, EntityRecord, Prefab, PrefabInstanceRef,
    PrefabLoad, PrefabRegistry, Scene, PREFAB_SCHEMA_VERSION,
};
use engine_serialize::{AssetId, PersistentId, Value};

use crate::commands::{
    Command, CommandBatch, EntityClipboard, EntityPasteParent, PasteEntityRecords,
};

use super::error::join_validation_errors;
use super::util::{allocate_unique_string, instance_id, portable_token, remap_entity_values};
use super::{load_prefab_source, PrefabAuthoringError};

pub struct PrefabInstantiationPlan {
    root_entity_id: PersistentId,
    entity_ids: Vec<PersistentId>,
    command: CommandBatch,
}

impl PrefabInstantiationPlan {
    pub fn root_entity_id(&self) -> &PersistentId {
        &self.root_entity_id
    }

    pub fn entity_ids(&self) -> &[PersistentId] {
        &self.entity_ids
    }

    pub fn into_command(self) -> Box<dyn Command> {
        Box::new(self.command)
    }
}

/// Prepare one atomic, undoable prefab insertion from an already-decoded
/// asset. Nested child prefabs are resolved through `resolver`.
pub fn prepare_prefab_instantiation(
    scene: &Scene,
    prefab: &Prefab,
    resolver: Option<&dyn PrefabLoad>,
    parent: EntityPasteParent,
) -> Result<PrefabInstantiationPlan, PrefabAuthoringError> {
    let mut flattener = PrefabFlattener::new(scene);
    let root_id = flattener.flatten(prefab, resolver, None, 0)?;
    let clipboard = EntityClipboard::from_records(vec![root_id], flattener.records)?;
    let paste = PasteEntityRecords::prepare(scene, &clipboard, parent)?;
    let root_entity_id = paste.pasted_root_ids()[0].clone();
    let entity_ids = paste
        .pasted_records()
        .iter()
        .map(|record| record.persistent_id.clone())
        .collect();
    Ok(PrefabInstantiationPlan {
        root_entity_id,
        entity_ids,
        command: CommandBatch::new("Instantiate Prefab", vec![Box::new(paste)]),
    })
}

/// Prepare an insertion from a source file. Child assets, if present, still
/// require an explicit resolver so the operation cannot silently omit them.
pub fn prepare_prefab_instantiation_from_source(
    scene: &Scene,
    source_path: &Path,
    resolver: Option<&dyn PrefabLoad>,
    parent: EntityPasteParent,
) -> Result<PrefabInstantiationPlan, PrefabAuthoringError> {
    let prefab = load_prefab_source(source_path)?;
    prepare_prefab_instantiation(scene, &prefab, resolver, parent)
}

/// Prepare an insertion from the canonical typed [`AssetRegistry`] cache.
/// Every reachable child prefab must already be loaded in the same registry.
pub fn prepare_prefab_instantiation_from_registry(
    scene: &Scene,
    asset_registry: &AssetRegistry,
    asset_id: &AssetId,
    parent: EntityPasteParent,
) -> Result<PrefabInstantiationPlan, PrefabAuthoringError> {
    let mut prefab_registry = PrefabRegistry::new();
    let mut visiting = BTreeSet::new();
    collect_loaded_prefab_graph(
        asset_registry,
        asset_id,
        &mut prefab_registry,
        &mut visiting,
    )?;
    let root = prefab_registry
        .load_prefab(&asset_id.id)
        .cloned()
        .ok_or_else(|| PrefabAuthoringError::AssetNotLoaded(asset_id.id.clone()))?;
    prepare_prefab_instantiation(scene, &root, Some(&prefab_registry), parent)
}

struct PrefabFlattener {
    records: Vec<EntityRecord>,
    used_record_ids: BTreeSet<PersistentId>,
    used_instance_ids: BTreeSet<String>,
    visiting_assets: BTreeSet<String>,
}

impl PrefabFlattener {
    fn new(scene: &Scene) -> Self {
        Self {
            records: Vec::new(),
            used_record_ids: BTreeSet::new(),
            used_instance_ids: scene.entities.iter().filter_map(instance_id).collect(),
            visiting_assets: BTreeSet::new(),
        }
    }

    fn flatten(
        &mut self,
        prefab: &Prefab,
        resolver: Option<&dyn PrefabLoad>,
        attached_to: Option<PersistentId>,
        depth: usize,
    ) -> Result<PersistentId, PrefabAuthoringError> {
        if depth > engine_scene::prefab::MAX_PREFAB_NESTING_DEPTH {
            return Err(PrefabAuthoringError::InvalidPrefab(format!(
                "prefab nesting exceeds {}",
                engine_scene::prefab::MAX_PREFAB_NESTING_DEPTH
            )));
        }
        validate_prefab_structure(prefab).map_err(|errors| {
            PrefabAuthoringError::InvalidPrefab(join_validation_errors(errors))
        })?;
        if !self.visiting_assets.insert(prefab.source_asset.id.clone()) {
            return Err(PrefabAuthoringError::InvalidPrefab(format!(
                "prefab dependency cycle includes '{}'",
                prefab.source_asset.id
            )));
        }

        let instance_id = allocate_unique_string(
            &format!(
                "prefab-instance-{}",
                portable_token(&prefab.source_asset.id)
            ),
            &mut self.used_instance_ids,
        );
        let mut id_map = BTreeMap::new();
        for record in &prefab.hierarchy {
            let id = allocate_unique_string(&record.persistent_id, &mut self.used_record_ids);
            id_map.insert(record.persistent_id.clone(), id);
        }
        let root_original = prefab
            .hierarchy
            .iter()
            .find(|record| record.parent.is_none())
            .expect("structure validation guarantees one root")
            .persistent_id
            .clone();
        let root_id = id_map[&root_original].clone();
        let start = self.records.len();
        for original in &prefab.hierarchy {
            let mut record = original.clone();
            record.persistent_id = id_map[&original.persistent_id].clone();
            record.parent = original
                .parent
                .as_ref()
                .map(|parent| id_map[parent].clone());
            for (component_type, defaults) in &prefab.component_defaults {
                if let Some(component) = record.components.get_mut(component_type) {
                    component.fields.extend(defaults.clone());
                }
            }
            for component in record.components.values_mut() {
                for value in component.fields.values_mut() {
                    remap_entity_values(value, &id_map);
                }
            }
            record.components.insert(
                PrefabInstanceRef::TYPE_ID.to_string(),
                prefab_instance_component(
                    &prefab.source_asset,
                    &instance_id,
                    &original.persistent_id,
                ),
            );
            self.records.push(record);
        }
        if let Some(parent) = attached_to {
            let root = self.records[start..]
                .iter_mut()
                .find(|record| record.persistent_id == root_id)
                .expect("root record was appended");
            root.parent = Some(parent.clone());
            if let Some(transform) = root.components.get_mut("engine.transform") {
                transform
                    .fields
                    .insert("parent".into(), Value::Entity(parent));
            }
        }

        for child_ref in &prefab.child_prefab_refs {
            let resolver = resolver.ok_or_else(|| {
                PrefabAuthoringError::AssetNotLoaded(child_ref.prefab_asset.id.clone())
            })?;
            let child = resolver
                .load_prefab(&child_ref.prefab_asset.id)
                .ok_or_else(|| {
                    PrefabAuthoringError::AssetNotLoaded(child_ref.prefab_asset.id.clone())
                })?;
            let attachment = id_map[&child_ref.entity_persistent_id].clone();
            self.flatten(child, Some(resolver), Some(attachment), depth + 1)?;
        }
        self.visiting_assets.remove(&prefab.source_asset.id);
        Ok(root_id)
    }
}

fn collect_loaded_prefab_graph(
    assets: &AssetRegistry,
    asset_id: &AssetId,
    prefabs: &mut PrefabRegistry,
    visiting: &mut BTreeSet<String>,
) -> Result<(), PrefabAuthoringError> {
    if !visiting.insert(asset_id.id.clone()) {
        return Err(PrefabAuthoringError::InvalidPrefab(format!(
            "prefab dependency cycle includes '{}'",
            asset_id.id
        )));
    }
    let prefab = assets
        .get::<Prefab>(asset_id)
        .ok_or_else(|| PrefabAuthoringError::AssetNotLoaded(asset_id.id.clone()))?
        .get()
        .clone();
    for child in &prefab.child_prefab_refs {
        collect_loaded_prefab_graph(assets, &child.prefab_asset, prefabs, visiting)?;
    }
    prefabs.register(asset_id.id.clone(), prefab);
    visiting.remove(&asset_id.id);
    Ok(())
}

pub(super) fn prefab_instance_component(
    source_asset: &AssetId,
    instance_id: &str,
    entity_persistent_id: &str,
) -> ComponentRecord {
    ComponentRecord {
        schema_version: PREFAB_SCHEMA_VERSION,
        enabled: true,
        fields: BTreeMap::from([
            ("source_asset".into(), Value::Asset(source_asset.clone())),
            ("instance_id".into(), Value::Str(instance_id.into())),
            (
                "entity_persistent_id".into(),
                Value::Str(entity_persistent_id.into()),
            ),
            (
                "schema_major".into(),
                Value::UInt(u64::from(PREFAB_SCHEMA_VERSION.major)),
            ),
            (
                "schema_minor".into(),
                Value::UInt(u64::from(PREFAB_SCHEMA_VERSION.minor)),
            ),
            (
                "schema_patch".into(),
                Value::UInt(u64::from(PREFAB_SCHEMA_VERSION.patch)),
            ),
        ]),
    }
}
