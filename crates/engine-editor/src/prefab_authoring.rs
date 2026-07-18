//! Transactional prefab asset authoring and undoable scene integration.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Component as PathComponent, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use engine_asset::cook::manifest::CURRENT_MANIFEST_VERSION;
use engine_asset::cook::{AssetType, CookRules, SourceAssetEntry, SourceManifest};
use engine_asset::{validate_asset_id, AssetRegistry};
use engine_scene::{
    serialize_prefab_source, validate_prefab_structure, Component, ComponentRecord, EntityRecord,
    Prefab, PrefabInstanceRef, PrefabLoad, PrefabRegistry, Scene, PREFAB_SCHEMA_VERSION,
};
use engine_serialize::{AssetId, PersistentId, Value};
use thiserror::Error;

use crate::commands::{
    Command, CommandBatch, EntityClipboard, EntityPasteParent, PasteEntityRecords, RemoveComponent,
};
use crate::EditorError;

static PREFAB_ASSET_WRITE_LOCK: Mutex<()> = Mutex::new(());
static PREFAB_TRANSACTION_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Canonical source suffix for prefab assets.
pub const PREFAB_SOURCE_SUFFIX: &str = ".prefab.ron";

#[derive(Debug, Error)]
pub enum PrefabAuthoringError {
    #[error("invalid prefab request: {0}")]
    InvalidRequest(String),
    #[error("invalid prefab data: {0}")]
    InvalidPrefab(String),
    #[error("prefab asset is not loaded: {0}")]
    AssetNotLoaded(String),
    #[error("prefab source I/O failed: {0}")]
    Io(String),
    #[error("prefab manifest failed: {0}")]
    Manifest(String),
    #[error(transparent)]
    Editor(#[from] EditorError),
}

/// Explicit filesystem targets for creating one prefab source asset.
///
/// `manifest_path` must be a top-level manifest inside `source_root`, matching
/// the canonical cook scanner. `relative_source_path` must end in
/// `.prefab.ron` and cannot escape `source_root`.
pub struct PrefabAssetCreateRequest<'a> {
    pub source_root: &'a Path,
    pub manifest_path: &'a Path,
    pub relative_source_path: &'a Path,
    pub asset_id: AssetId,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CreatedPrefabAsset {
    pub asset_id: AssetId,
    pub source_path: PathBuf,
    pub manifest_path: PathBuf,
    pub prefab: Prefab,
}

/// Build a prefab document from one scene entity and its complete subtree.
///
/// Existing prefab-instance linkage is stripped so a newly-authored asset does
/// not accidentally point back to another prefab. References to entities
/// outside the captured subtree are rejected instead of silently becoming
/// dangling references.
pub fn prefab_from_scene_subtree(
    scene: &Scene,
    root_entity_id: &PersistentId,
    asset_id: AssetId,
) -> Result<Prefab, PrefabAuthoringError> {
    validate_asset_id(&asset_id)
        .map_err(|error| PrefabAuthoringError::InvalidRequest(error.to_string()))?;
    let clipboard = EntityClipboard::capture(scene, std::slice::from_ref(root_entity_id))?;
    let captured_ids = clipboard
        .entities()
        .iter()
        .map(|record| record.persistent_id.clone())
        .collect::<BTreeSet<_>>();
    let mut hierarchy = clipboard.entities().to_vec();
    for record in &mut hierarchy {
        record.components.remove(PrefabInstanceRef::TYPE_ID);
        if &record.persistent_id == root_entity_id {
            record.parent = None;
            if let Some(transform) = record.components.get_mut("engine.transform") {
                transform.fields.remove("parent");
            }
        }
        for component in record.components.values() {
            for value in component.fields.values() {
                reject_external_entity_reference(value, &captured_ids, &record.persistent_id)?;
            }
        }
    }

    let mut prefab = Prefab::new(asset_id);
    prefab.hierarchy = hierarchy;
    validate_prefab_structure(&prefab)
        .map_err(|errors| PrefabAuthoringError::InvalidPrefab(join_validation_errors(errors)))?;
    Ok(prefab)
}

/// Create a prefab source and its manifest declaration as one recoverable
/// filesystem transaction.
pub fn create_prefab_asset_from_scene(
    scene: &Scene,
    root_entity_id: &PersistentId,
    request: PrefabAssetCreateRequest<'_>,
) -> Result<CreatedPrefabAsset, PrefabAuthoringError> {
    let _guard = PREFAB_ASSET_WRITE_LOCK
        .lock()
        .map_err(|_| PrefabAuthoringError::Io("prefab asset write lock was poisoned".into()))?;
    if !request.source_root.is_dir() {
        return Err(PrefabAuthoringError::InvalidRequest(format!(
            "source root is not a directory: {}",
            request.source_root.display()
        )));
    }
    let relative_source = validate_relative_prefab_path(request.relative_source_path)?;
    reject_symlink_ancestors(request.source_root, &relative_source)?;
    let source_path = request.source_root.join(&relative_source);
    if source_path.exists() {
        return Err(PrefabAuthoringError::InvalidRequest(format!(
            "prefab source already exists: {}",
            source_path.display()
        )));
    }
    let manifest_path = resolve_manifest_path(request.source_root, request.manifest_path)?;
    if manifest_path.exists() {
        let metadata = std::fs::symlink_metadata(&manifest_path).map_err(|error| {
            PrefabAuthoringError::Manifest(format!(
                "could not inspect {}: {error}",
                manifest_path.display()
            ))
        })?;
        if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            return Err(PrefabAuthoringError::Manifest(format!(
                "manifest is not a regular project file: {}",
                manifest_path.display()
            )));
        }
    }
    let prefab = prefab_from_scene_subtree(scene, root_entity_id, request.asset_id.clone())?;
    let mut source_text =
        serialize_prefab_source(&prefab).map_err(PrefabAuthoringError::InvalidPrefab)?;
    source_text.push('\n');

