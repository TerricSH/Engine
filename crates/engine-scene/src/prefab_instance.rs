//! Transactional prefab instantiation and prefab-instance tracking.

use std::any::Any;
use std::collections::{BTreeMap, HashSet};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicU64, Ordering};

use engine_serialize::{SchemaVersion, Value};
use serde::{Deserialize, Serialize};

use crate::component::Component;
use crate::components::{Bounds, Camera, Light, Name, Renderable, Transform};
use crate::prefab::{Prefab, MAX_PREFAB_NESTING_DEPTH};
use crate::validation::SCENE_ONLY_COMPONENT_TYPES;
use crate::{Entity, World};

/// ECS component attached to every entity created from a prefab.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PrefabInstanceRef {
    /// Asset path of the source prefab.
    pub source_asset: String,
    /// Unique identifier shared by the entities from one prefab node.
    pub instance_id: String,
    /// Persistent ID of this entity inside its source prefab.
    pub entity_persistent_id: String,
    /// Schema version of the source prefab.
    pub schema_version: SchemaVersion,
}

impl Component for PrefabInstanceRef {
    const TYPE_ID: &'static str = "engine.prefab_instance_ref";
}

/// The result of one prefab instantiation transaction.
#[derive(Clone, Debug)]
pub struct PrefabInstantiateResult {
    /// Root entity of the instantiated hierarchy.
    pub root_entity: Entity,
    /// Every entity created by this transaction, including nested prefabs.
    pub all_entities: Vec<Entity>,
    /// Scene-only component records (for example `engine.script`) stripped
    /// from ECS materialisation and returned for their dedicated subsystem.
    ///
    /// Each entry carries the created entity, the component type id, and the
    /// full record including its `enabled` flag, mirroring how strict scene
    /// loading keeps these records out of the ECS World.
    pub scene_only_components: Vec<(Entity, String, crate::ComponentRecord)>,
}

/// A structured prefab validation or instantiation failure.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PrefabInstantiateError {
    #[error("prefab asset '{asset_id}' was not found")]
    RootAssetNotFound { asset_id: String },
    #[error("prefab '{asset_id}' has an empty hierarchy")]
    EmptyHierarchy { asset_id: String },
    #[error("prefab '{asset_id}' contains an empty persistent_id")]
    EmptyPersistentId { asset_id: String },
    #[error("prefab '{asset_id}' contains duplicate persistent_id '{persistent_id}'")]
    DuplicatePersistentId {
        asset_id: String,
        persistent_id: String,
    },
    #[error("prefab '{asset_id}' must have exactly one root entity, found {root_count}")]
    InvalidRootCount { asset_id: String, root_count: usize },
    #[error(
        "prefab '{asset_id}' entity '{entity_persistent_id}' references missing parent '{parent_persistent_id}'"
    )]
    MissingParentEntity {
        asset_id: String,
        entity_persistent_id: String,
        parent_persistent_id: String,
    },
    #[error(
        "prefab '{asset_id}' child '{child_asset_id}' references missing attachment entity '{entity_persistent_id}'"
    )]
    MissingAttachmentEntity {
        asset_id: String,
        child_asset_id: String,
        entity_persistent_id: String,
    },
    #[error(
        "prefab '{asset_id}' needs a resolver to load child '{child_asset_id}' at entity '{entity_persistent_id}'"
    )]
    MissingChildResolver {
        asset_id: String,
        child_asset_id: String,
        entity_persistent_id: String,
    },
    #[error(
        "prefab '{asset_id}' references missing child prefab asset '{child_asset_id}' at entity '{entity_persistent_id}'"
    )]
    MissingChildPrefab {
        asset_id: String,
        child_asset_id: String,
        entity_persistent_id: String,
    },
    #[error("prefab dependency cycle detected: {cycle:?}")]
    DependencyCycle { cycle: Vec<String> },
    #[error("prefab nesting exceeds maximum depth {max_depth} while processing '{asset_id}'")]
    MaximumDepthExceeded { asset_id: String, max_depth: usize },
    #[error(
        "component '{component_type_id}' on prefab '{asset_id}' entity '{entity_persistent_id}' is not constructible: {reason}"
    )]
    ComponentNotConstructible {
        asset_id: String,
        entity_persistent_id: String,
        component_type_id: String,
        reason: String,
    },
    #[error(
        "prefab '{asset_id}' entity '{entity_persistent_id}' needs an enabled Transform to participate in hierarchy parenting"
    )]
    MissingParentingTransform {
        asset_id: String,
        entity_persistent_id: String,
    },
    #[error("prefab '{asset_id}' failed an internal invariant: {reason}")]
    InternalInvariant { asset_id: String, reason: String },
}

static INSTANCE_COUNTER: AtomicU64 = AtomicU64::new(0);

fn generate_instance_id() -> String {
    let count = INSTANCE_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("inst_{count}")
}

/// Trait for types that can resolve a concrete prefab asset by identifier.
pub trait PrefabLoad {
    /// Look up a prefab by its asset identifier.
    fn load_prefab(&self, asset_id: &str) -> Option<&Prefab>;
}

/// A simple in-memory prefab registry for tooling, tests, and embedded use.
#[derive(Clone, Debug, Default)]
pub struct PrefabRegistry {
    prefabs: std::collections::HashMap<String, Prefab>,
}

impl PrefabRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register or replace a concrete prefab asset.
    pub fn register(&mut self, asset_id: impl Into<String>, prefab: Prefab) {
        self.prefabs.insert(asset_id.into(), prefab);
    }
}

impl PrefabLoad for PrefabRegistry {
    fn load_prefab(&self, asset_id: &str) -> Option<&Prefab> {
        self.prefabs.get(asset_id)
    }
}

/// Validate and instantiate a prefab as one transaction.
///
/// The entire reachable prefab graph is checked before the first entity is
/// allocated. If a resolver changes or a component fails during construction,
/// every entity allocated by this call is destroyed before the error returns.
pub fn instantiate_prefab(
    world: &mut World,
    prefab: &Prefab,
    child_resolver: Option<&dyn PrefabLoad>,
) -> Result<PrefabInstantiateResult, PrefabInstantiateError> {
    validate_prefab_for_instantiation(world, prefab, child_resolver)?;

    let mut instantiator = Instantiator {
        resolver: child_resolver,
        visiting: HashSet::new(),
        path: Vec::new(),
        created: Vec::new(),
    };
    match instantiator.instantiate_node(world, &prefab.source_asset.id, prefab, 0) {
        Ok(result) => Ok(result),
        Err(error) => {
            for entity in instantiator.created.into_iter().rev() {
                let _ = world.destroy_entity(entity);
            }
            Err(error)
        }
    }
}

/// Load a root prefab from a concrete asset resolver and instantiate it.
pub fn instantiate_prefab_from_asset(
    world: &mut World,
    registry: &dyn PrefabLoad,
    asset_id: &str,
) -> Result<PrefabInstantiateResult, PrefabInstantiateError> {
    let prefab = registry.load_prefab(asset_id).ok_or_else(|| {
        PrefabInstantiateError::RootAssetNotFound {
            asset_id: asset_id.to_string(),
        }
    })?;
    validate_prefab_for_instantiation_with_root_id(world, asset_id, prefab, Some(registry))?;

    let mut instantiator = Instantiator {
        resolver: Some(registry),
        visiting: HashSet::new(),
        path: Vec::new(),
        created: Vec::new(),
    };
    match instantiator.instantiate_node(world, asset_id, prefab, 0) {
        Ok(result) => Ok(result),
        Err(error) => {
            for entity in instantiator.created.into_iter().rev() {
                let _ = world.destroy_entity(entity);
            }
            Err(error)
        }
    }
}

pub(crate) fn validate_prefab_for_instantiation(
    world: &World,
    prefab: &Prefab,
    resolver: Option<&dyn PrefabLoad>,
) -> Result<(), PrefabInstantiateError> {
    validate_prefab_for_instantiation_with_root_id(world, &prefab.source_asset.id, prefab, resolver)
}