    let mut manifest = if manifest_path.exists() {
        let bytes = std::fs::read(&manifest_path).map_err(|error| {
            PrefabAuthoringError::Manifest(format!(
                "could not read {}: {error}",
                manifest_path.display()
            ))
        })?;
        serde_json::from_slice::<SourceManifest>(&bytes).map_err(|error| {
            PrefabAuthoringError::Manifest(format!(
                "could not parse {}: {error}",
                manifest_path.display()
            ))
        })?
    } else {
        SourceManifest {
            schema_version: CURRENT_MANIFEST_VERSION,
            assets: Vec::new(),
        }
    };
    if manifest.schema_version != CURRENT_MANIFEST_VERSION {
        return Err(PrefabAuthoringError::Manifest(format!(
            "{} uses unsupported manifest schema",
            manifest_path.display()
        )));
    }
    let relative_source_string = portable_path(&relative_source)?;
    let requested_id_key = request.asset_id.id.to_ascii_lowercase();
    let requested_path_key = relative_source_string.to_ascii_lowercase();
    if manifest
        .assets
        .iter()
        .any(|entry| entry.id.id.to_ascii_lowercase() == requested_id_key)
    {
        return Err(PrefabAuthoringError::Manifest(format!(
            "asset ID '{}' is already declared",
            request.asset_id.id
        )));
    }
    if manifest.assets.iter().any(|entry| {
        entry.source_path.replace('\\', "/").to_ascii_lowercase() == requested_path_key
    }) {
        return Err(PrefabAuthoringError::Manifest(format!(
            "source path '{}' is already declared",
            relative_source_string
        )));
    }
    manifest.assets.push(SourceAssetEntry {
        id: request.asset_id.clone(),
        asset_type: AssetType::Prefab,
        source_path: relative_source_string,
        cook_rules: CookRules::default(),
    });
    manifest
        .assets
        .sort_by(|left, right| left.id.id.cmp(&right.id.id));
    let mut manifest_bytes = serde_json::to_vec_pretty(&manifest).map_err(|error| {
        PrefabAuthoringError::Manifest(format!("could not serialize source manifest: {error}"))
    })?;
    manifest_bytes.push(b'\n');