fn validate_prefab_for_instantiation_with_root_id(
    world: &World,
    root_asset_id: &str,
    prefab: &Prefab,
    resolver: Option<&dyn PrefabLoad>,
) -> Result<(), PrefabInstantiateError> {
    let mut validator = GraphValidator {
        world,
        resolver,
        visiting: HashSet::new(),
        visited: HashSet::new(),
        path: Vec::new(),
    };
    validator.visit(root_asset_id, prefab, 0, false)
}

struct GraphValidator<'a> {
    world: &'a World,
    resolver: Option<&'a dyn PrefabLoad>,
    visiting: HashSet<String>,
    visited: HashSet<String>,
    path: Vec<String>,
}

impl GraphValidator<'_> {
    fn visit(
        &mut self,
        asset_id: &str,
        prefab: &Prefab,
        depth: usize,
        requires_attachment_transform: bool,
    ) -> Result<(), PrefabInstantiateError> {
        if depth > MAX_PREFAB_NESTING_DEPTH {
            return Err(PrefabInstantiateError::MaximumDepthExceeded {
                asset_id: asset_id.to_string(),
                max_depth: MAX_PREFAB_NESTING_DEPTH,
            });
        }
        if self.visiting.contains(asset_id) {
            let start = self
                .path
                .iter()
                .position(|candidate| candidate == asset_id)
                .unwrap_or(0);
            let mut cycle = self.path[start..].to_vec();
            cycle.push(asset_id.to_string());
            return Err(PrefabInstantiateError::DependencyCycle { cycle });
        }
        if self.visited.contains(asset_id) {
            return Ok(());
        }

        self.visiting.insert(asset_id.to_string());
        self.path.push(asset_id.to_string());
        let result = self.validate_node(asset_id, prefab, depth, requires_attachment_transform);
        self.path.pop();
        self.visiting.remove(asset_id);
        if result.is_ok() {
            self.visited.insert(asset_id.to_string());
        }
        result
    }

    fn validate_node(
        &mut self,
        asset_id: &str,
        prefab: &Prefab,
        depth: usize,
        requires_attachment_transform: bool,
    ) -> Result<(), PrefabInstantiateError> {
        if prefab.hierarchy.is_empty() {
            return Err(PrefabInstantiateError::EmptyHierarchy {
                asset_id: asset_id.to_string(),
            });
        }

        let mut ids = HashSet::new();
        for record in &prefab.hierarchy {
            if record.persistent_id.is_empty() {
                return Err(PrefabInstantiateError::EmptyPersistentId {
                    asset_id: asset_id.to_string(),
                });
            }
            if !ids.insert(record.persistent_id.as_str()) {
                return Err(PrefabInstantiateError::DuplicatePersistentId {
                    asset_id: asset_id.to_string(),
                    persistent_id: record.persistent_id.clone(),
                });
            }
        }

        let roots: Vec<_> = prefab
            .hierarchy
            .iter()
            .filter(|record| record.parent.is_none())
            .collect();
        if roots.len() != 1 {
            return Err(PrefabInstantiateError::InvalidRootCount {
                asset_id: asset_id.to_string(),
                root_count: roots.len(),
            });
        }

        for record in &prefab.hierarchy {
            if let Some(parent_id) = &record.parent {
                if !ids.contains(parent_id.as_str()) {
                    return Err(PrefabInstantiateError::MissingParentEntity {
                        asset_id: asset_id.to_string(),
                        entity_persistent_id: record.persistent_id.clone(),
                        parent_persistent_id: parent_id.clone(),
                    });
                }
                require_enabled_transform(asset_id, record)?;
            }

            for (component_type_id, component) in &record.components {
                // Scene-only component types (such as `engine.script`) are
                // metadata for their dedicated subsystem, not ECS content;
                // strict scene loading strips them the same way.
                if SCENE_ONLY_COMPONENT_TYPES.contains(&component_type_id.as_str()) {
                    continue;
                }
                let fields = merge_defaults(
                    &component.fields,
                    prefab.component_defaults.get(component_type_id),
                );
                validate_component_constructible(
                    self.world,
                    asset_id,
                    &record.persistent_id,
                    component_type_id,
                    &fields,
                )?;
            }
        }

        if requires_attachment_transform {
            require_enabled_transform(asset_id, roots[0])?;
        }

        for child_ref in &prefab.child_prefab_refs {
            if !ids.contains(child_ref.entity_persistent_id.as_str()) {
                return Err(PrefabInstantiateError::MissingAttachmentEntity {
                    asset_id: asset_id.to_string(),
                    child_asset_id: child_ref.prefab_asset.id.clone(),
                    entity_persistent_id: child_ref.entity_persistent_id.clone(),
                });
            }
            let resolver =
                self.resolver
                    .ok_or_else(|| PrefabInstantiateError::MissingChildResolver {
                        asset_id: asset_id.to_string(),
                        child_asset_id: child_ref.prefab_asset.id.clone(),
                        entity_persistent_id: child_ref.entity_persistent_id.clone(),
                    })?;
            let child = resolver
                .load_prefab(&child_ref.prefab_asset.id)
                .ok_or_else(|| PrefabInstantiateError::MissingChildPrefab {
                    asset_id: asset_id.to_string(),
                    child_asset_id: child_ref.prefab_asset.id.clone(),
                    entity_persistent_id: child_ref.entity_persistent_id.clone(),
                })?;
            self.visit(&child_ref.prefab_asset.id, child, depth + 1, true)?;
        }

        Ok(())
    }
}