    if let Some(parent) = source_path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            PrefabAuthoringError::Io(format!("could not create {}: {error}", parent.display()))
        })?;
    }
    commit_new_source_and_manifest(
        &source_path,
        source_text.as_bytes(),
        &manifest_path,
        &manifest_bytes,
    )?;

    Ok(CreatedPrefabAsset {
        asset_id: request.asset_id,
        source_path,
        manifest_path,
        prefab,
    })
}

/// Read and validate one canonical prefab source document.
pub fn load_prefab_source(path: &Path) -> Result<Prefab, PrefabAuthoringError> {
    let bytes = std::fs::read(path).map_err(|error| {
        PrefabAuthoringError::Io(format!("could not read {}: {error}", path.display()))
    })?;
    engine_scene::parse_prefab_source(&bytes).map_err(PrefabAuthoringError::InvalidPrefab)
}

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrefabUnpackMode {
    /// Remove the linkage for the selected prefab node while preserving nested
    /// prefab instance links.
    Instance,
    /// Remove linkage from the selected prefab node and every nested prefab in
    /// its scene subtree.
    Completely,
}

pub struct PrefabUnpackPlan {
    entity_ids: Vec<PersistentId>,
    command: CommandBatch,
}

impl PrefabUnpackPlan {
    pub fn entity_ids(&self) -> &[PersistentId] {
        &self.entity_ids
    }

    pub fn into_command(self) -> Box<dyn Command> {
        Box::new(self.command)
    }
}