fn require_enabled_transform(
    asset_id: &str,
    record: &crate::EntityRecord,
) -> Result<(), PrefabInstantiateError> {
    if record
        .components
        .get(Transform::TYPE_ID)
        .is_some_and(|component| component.enabled)
    {
        Ok(())
    } else {
        Err(PrefabInstantiateError::MissingParentingTransform {
            asset_id: asset_id.to_string(),
            entity_persistent_id: record.persistent_id.clone(),
        })
    }
}

fn validate_component_constructible(
    world: &World,
    asset_id: &str,
    entity_persistent_id: &str,
    component_type_id: &str,
    fields: &BTreeMap<String, Value>,
) -> Result<(), PrefabInstantiateError> {
    let fail = |reason: String| PrefabInstantiateError::ComponentNotConstructible {
        asset_id: asset_id.to_string(),
        entity_persistent_id: entity_persistent_id.to_string(),
        component_type_id: component_type_id.to_string(),
        reason,
    };

    if is_core_component(component_type_id) {
        let mut scratch = World::new();
        let entity = scratch.create_entity();
        catch_unwind(AssertUnwindSafe(|| {
            scratch.populate_component(entity, component_type_id, fields);
        }))
        .map_err(|payload| fail(format!("constructor panicked: {}", panic_message(payload))))?;
        if scratch.get_any(entity, component_type_id).is_none() {
            return Err(fail(
                "constructor did not produce the requested component".to_string(),
            ));
        }
        return Ok(());
    }

    let registry = world
        .component_registry()
        .ok_or_else(|| fail("no ComponentRegistry is installed".to_string()))?;
    let extension = registry
        .get(component_type_id)
        .ok_or_else(|| fail("component type is not registered".to_string()))?;
    let deserialize = extension
        .deserialize
        .ok_or_else(|| fail("component type has no deserialize hook".to_string()))?;

    let mut storage =
        catch_unwind(AssertUnwindSafe(|| (extension.storage_factory)())).map_err(|payload| {
            fail(format!(
                "storage factory panicked: {}",
                panic_message(payload)
            ))
        })?;
    let storage_type_id = crate::ComponentStorageDyn::type_id(storage.as_ref());
    if storage_type_id != extension.meta.type_id {
        return Err(fail(format!(
            "storage factory returned type '{}'",
            storage_type_id
        )));
    }
    let component = catch_unwind(AssertUnwindSafe(|| deserialize(fields))).map_err(|payload| {
        fail(format!(
            "deserialize hook panicked: {}",
            panic_message(payload)
        ))
    })?;
    let probe = Entity::new(0, 1);
    storage
        .insert_any(probe, component)
        .map_err(|_| fail("deserialized value is incompatible with storage".to_string()))
}