/// Prepare an atomic, undoable unpack operation. This only removes explicit
/// prefab linkage; entity/component data remains unchanged.
pub fn prepare_unpack_prefab(
    scene: &Scene,
    selected_entity_id: &PersistentId,
    mode: PrefabUnpackMode,
) -> Result<PrefabUnpackPlan, PrefabAuthoringError> {
    let selected = scene
        .entities
        .iter()
        .find(|entity| &entity.persistent_id == selected_entity_id)
        .ok_or_else(|| EditorError::EntityNotFound(selected_entity_id.clone()))?;
    let selected_instance = instance_id(selected).ok_or_else(|| {
        PrefabAuthoringError::InvalidRequest(format!(
            "entity '{}' is not a prefab instance",
            selected_entity_id
        ))
    })?;
    let direct_ids = scene
        .entities
        .iter()
        .filter(|entity| instance_id(entity).as_deref() == Some(selected_instance.as_str()))
        .map(|entity| entity.persistent_id.clone())
        .collect::<BTreeSet<_>>();
    let root_candidates = scene
        .entities
        .iter()
        .filter(|entity| direct_ids.contains(&entity.persistent_id))
        .filter(|entity| {
            entity
                .parent
                .as_ref()
                .is_none_or(|parent| !direct_ids.contains(parent))
        })
        .map(|entity| entity.persistent_id.clone())
        .collect::<Vec<_>>();
    if root_candidates.len() != 1 {
        return Err(PrefabAuthoringError::InvalidPrefab(format!(
            "instance '{}' has {} scene roots",
            selected_instance,
            root_candidates.len()
        )));
    }

    let ids = match mode {
        PrefabUnpackMode::Instance => direct_ids,
        PrefabUnpackMode::Completely => {
            let subtree = collect_scene_subtree_ids(scene, &root_candidates[0]);
            scene
                .entities
                .iter()
                .filter(|entity| {
                    subtree.contains(&entity.persistent_id)
                        && entity.components.contains_key(PrefabInstanceRef::TYPE_ID)
                })
                .map(|entity| entity.persistent_id.clone())
                .collect()
        }
    };
    let entity_ids = scene
        .entities
        .iter()
        .filter(|entity| ids.contains(&entity.persistent_id))
        .map(|entity| entity.persistent_id.clone())
        .collect::<Vec<_>>();
    let commands = entity_ids
        .iter()
        .map(|entity_id| {
            Box::new(RemoveComponent::new(
                entity_id.clone(),
                PrefabInstanceRef::TYPE_ID.to_string(),
            )) as Box<dyn Command>
        })
        .collect();
    Ok(PrefabUnpackPlan {
        entity_ids,
        command: CommandBatch::new("Unpack Prefab", commands),
    })
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

fn prefab_instance_component(
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

fn instance_id(entity: &EntityRecord) -> Option<String> {
    let component = entity.components.get(PrefabInstanceRef::TYPE_ID)?;
    match component.fields.get("instance_id") {
        Some(Value::Str(instance_id)) if !instance_id.is_empty() => Some(instance_id.clone()),
        _ => None,
    }
}

fn collect_scene_subtree_ids(scene: &Scene, root: &PersistentId) -> BTreeSet<PersistentId> {
    let mut result = BTreeSet::new();
    let mut pending = vec![root.clone()];
    while let Some(parent) = pending.pop() {
        if !result.insert(parent.clone()) {
            continue;
        }
        pending.extend(
            scene
                .entities
                .iter()
                .filter(|entity| entity.parent.as_ref() == Some(&parent))
                .map(|entity| entity.persistent_id.clone()),
        );
    }
    result
}

fn reject_external_entity_reference(
    value: &Value,
    captured_ids: &BTreeSet<PersistentId>,
    owner: &str,
) -> Result<(), PrefabAuthoringError> {
    match value {
        Value::Entity(entity_id) if !captured_ids.contains(entity_id) => {
            return Err(PrefabAuthoringError::InvalidPrefab(format!(
                "entity '{owner}' references external entity '{entity_id}'"
            )));
        }
        Value::List(values) => {
            for value in values {
                reject_external_entity_reference(value, captured_ids, owner)?;
            }
        }
        Value::Map(values) => {
            for value in values.values() {
                reject_external_entity_reference(value, captured_ids, owner)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn remap_entity_values(value: &mut Value, id_map: &BTreeMap<PersistentId, PersistentId>) {
    match value {
        Value::Entity(entity_id) => {
            if let Some(remapped) = id_map.get(entity_id) {
                *entity_id = remapped.clone();
            }
        }
        Value::List(values) => {
            for value in values {
                remap_entity_values(value, id_map);
            }
        }
        Value::Map(values) => {
            for value in values.values_mut() {
                remap_entity_values(value, id_map);
            }
        }
        _ => {}
    }
}

fn allocate_unique_string(base: &str, used: &mut BTreeSet<String>) -> String {
    if used.insert(base.to_string()) {
        return base.to_string();
    }
    for suffix in 2_u64.. {
        let candidate = format!("{base}-{suffix}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
    }
    unreachable!("the u64 identifier suffix space cannot be exhausted in memory")
}

fn portable_token(value: &str) -> String {
    let token = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    token.trim_matches('-').to_string()
}

fn validate_relative_prefab_path(path: &Path) -> Result<PathBuf, PrefabAuthoringError> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|component| !matches!(component, PathComponent::Normal(_)))
    {
        return Err(PrefabAuthoringError::InvalidRequest(format!(
            "prefab source path must be portable and relative: {}",
            path.display()
        )));
    }
    let portable = portable_path(path)?;
    if !portable
        .to_ascii_lowercase()
        .ends_with(PREFAB_SOURCE_SUFFIX)
    {
        return Err(PrefabAuthoringError::InvalidRequest(format!(
            "prefab source path must end with '{PREFAB_SOURCE_SUFFIX}'"
        )));
    }
    Ok(path.to_path_buf())
}

fn portable_path(path: &Path) -> Result<String, PrefabAuthoringError> {
    let parts = path
        .components()
        .map(|component| match component {
            PathComponent::Normal(value) => value.to_str().map(str::to_owned).ok_or_else(|| {
                PrefabAuthoringError::InvalidRequest("path is not valid UTF-8".into())
            }),
            _ => Err(PrefabAuthoringError::InvalidRequest(
                "path is not portable and relative".into(),
            )),
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(parts.join("/"))
}

fn resolve_manifest_path(
    source_root: &Path,
    requested: &Path,
) -> Result<PathBuf, PrefabAuthoringError> {
    let path = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        source_root.join(requested)
    };
    let file_name = path.file_name().ok_or_else(|| {
        PrefabAuthoringError::InvalidRequest("manifest path has no file name".into())
    })?;
    if path.parent() != Some(source_root)
        || !file_name
            .to_string_lossy()
            .to_ascii_lowercase()
            .ends_with(".manifest")
    {
        return Err(PrefabAuthoringError::InvalidRequest(format!(
            "manifest must be a top-level .manifest file inside {}",
            source_root.display()
        )));
    }
    Ok(path)
}

fn commit_new_source_and_manifest(
    source_path: &Path,
    source_bytes: &[u8],
    manifest_path: &Path,
    manifest_bytes: &[u8],
) -> Result<(), PrefabAuthoringError> {
    let source_temp = write_transaction_temp(source_path, "source", source_bytes)?;
    let manifest_temp = match write_transaction_temp(manifest_path, "manifest", manifest_bytes) {
        Ok(path) => path,
        Err(error) => {
            let _ = std::fs::remove_file(&source_temp);
            return Err(error);
        }
    };
    let manifest_existed = manifest_path.exists();
    let manifest_backup = transaction_sibling(manifest_path, "backup");
    if manifest_existed {
        if let Err(error) = std::fs::rename(manifest_path, &manifest_backup) {
            let _ = std::fs::remove_file(&source_temp);
            let _ = std::fs::remove_file(&manifest_temp);
            return Err(PrefabAuthoringError::Io(format!(
                "could not stage manifest replacement {}: {error}",
                manifest_path.display()
            )));
        }
    }
    if let Err(error) = std::fs::rename(&source_temp, source_path) {
        if manifest_existed {
            let _ = std::fs::rename(&manifest_backup, manifest_path);
        }
        let _ = std::fs::remove_file(&manifest_temp);
        return Err(PrefabAuthoringError::Io(format!(
            "could not install prefab source {}: {error}",
            source_path.display()
        )));
    }
    if let Err(error) = std::fs::rename(&manifest_temp, manifest_path) {
        let _ = std::fs::remove_file(source_path);
        if manifest_existed {
            let _ = std::fs::rename(&manifest_backup, manifest_path);
        }
        return Err(PrefabAuthoringError::Io(format!(
            "could not install source manifest {}: {error}",
            manifest_path.display()
        )));
    }
    if manifest_existed {
        if let Err(error) = std::fs::remove_file(&manifest_backup) {
            let _ = std::fs::remove_file(source_path);
            let _ = std::fs::remove_file(manifest_path);
            let restored = std::fs::rename(&manifest_backup, manifest_path);
            return Err(PrefabAuthoringError::Io(match restored {
                Ok(()) => format!(
                    "could not remove transaction backup {}; changes were rolled back: {error}",
                    manifest_backup.display()
                ),
                Err(restore_error) => format!(
                    "could not remove transaction backup {} ({error}) and rollback failed: {restore_error}",
                    manifest_backup.display()
                ),
            }));
        }
    }
    Ok(())
}

fn write_transaction_temp(
    target: &Path,
    role: &str,
    bytes: &[u8],
) -> Result<PathBuf, PrefabAuthoringError> {
    for _ in 0..32 {
        let path = transaction_sibling(target, role);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                if let Err(error) = file.write_all(bytes).and_then(|()| file.sync_all()) {
                    let _ = std::fs::remove_file(&path);
                    return Err(PrefabAuthoringError::Io(format!(
                        "could not write transaction file {}: {error}",
                        path.display()
                    )));
                }
                return Ok(path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(PrefabAuthoringError::Io(format!(
                    "could not create transaction file {}: {error}",
                    path.display()
                )));
            }
        }
    }
    Err(PrefabAuthoringError::Io(
        "could not allocate a unique prefab transaction file".into(),
    ))
}

fn transaction_sibling(target: &Path, role: &str) -> PathBuf {
    let counter = PREFAB_TRANSACTION_COUNTER.fetch_add(1, Ordering::Relaxed);
    let name = target.file_name().unwrap_or_default().to_string_lossy();
    target.with_file_name(format!(
        ".{name}.prefab-txn-{}-{counter}.{role}",
        std::process::id()
    ))
}

fn join_validation_errors(errors: Vec<engine_scene::PrefabValidationError>) -> String {
    errors
        .into_iter()
        .map(|error| error.to_string())
        .collect::<Vec<_>>()
        .join("; ")
}

fn reject_symlink_ancestors(
    source_root: &Path,
    relative_source: &Path,
) -> Result<(), PrefabAuthoringError> {
    let root_metadata = std::fs::symlink_metadata(source_root).map_err(|error| {
        PrefabAuthoringError::Io(format!(
            "could not inspect source root {}: {error}",
            source_root.display()
        ))
    })?;
    if root_metadata.file_type().is_symlink() {
        return Err(PrefabAuthoringError::InvalidRequest(format!(
            "source root cannot be a symbolic link: {}",
            source_root.display()
        )));
    }
    let mut cursor = source_root.to_path_buf();
    if let Some(parent) = relative_source.parent() {
        for component in parent.components() {
            let PathComponent::Normal(component) = component else {
                return Err(PrefabAuthoringError::InvalidRequest(
                    "prefab source parent is not portable".into(),
                ));
            };
            cursor.push(component);
            match std::fs::symlink_metadata(&cursor) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(PrefabAuthoringError::InvalidRequest(format!(
                        "prefab source path crosses symbolic link {}",
                        cursor.display()
                    )));
                }
                Ok(metadata) if !metadata.is_dir() => {
                    return Err(PrefabAuthoringError::InvalidRequest(format!(
                        "prefab source parent is not a directory: {}",
                        cursor.display()
                    )));
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(PrefabAuthoringError::Io(format!(
                        "could not inspect {}: {error}",
                        cursor.display()
                    )));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use engine_scene::{sample_scene, ComponentRecord};
    use engine_serialize::SchemaVersion;

    use super::*;
    use crate::EditorScene;

    fn transform() -> ComponentRecord {
        ComponentRecord {
            schema_version: SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: BTreeMap::from([
                ("translation".into(), Value::Vec3([0.0, 0.0, 0.0])),
                ("rotation".into(), Value::Quat([0.0, 0.0, 0.0, 1.0])),
                ("scale".into(), Value::Vec3([1.0, 1.0, 1.0])),
            ]),
        }
    }

    fn authoring_scene() -> Scene {
        let mut scene = sample_scene();
        scene.scene_settings.active_camera = None;
        scene.entities = vec![
            EntityRecord {
                persistent_id: "vehicle".into(),
                parent: None,
                name: Some("Vehicle".into()),
                enabled: true,
                components: BTreeMap::from([("engine.transform".into(), transform())]),
            },
            EntityRecord {
                persistent_id: "wheel".into(),
                parent: Some("vehicle".into()),
                name: Some("Wheel".into()),
                enabled: true,
                components: BTreeMap::from([("engine.transform".into(), transform())]),
            },
        ];
        scene
    }

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "engine_editor_prefab_{name}_{}_{}",
            std::process::id(),
            PREFAB_TRANSACTION_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn scene_subtree_becomes_self_contained_prefab() {
        let mut scene = authoring_scene();
        scene.entities[0].components.insert(
            PrefabInstanceRef::TYPE_ID.into(),
            prefab_instance_component(&AssetId::new("prefab-old"), "old", "vehicle"),
        );
        let prefab =
            prefab_from_scene_subtree(&scene, &"vehicle".into(), AssetId::new("prefab-vehicle"))
                .unwrap();
        assert_eq!(prefab.hierarchy.len(), 2);
        assert!(prefab.hierarchy[0].parent.is_none());
        assert!(prefab
            .hierarchy
            .iter()
            .all(|record| !record.components.contains_key(PrefabInstanceRef::TYPE_ID)));
    }

    #[test]
    fn create_is_manifest_and_source_transaction() {
        let root = temp_root("create");
        let manifest_path = root.join("game.manifest");
        let request = PrefabAssetCreateRequest {
            source_root: &root,
            manifest_path: &manifest_path,
            relative_source_path: Path::new("Prefabs/vehicle.prefab.ron"),
            asset_id: AssetId::new("prefab-vehicle"),
        };
        let created =
            create_prefab_asset_from_scene(&authoring_scene(), &"vehicle".into(), request).unwrap();
        assert_eq!(
            load_prefab_source(&created.source_path).unwrap(),
            created.prefab
        );
        let manifest: SourceManifest =
            serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
        assert_eq!(manifest.assets.len(), 1);
        assert_eq!(manifest.assets[0].asset_type, AssetType::Prefab);

        let manifest_before = std::fs::read(&manifest_path).unwrap();
        let duplicate = PrefabAssetCreateRequest {
            source_root: &root,
            manifest_path: &manifest_path,
            relative_source_path: Path::new("Prefabs/other.prefab.ron"),
            asset_id: AssetId::new("prefab-vehicle"),
        };
        assert!(
            create_prefab_asset_from_scene(&authoring_scene(), &"vehicle".into(), duplicate)
                .is_err()
        );
        assert_eq!(std::fs::read(&manifest_path).unwrap(), manifest_before);
        assert!(!root.join("Prefabs/other.prefab.ron").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn instantiate_and_unpack_are_atomic_undo_commands() {
        let prefab = prefab_from_scene_subtree(
            &authoring_scene(),
            &"vehicle".into(),
            AssetId::new("prefab-vehicle"),
        )
        .unwrap();
        let mut destination = sample_scene();
        destination.scene_settings.active_camera = None;
        let plan =
            prepare_prefab_instantiation(&destination, &prefab, None, EntityPasteParent::SceneRoot)
                .unwrap();
        let root_id = plan.root_entity_id().clone();
        let inserted = plan.entity_ids().to_vec();
        let original_count = destination.entities.len();
        let mut editor = EditorScene::new(destination);
        editor.execute(plan.into_command()).unwrap();
        assert_eq!(editor.scene.entities.len(), original_count + 2);
        assert!(inserted.iter().all(|id| editor
            .scene
            .entities
            .iter()
            .find(|entity| &entity.persistent_id == id)
            .unwrap()
            .components
            .contains_key(PrefabInstanceRef::TYPE_ID)));

        let unpack =
            prepare_unpack_prefab(&editor.scene, &root_id, PrefabUnpackMode::Completely).unwrap();
        assert_eq!(unpack.entity_ids().len(), 2);
        editor.execute(unpack.into_command()).unwrap();
        assert!(inserted.iter().all(|id| !editor
            .scene
            .entities
            .iter()
            .find(|entity| &entity.persistent_id == id)
            .unwrap()
            .components
            .contains_key(PrefabInstanceRef::TYPE_ID)));
        editor.undo().unwrap();
        assert!(inserted.iter().all(|id| editor
            .scene
            .entities
            .iter()
            .find(|entity| &entity.persistent_id == id)
            .unwrap()
            .components
            .contains_key(PrefabInstanceRef::TYPE_ID)));
    }

    #[test]
    fn loaded_registry_is_a_real_instantiation_source() {
        let prefab = prefab_from_scene_subtree(
            &authoring_scene(),
            &"vehicle".into(),
            AssetId::new("prefab-vehicle"),
        )
        .unwrap();
        let mut assets = AssetRegistry::new();
        assets.insert_typed(AssetId::new("prefab-vehicle"), prefab);
        let scene = sample_scene();
        let plan = prepare_prefab_instantiation_from_registry(
            &scene,
            &assets,
            &AssetId::new("prefab-vehicle"),
            EntityPasteParent::SceneRoot,
        )
        .unwrap();
        assert_eq!(plan.entity_ids().len(), 2);
    }
}