fn is_core_component(type_id: &str) -> bool {
    matches!(
        type_id,
        Name::TYPE_ID
            | Transform::TYPE_ID
            | Renderable::TYPE_ID
            | Camera::TYPE_ID
            | Light::TYPE_ID
            | Bounds::TYPE_ID
    )
}

fn panic_message(payload: Box<dyn Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_string()
    }
}

struct Instantiator<'a> {
    resolver: Option<&'a dyn PrefabLoad>,
    visiting: HashSet<String>,
    path: Vec<String>,
    created: Vec<Entity>,
}

impl Instantiator<'_> {
    fn instantiate_node(
        &mut self,
        world: &mut World,
        asset_id: &str,
        prefab: &Prefab,
        depth: usize,
    ) -> Result<PrefabInstantiateResult, PrefabInstantiateError> {
        if depth > MAX_PREFAB_NESTING_DEPTH {
            return Err(PrefabInstantiateError::MaximumDepthExceeded {
                asset_id: asset_id.to_string(),
                max_depth: MAX_PREFAB_NESTING_DEPTH,
            });
        }
        if self.visiting.contains(asset_id) {
            let start = self
                .path
                .iter()
                .position(|candidate| candidate == asset_id)
                .unwrap_or(0);
            let mut cycle = self.path[start..].to_vec();
            cycle.push(asset_id.to_string());
            return Err(PrefabInstantiateError::DependencyCycle { cycle });
        }

        self.visiting.insert(asset_id.to_string());
        self.path.push(asset_id.to_string());
        let result = self.instantiate_node_inner(world, asset_id, prefab, depth);
        self.path.pop();
        self.visiting.remove(asset_id);
        result
    }

    fn instantiate_node_inner(
        &mut self,
        world: &mut World,
        asset_id: &str,
        prefab: &Prefab,
        depth: usize,
    ) -> Result<PrefabInstantiateResult, PrefabInstantiateError> {
        let instance_id = generate_instance_id();
        let mut entity_map = BTreeMap::new();
        let mut local_entities = Vec::with_capacity(prefab.hierarchy.len());
        let mut scene_only_components = Vec::new();

        for record in &prefab.hierarchy {
            let entity = world.create_entity();
            self.created.push(entity);
            if entity_map
                .insert(record.persistent_id.clone(), entity)
                .is_some()
            {
                return Err(PrefabInstantiateError::DuplicatePersistentId {
                    asset_id: asset_id.to_string(),
                    persistent_id: record.persistent_id.clone(),
                });
            }
            local_entities.push((record.persistent_id.clone(), entity));
        }

        for record in &prefab.hierarchy {
            let entity = entity_map
                .get(&record.persistent_id)
                .copied()
                .ok_or_else(|| PrefabInstantiateError::InternalInvariant {
                    asset_id: asset_id.to_string(),
                    reason: format!(
                        "entity '{}' is absent from allocation map",
                        record.persistent_id
                    ),
                })?;
            world.set_enabled(entity, record.enabled);
            if let Some(name) = &record.name {
                world.add_component(entity, Name(name.clone()));
            }

            for (component_type_id, component) in &record.components {
                // Scene-only component types (such as `engine.script`) are
                // collected for their dedicated subsystem instead of being
                // materialised into ECS storage, matching strict scene loads.
                if SCENE_ONLY_COMPONENT_TYPES.contains(&component_type_id.as_str()) {
                    scene_only_components.push((
                        entity,
                        component_type_id.clone(),
                        component.clone(),
                    ));
                    continue;
                }
                if !component.enabled {
                    continue;
                }
                let fields = merge_defaults(
                    &component.fields,
                    prefab.component_defaults.get(component_type_id),
                );
                let populated = catch_unwind(AssertUnwindSafe(|| {
                    world.populate_component(entity, component_type_id, &fields);
                }));
                if let Err(payload) = populated {
                    return Err(PrefabInstantiateError::ComponentNotConstructible {
                        asset_id: asset_id.to_string(),
                        entity_persistent_id: record.persistent_id.clone(),
                        component_type_id: component_type_id.clone(),
                        reason: format!("constructor panicked: {}", panic_message(payload)),
                    });
                }
                if world.get_any(entity, component_type_id).is_none() {
                    return Err(PrefabInstantiateError::ComponentNotConstructible {
                        asset_id: asset_id.to_string(),
                        entity_persistent_id: record.persistent_id.clone(),
                        component_type_id: component_type_id.clone(),
                        reason: "constructor did not produce the requested component".to_string(),
                    });
                }
            }

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

        for record in &prefab.hierarchy {
            let Some(parent_id) = &record.parent else {
                continue;
            };
            let entity = entity_map[&record.persistent_id];
            let parent = entity_map.get(parent_id).copied().ok_or_else(|| {
                PrefabInstantiateError::MissingParentEntity {
                    asset_id: asset_id.to_string(),
                    entity_persistent_id: record.persistent_id.clone(),
                    parent_persistent_id: parent_id.clone(),
                }
            })?;
            let transform = world.get_mut::<Transform>(entity).ok_or_else(|| {
                PrefabInstantiateError::MissingParentingTransform {
                    asset_id: asset_id.to_string(),
                    entity_persistent_id: record.persistent_id.clone(),
                }
            })?;
            transform.parent = Some(parent);
        }

        let root_record = prefab
            .hierarchy
            .iter()
            .find(|record| record.parent.is_none())
            .ok_or_else(|| PrefabInstantiateError::InvalidRootCount {
                asset_id: asset_id.to_string(),
                root_count: 0,
            })?;
        let root_entity = entity_map[&root_record.persistent_id];
        let mut all_entities = Vec::with_capacity(local_entities.len());
        all_entities.push(root_entity);
        all_entities.extend(
            local_entities
                .iter()
                .filter_map(|(_, entity)| (*entity != root_entity).then_some(*entity)),
        );

        for child_ref in &prefab.child_prefab_refs {
            let parent = entity_map
                .get(&child_ref.entity_persistent_id)
                .copied()
                .ok_or_else(|| PrefabInstantiateError::MissingAttachmentEntity {
                    asset_id: asset_id.to_string(),
                    child_asset_id: child_ref.prefab_asset.id.clone(),
                    entity_persistent_id: child_ref.entity_persistent_id.clone(),
                })?;
            let resolver =
                self.resolver
                    .ok_or_else(|| PrefabInstantiateError::MissingChildResolver {
                        asset_id: asset_id.to_string(),
                        child_asset_id: child_ref.prefab_asset.id.clone(),
                        entity_persistent_id: child_ref.entity_persistent_id.clone(),
                    })?;
            let child = resolver
                .load_prefab(&child_ref.prefab_asset.id)
                .ok_or_else(|| PrefabInstantiateError::MissingChildPrefab {
                    asset_id: asset_id.to_string(),
                    child_asset_id: child_ref.prefab_asset.id.clone(),
                    entity_persistent_id: child_ref.entity_persistent_id.clone(),
                })?;
            let child_result =
                self.instantiate_node(world, &child_ref.prefab_asset.id, child, depth + 1)?;
            let child_transform = world
                .get_mut::<Transform>(child_result.root_entity)
                .ok_or_else(|| PrefabInstantiateError::MissingParentingTransform {
                    asset_id: child_ref.prefab_asset.id.clone(),
                    entity_persistent_id: child
                        .hierarchy
                        .iter()
                        .find(|record| record.parent.is_none())
                        .map_or_else(String::new, |record| record.persistent_id.clone()),
                })?;
            child_transform.parent = Some(parent);
            all_entities.extend(child_result.all_entities);
            scene_only_components.extend(child_result.scene_only_components);
        }

        Ok(PrefabInstantiateResult {
            root_entity,
            all_entities,
            scene_only_components,
        })
    }
}

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

#[cfg(test)]
include!("prefab_instance/tests.rs");
